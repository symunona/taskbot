//! End-to-end: a real server on an ephemeral loopback port, temp state dir,
//! temp vault, a fake spawn-session.sh and a fake channel shim.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;
use taltosbot::config::Config;
use taltosbot::server::{self, Running};
use tokio_tungstenite::tungstenite::Message;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Env {
    _tmp: tempfile::TempDir,
    running: Running,
    base: String,
    http: reqwest::Client,
    channel: std::path::PathBuf,
}

/// `script` is the body of the fake spawn-session.sh; `shim_port` becomes channel_port_base.
async fn boot(script: &str, shim_port: u16) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    for d in ["vault/sub", "state", "channel", "sessions", "outside"] {
        std::fs::create_dir_all(p.join(d)).unwrap();
    }
    std::fs::write(p.join("vault/note.md"), "hello from the vault").unwrap();
    std::fs::write(p.join("outside/secret.txt"), "TOPSECRET").unwrap();
    std::os::unix::fs::symlink(p.join("outside/secret.txt"), p.join("vault/leak.md")).unwrap();
    let script_path = p.join("channel/spawn-session.sh");
    std::fs::write(&script_path, script).unwrap();
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let cfg = Config::parse(&format!(
        r#"
listen_addr = "127.0.0.1"
listen_port = 0
vault_path = "{root}/vault"
gemini_model = "test-model"
channel_dir = "{root}/channel"
channel_port_base = {shim_port}
claude_sessions_dir = "{root}/sessions"
public_urls = ["https://public.example"]
[keys]
gemini_api_key = "test-key"
[projects.demo]
dir = "{root}/vault"
permission_mode = "acceptEdits"
"#,
        root = p.display()
    ))
    .unwrap();
    let running = server::start(cfg, &p.join("state")).await.unwrap();
    let base = format!("http://{}", running.addr);
    Env {
        channel: p.join("channel"),
        _tmp: tmp,
        running,
        base,
        http: reqwest::Client::new(),
    }
}

impl Env {
    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let r = self
            .http
            .post(format!("{}{path}", self.base))
            .json(&body)
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str, token: Option<&str>) -> (u16, Value) {
        let mut req = self.http.get(format!("{}{path}", self.base));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        let r = req.send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
    }

    /// Mint a code on the admin route and claim it. Returns `(token, device_id)`.
    async fn pair(&self) -> (String, String) {
        let (st, new) = self.post("/api/v1/admin/pair/new", json!({})).await;
        assert_eq!(st, 200);
        let (st, paired) = self
            .post(
                "/api/v1/pair",
                json!({"code": new["code"], "name": "test phone"}),
            )
            .await;
        assert_eq!(st, 200, "{paired}");
        (
            paired["device_token"].as_str().unwrap().into(),
            paired["device_id"].as_str().unwrap().into(),
        )
    }

    async fn ws(&self) -> Ws {
        let url = format!("ws://{}/api/v1/ws", self.running.addr);
        tokio_tungstenite::connect_async(url).await.unwrap().0
    }

    async fn ws_authed(&self, token: &str) -> Ws {
        let mut ws = self.ws().await;
        send(
            &mut ws,
            json!({"event": "auth", "payload": {"token": token}}),
        )
        .await;
        assert_eq!(next(&mut ws).await.unwrap()["event"], "auth.ok");
        ws
    }
}

async fn send(ws: &mut Ws, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

/// Next JSON frame; `None` once the server has closed the socket.
async fn next(ws: &mut Ws) -> Option<Value> {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("no frame within 10 s");
        match msg {
            Some(Ok(Message::Text(t))) => return Some(serde_json::from_str(&t).unwrap()),
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return None,
            Some(Ok(_)) => continue,
        }
    }
}

/// Send a request and return the payload of its reply, checking the envelope.
async fn request(ws: &mut Ws, id: &str, event: &str, payload: Value, reply_event: &str) -> Value {
    send(
        ws,
        json!({"event": event, "id": id, "ts": 0, "payload": payload}),
    )
    .await;
    let reply = next(ws).await.expect("socket closed");
    assert_eq!(reply["event"], reply_event, "{reply}");
    assert_eq!(reply["ref_id"], id);
    assert!(reply["id"].is_string() && reply["ts"].is_i64());
    reply["payload"].clone()
}

async fn call_tool(ws: &mut Ws, name: &str, args: Value) -> Value {
    request(
        ws,
        name,
        "tool.call",
        json!({"name": name, "args": args}),
        "tool.result",
    )
    .await["result"]
        .clone()
}

async fn free_listener() -> (u16, tokio::net::TcpListener) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    (listener.local_addr().unwrap().port(), listener)
}

/// A stand-in for the channel shim: `/health` is up once `ready` exists
/// (the fake spawn script creates it); `/prompt` bodies land in the channel.
fn serve_shim(
    listener: tokio::net::TcpListener,
    ready: std::path::PathBuf,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) {
    use axum::http::StatusCode;
    use axum::routing::{get, post};
    let app = axum::Router::new()
        .route(
            "/health",
            get(move || async move {
                if ready.exists() {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                }
            }),
        )
        .route(
            "/prompt",
            post(move |body: String| async move {
                tx.send(body).unwrap();
                StatusCode::OK
            }),
        );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
}

const OK_SCRIPT: &str =
    "#!/bin/sh\nd=$(dirname \"$0\")\necho \"$*\" >> \"$d/calls.log\"\ntouch \"$d/ready\"\n";

#[tokio::test]
async fn pair_then_tools_and_agent_say() {
    let env = boot(OK_SCRIPT, 55600).await;

    let (st, health) = env.get("/api/v1/health", None).await;
    assert_eq!(
        (st, &health["app"], &health["status"]),
        (200, &json!("taltosbot"), &json!("ok"))
    );
    assert!(health["version"].is_string());

    // Pairing: the code works once, hands out a 64-hex token and the URLs.
    let (st, new) = env.post("/api/v1/admin/pair/new", json!({})).await;
    assert_eq!(st, 200);
    let code = new["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 8);
    assert!(new["expires_at"].is_string());
    assert_eq!(new["server_urls"][0], "https://public.example");
    let (st, bad) = env
        .post("/api/v1/pair", json!({"code": "AAAAAAAA", "name": "x"}))
        .await;
    assert_eq!(st, 401);
    assert!(bad["error"].is_string());
    let (st, paired) = env
        .post(
            "/api/v1/pair",
            json!({"code": code.to_lowercase(), "name": "phone"}),
        )
        .await;
    assert_eq!(st, 200);
    let token = paired["device_token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 64);
    assert!(paired["server_urls"].is_array());
    assert_eq!(
        env.post("/api/v1/pair", json!({"code": code, "name": "again"}))
            .await
            .0,
        401
    );

    // Bearer routes.
    let (st, me) = env.get("/api/v1/me", Some(&token)).await;
    assert_eq!(
        (st, &me["device_id"], &me["name"]),
        (200, &paired["device_id"], &json!("phone"))
    );
    assert_eq!(env.get("/api/v1/me", None).await.0, 401);
    assert_eq!(env.get("/api/v1/me", Some("nope")).await.0, 401);
    assert_eq!(env.get("/api/v1/config", None).await.0, 401);
    let (st, cfg) = env.get("/api/v1/config", Some(&token)).await;
    assert_eq!((st, &cfg["gemini_model"]), (200, &json!("test-model")));
    assert!(cfg["gemini_api_key"].is_string());
    assert_eq!(
        cfg["gemini_text_model"], "gemini-flash-latest",
        "default when the config does not set it"
    );
    assert_eq!(
        cfg.as_object().unwrap().len(),
        3,
        "nothing but the three documented keys"
    );

    let (st, devices) = env.get("/api/v1/admin/devices", None).await;
    assert_eq!(st, 200);
    assert_eq!(devices["devices"][0]["id"], paired["device_id"]);
    assert!(!devices.to_string().contains(&token));
    assert_eq!(
        env.get("/nope", None).await,
        (404, json!({"error": "not found"}))
    );

    // WebSocket.
    let mut ws = env.ws_authed(&token).await;
    let tools = request(&mut ws, "1", "tools.list", json!({}), "tools.list.result").await;
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "read_file",
            "list_directory",
            "search_regexp",
            "projects",
            "sessions",
            "tasks",
            "rename_thread",
            "switch_thread",
            "memory_note",
            "memory_forget",
            "memory_read",
            "set_project",
            "delegate",
            "web_search",
            "end_conversation"
        ]
    );
    assert_eq!(tools["tools"][0]["parameters"]["type"], "OBJECT");

    assert_eq!(
        call_tool(&mut ws, "read_file", json!({"path": "note.md"})).await,
        json!({"content": "hello from the vault"})
    );
    for path in ["../outside/secret.txt", "leak.md", "/etc/passwd"] {
        let r = call_tool(&mut ws, "read_file", json!({ "path": path })).await;
        assert!(
            r["error"].is_string() && r.get("content").is_none(),
            "{path}: {r}"
        );
    }
    assert_eq!(
        call_tool(&mut ws, "list_directory", json!({"path": ""})).await,
        json!({"items": ["leak.md", "note.md", "sub/"]})
    );
    assert_eq!(
        call_tool(&mut ws, "search_regexp", json!({"pattern": "(?i)HELLO"})).await,
        json!({"results": [{"path": "note.md", "match": "hello from the vault"}]})
    );
    assert_eq!(
        call_tool(
            &mut ws,
            "read_file",
            json!({"path": "note.md", "start_line": 1.0, "end_line": "1"})
        )
        .await,
        json!({"content": "hello from the vault\n", "start_line": 1, "end_line": 1, "lines": 1})
    );
    assert!(
        call_tool(&mut ws, "read_notes", json!({"paths": ["note.md"]})).await["error"].is_string()
    );
    let projects = call_tool(&mut ws, "projects", json!({})).await;
    assert_eq!(projects["projects"][0]["name"], "demo");
    assert!(
        call_tool(&mut ws, "create_file", json!({"path": "x", "content": "y"})).await["error"]
            .is_string()
    );
    assert!(!env.running.app.vault.root().join("x").exists());

    let sessions = request(
        &mut ws,
        "s",
        "sessions.list",
        json!({}),
        "sessions.list.result",
    )
    .await;
    assert_eq!(
        sessions["sessions"],
        json!([{"project": "demo", "tmux": "tb-demo", "status": "dead", "claude_session_id": null, "url": null}])
    );
    assert_eq!(
        call_tool(&mut ws, "sessions", json!({})).await["sessions"],
        sessions["sessions"]
    );
    let unknown = request(&mut ws, "u", "no.such.event", json!({}), "error").await;
    assert!(unknown["error"].is_string());

    // Server push: every authenticated socket gets it, without ref_id.
    let mut ws2 = env.ws_authed(&token).await;
    let (st, ok) = env
        .post(
            "/internal/agent/say",
            json!({"project": "demo", "text": "done, boss"}),
        )
        .await;
    assert_eq!((st, ok), (200, json!({"ok": true})));
    for sock in [&mut ws, &mut ws2] {
        let push = next(sock).await.unwrap();
        assert_eq!(push["event"], "agent.say");
        assert_eq!(
            push["payload"],
            json!({"project": "demo", "text": "done, boss"})
        );
        assert!(push.get("ref_id").is_none());
    }
    let (st, _) = env
        .post(
            "/internal/agent/event",
            json!({"project": "demo", "kind": "stop", "detail": "bye"}),
        )
        .await;
    assert_eq!(st, 200);
    for sock in [&mut ws, &mut ws2] {
        let push = next(sock).await.unwrap();
        assert_eq!(
            (&push["event"], &push["payload"]["kind"]),
            (&json!("agent.event"), &json!("stop"))
        );
    }

    // A web page on this machine must not reach the loopback routes.
    let r = env
        .http
        .post(format!("{}/api/v1/admin/pair/new", env.base))
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = env
        .http
        .post(format!("{}/internal/agent/say", env.base))
        .header("x-forwarded-for", "1.2.3.4")
        .json(&json!({"project": "p", "text": "t"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    // Revoking the device ends its sockets and its token.
    let id = paired["device_id"].as_str().unwrap();
    let r = env
        .http
        .delete(format!("{}/api/v1/admin/devices/{id}", env.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert!(
        next(&mut ws2).await.is_none(),
        "socket of a revoked device is closed"
    );
    assert_eq!(env.get("/api/v1/me", Some(&token)).await.0, 401);
    let r = env
        .http
        .delete(format!("{}/api/v1/admin/devices/{id}", env.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

/// Like `request`, for requests that also trigger a `threads.changed` push:
/// the push and the reply race, so pushes seen on the way are returned too.
async fn request_with_pushes(
    ws: &mut Ws,
    id: &str,
    event: &str,
    payload: Value,
    reply_event: &str,
) -> (Value, Vec<Value>) {
    send(
        ws,
        json!({"event": event, "id": id, "ts": 0, "payload": payload}),
    )
    .await;
    let mut pushes = Vec::new();
    loop {
        let frame = next(ws).await.expect("socket closed");
        if frame.get("ref_id").is_none() {
            assert_eq!(frame["event"], "threads.changed", "{frame}");
            pushes.push(frame["payload"].clone());
            continue;
        }
        assert_eq!(frame["event"], reply_event, "{frame}");
        assert_eq!(frame["ref_id"], id);
        return (frame["payload"].clone(), pushes);
    }
}

/// The next frame must be a `threads.changed` push; returns its payload.
async fn next_changed(ws: &mut Ws) -> Value {
    let push = next(ws).await.expect("socket closed");
    assert_eq!(push["event"], "threads.changed", "{push}");
    assert!(push.get("ref_id").is_none());
    push["payload"].clone()
}

fn entry_ids(got: &Value) -> Vec<&str> {
    got["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn threads_roundtrip() {
    let env = boot(OK_SCRIPT, 55600).await;
    let (token, _) = env.pair().await;
    let mut ws = env.ws_authed(&token).await;
    // A second socket: sees every push, sends nothing.
    let mut watcher = env.ws_authed(&token).await;

    let list = request(
        &mut ws,
        "l0",
        "threads.list",
        json!({}),
        "threads.list.result",
    )
    .await;
    assert_eq!(list, json!({"threads": []}));

    // create: default name, then a given one. Each is pushed to every socket.
    let (made, mut pushes) = request_with_pushes(
        &mut ws,
        "c1",
        "threads.create",
        json!({}),
        "threads.create.result",
    )
    .await;
    let first = made["thread"].clone();
    let first_id = first["id"].as_str().unwrap().to_string();
    assert_eq!(first["name"], "New thread");
    assert_eq!(first_id.len(), 16);
    assert!(first_id.bytes().all(|b| b.is_ascii_hexdigit()));
    assert!(
        first["created_at"].as_i64().unwrap() > 1_600_000_000_000,
        "epoch ms"
    );
    assert_eq!(first["created_at"], first["last_accessed_at"]);
    assert_eq!(first.as_object().unwrap().len(), 4);
    assert_eq!(next_changed(&mut watcher).await, json!({"thread": first}));
    if pushes.is_empty() {
        pushes.push(next_changed(&mut ws).await);
    }
    assert_eq!(pushes, [json!({"thread": first})], "requester hears it too");

    let (made, pushes) = request_with_pushes(
        &mut ws,
        "c2",
        "threads.create",
        json!({"name": "  Tea\n notes  "}),
        "threads.create.result",
    )
    .await;
    let second = made["thread"].clone();
    let second_id = second["id"].as_str().unwrap().to_string();
    assert_eq!(
        second["name"], "Tea notes",
        "trimmed, control chars dropped"
    );
    assert_eq!(next_changed(&mut watcher).await, json!({"thread": second}));
    if pushes.is_empty() {
        next_changed(&mut ws).await;
    }
    for bad in [json!({"name": "   "}), json!({"name": 7})] {
        let e = request(&mut ws, "c3", "threads.create", bad, "error").await;
        assert!(e["error"].is_string());
    }

    // append: new entries take the next slot, a known id is replaced in place.
    let tool_running = json!({"id": "t1", "ts": 30, "kind": "tool", "text": "",
        "tool": {"name": "read_file", "args": {"path": "note.md"}, "called_at": 30}});
    let count = request(
        &mut ws,
        "a1",
        "threads.append",
        json!({"id": first_id, "entries": [
            {"id": "m1", "ts": 10, "kind": "marker", "text": "voice_started"},
            {"id": "u1", "ts": 20, "kind": "user", "text": "hel", "voice": true},
            tool_running,
        ]}),
        "threads.append.result",
    )
    .await;
    assert_eq!(count, json!({"count": 3}));
    let tool_done = json!({"id": "t1", "ts": 30, "kind": "tool", "text": "",
        "tool": {"name": "read_file", "args": {"path": "note.md"}, "called_at": 30,
                 "result": {"content": "hello"}, "result_at": 45}});
    let user_final = json!({"id": "u1", "ts": 20, "kind": "user", "text": "hello", "voice": true});
    let count = request(
        &mut ws,
        "a2",
        "threads.append",
        json!({"id": first_id, "entries": [
            user_final,
            tool_done,
            {"id": "g1", "ts": 50, "kind": "agent", "text": "done", "project": "demo", "error": true},
        ]}),
        "threads.append.result",
    )
    .await;
    assert_eq!(count, json!({"count": 3}));

    let got = request(
        &mut ws,
        "g1",
        "threads.get",
        json!({"id": first_id}),
        "threads.get.result",
    )
    .await;
    assert_eq!(
        entry_ids(&got),
        ["m1", "u1", "t1", "g1"],
        "seq order, slots kept"
    );
    assert_eq!(got["entries"][1], user_final, "entry comes back as sent");
    assert_eq!(got["entries"][2], tool_done);
    assert_eq!(got["entries"][3]["project"], "demo");
    assert_eq!(got["thread"]["id"], first["id"]);
    assert!(got["thread"]["last_accessed_at"].as_i64() >= first["last_accessed_at"].as_i64());
    // The other thread got none of it.
    let other = request(
        &mut ws,
        "g2",
        "threads.get",
        json!({"id": second_id}),
        "threads.get.result",
    )
    .await;
    assert_eq!(other["entries"], json!([]));

    // Refused appends write nothing.
    let too_many: Vec<Value> = (0..201)
        .map(|i| json!({"id": format!("x{i}"), "ts": i, "kind": "user", "text": ""}))
        .collect();
    for (bad, why) in [
        (json!({"id": first_id, "entries": too_many}), "201 entries"),
        (
            json!({"id": first_id, "entries": [
                {"id": "ok", "ts": 1, "kind": "user", "text": ""},
                {"id": "bad", "ts": 1, "kind": "robot", "text": ""},
            ]}),
            "unknown kind",
        ),
        (
            json!({"id": first_id, "entries": [{"ts": 1, "kind": "user"}]}),
            "no entry id",
        ),
        (json!({"id": first_id}), "no entries"),
        (
            json!({"id": "0000000000000000", "entries": []}),
            "unknown thread",
        ),
        (json!({"entries": []}), "no thread id"),
    ] {
        let e = request(&mut ws, "a3", "threads.append", bad, "error").await;
        assert!(e["error"].is_string(), "{why}: {e}");
    }
    let got = request(
        &mut ws,
        "g3",
        "threads.get",
        json!({"id": first_id}),
        "threads.get.result",
    )
    .await;
    assert_eq!(entry_ids(&got), ["m1", "u1", "t1", "g1"]);
    let e = request(&mut ws, "g4", "threads.get", json!({"id": "nope"}), "error").await;
    assert!(
        e["error"].as_str().unwrap().contains("unknown thread"),
        "{e}"
    );

    // list: most recently accessed first.
    tokio::time::sleep(Duration::from_millis(5)).await;
    let ids = |list: &Value| -> Vec<String> {
        list["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_string())
            .collect()
    };
    request(
        &mut ws,
        "g5",
        "threads.get",
        json!({"id": second_id}),
        "threads.get.result",
    )
    .await;
    let list = request(
        &mut ws,
        "l1",
        "threads.list",
        json!({}),
        "threads.list.result",
    )
    .await;
    assert_eq!(ids(&list), [second_id.clone(), first_id.clone()]);
    tokio::time::sleep(Duration::from_millis(5)).await;
    request(
        &mut ws,
        "a4",
        "threads.append",
        json!({"id": first_id, "entries": []}),
        "threads.append.result",
    )
    .await;
    let list = request(
        &mut ws,
        "l2",
        "threads.list",
        json!({}),
        "threads.list.result",
    )
    .await;
    assert_eq!(ids(&list), [first_id.clone(), second_id.clone()]);

    // rename: reply, push, and the list shows it.
    let (renamed, pushes) = request_with_pushes(
        &mut ws,
        "r1",
        "threads.rename",
        json!({"id": first_id, "name": format!(" {} ", "é".repeat(100))}),
        "threads.rename.result",
    )
    .await;
    assert_eq!(renamed["thread"]["name"], "é".repeat(80), "cut to 80 chars");
    assert_eq!(renamed["thread"]["created_at"], first["created_at"]);
    assert_eq!(next_changed(&mut watcher).await, renamed);
    if pushes.is_empty() {
        next_changed(&mut ws).await;
    }
    for bad in [
        json!({"id": first_id, "name": " \t "}),
        json!({"id": first_id}),
        json!({"id": "nope", "name": "x"}),
        json!({"name": "x"}),
    ] {
        let e = request(&mut ws, "r2", "threads.rename", bad, "error").await;
        assert!(e["error"].is_string());
    }

    // rename_thread tool: renames the thread named by tool.call.thread_id.
    let (result, pushes) = request_with_pushes(
        &mut ws,
        "t1",
        "tool.call",
        json!({"name": "rename_thread", "args": {"name": "Shopping list"}, "thread_id": second_id}),
        "tool.result",
    )
    .await;
    let renamed = result["result"].clone();
    assert_eq!(renamed["thread"]["name"], "Shopping list");
    assert_eq!(renamed["thread"]["id"], second["id"]);
    assert_eq!(next_changed(&mut watcher).await, renamed);
    if pushes.is_empty() {
        next_changed(&mut ws).await;
    }
    // Without thread_id, with an unknown one, with a bad name: {error}, no push.
    let r = call_tool(&mut ws, "rename_thread", json!({"name": "Orphan"})).await;
    assert!(r["error"].as_str().unwrap().contains("thread_id"), "{r}");
    for payload in [
        json!({"name": "rename_thread", "args": {"name": "x"}, "thread_id": "nope"}),
        json!({"name": "rename_thread", "args": {"name": "  "}, "thread_id": second_id}),
        json!({"name": "rename_thread", "args": {}, "thread_id": second_id}),
    ] {
        let r = request(&mut ws, "t2", "tool.call", payload, "tool.result").await;
        assert!(r["result"]["error"].is_string(), "{r}");
    }
    // Other tools ignore thread_id.
    let r = request(
        &mut ws,
        "t3",
        "tool.call",
        json!({"name": "read_file", "args": {"path": "note.md"}, "thread_id": second_id}),
        "tool.result",
    )
    .await;
    assert_eq!(r["result"], json!({"content": "hello from the vault"}));

    let list = request(
        &mut ws,
        "l3",
        "threads.list",
        json!({}),
        "threads.list.result",
    )
    .await;
    let names: Vec<&str> = list["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["é".repeat(80).as_str(), "Shopping list"]);

    // The watcher saw exactly the four pushes checked above: an agent.say sent
    // now is its next frame.
    env.post(
        "/internal/agent/say",
        json!({"project": "demo", "text": "end"}),
    )
    .await;
    assert_eq!(next(&mut watcher).await.unwrap()["event"], "agent.say");
}

#[tokio::test]
async fn switch_thread_finds_or_creates() {
    let env = boot(OK_SCRIPT, 55600).await;
    let (token, _) = env.pair().await;
    let mut ws = env.ws_authed(&token).await;
    let (made, pushes) = request_with_pushes(
        &mut ws,
        "c1",
        "threads.create",
        json!({"name": "Garden plans"}),
        "threads.create.result",
    )
    .await;
    if pushes.is_empty() {
        next_changed(&mut ws).await;
    }
    let garden = made["thread"]["id"].as_str().unwrap().to_string();

    // No thread: a new one, named, with its project. Pushed like threads.create.
    let (r, pushes) = request_with_pushes(
        &mut ws,
        "s1",
        "tool.call",
        json!({"name": "switch_thread", "args": {"name": "Taxes", "project": "demo"}, "thread_id": garden}),
        "tool.result",
    )
    .await;
    let made = r["result"].clone();
    assert_eq!(made["created"], true, "{made}");
    assert_eq!(made["thread"]["name"], "Taxes");
    assert_eq!(made["thread"]["project"], "demo");
    assert_ne!(made["thread"]["id"], garden);
    let pushed = match pushes.first() {
        Some(p) => p.clone(),
        None => next_changed(&mut ws).await,
    };
    assert_eq!(pushed["thread"], made["thread"]);

    // Defaults: no name, no project.
    let (r, pushes) = request_with_pushes(
        &mut ws,
        "s2",
        "tool.call",
        json!({"name": "switch_thread", "args": {}, "thread_id": garden}),
        "tool.result",
    )
    .await;
    assert_eq!(r["result"]["thread"]["name"], "New thread");
    assert!(r["result"]["thread"].get("project").is_none(), "{r}");
    if pushes.is_empty() {
        next_changed(&mut ws).await;
    }

    // Existing thread by part of its name, case ignored: found, nothing made.
    let r = call_tool(&mut ws, "switch_thread", json!({"thread": "garden"})).await;
    assert_eq!(r["created"], false, "{r}");
    assert_eq!(r["thread"]["id"], garden.as_str());

    // Errors, nothing made: the calling thread, no match, unknown project, empty name.
    for (args, needle) in [
        (json!({"thread": garden}), "already in"),
        (json!({"thread": "nothing like it"}), "no thread matches"),
        (json!({"project": "nope"}), "nope"),
        (json!({"thread": "e"}), "threads match"),
    ] {
        let r = request(
            &mut ws,
            "s3",
            "tool.call",
            json!({"name": "switch_thread", "args": args, "thread_id": garden}),
            "tool.result",
        )
        .await;
        let error = r["result"]["error"]
            .as_str()
            .unwrap_or_else(|| panic!("{r}"));
        assert!(error.contains(needle), "{error}");
    }
    let list = request(
        &mut ws,
        "l1",
        "threads.list",
        json!({}),
        "threads.list.result",
    )
    .await;
    assert_eq!(list["threads"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn ws_auth_failures_close_the_socket() {
    let env = boot(OK_SCRIPT, 55600).await;

    let mut ws = env.ws().await;
    send(
        &mut ws,
        json!({"event": "auth", "payload": {"token": "0".repeat(64)}}),
    )
    .await;
    let frame = next(&mut ws).await.unwrap();
    assert_eq!(
        frame,
        json!({"event": "auth.error", "payload": {"reason": "invalid_token"}})
    );
    assert!(
        next(&mut ws).await.is_none(),
        "no second attempt on the same socket"
    );

    // Anything but an auth frame first is refused too, and gets no tool output.
    let mut ws = env.ws().await;
    send(&mut ws, json!({"event": "tool.call", "id": "1", "payload": {"name": "read_file", "args": {"path": "note.md"}}})).await;
    assert_eq!(next(&mut ws).await.unwrap()["event"], "auth.error");
    assert!(next(&mut ws).await.is_none());
}

#[tokio::test]
async fn pair_rate_limit() {
    let env = boot(OK_SCRIPT, 55600).await;
    let (_, new) = env.post("/api/v1/admin/pair/new", json!({})).await;
    for _ in 0..5 {
        assert_eq!(
            env.post("/api/v1/pair", json!({"code": "ZZZZZZZZ", "name": "x"}))
                .await
                .0,
            401
        );
    }
    assert_eq!(
        env.post("/api/v1/pair", json!({"code": "ZZZZZZZZ", "name": "x"}))
            .await
            .0,
        429
    );
    // Locked out means locked out, even with the right code.
    let (st, body) = env
        .post("/api/v1/pair", json!({"code": new["code"], "name": "x"}))
        .await;
    assert_eq!(st, 429);
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn delegate_spawns_then_prompts() {
    let (shim_port, listener) = free_listener().await;
    let env = boot(OK_SCRIPT, shim_port).await;
    let (tx, mut prompts) = tokio::sync::mpsc::unbounded_channel();
    serve_shim(listener, env.channel.join("ready"), tx);
    let (token, _) = env.pair().await;
    let mut ws = env.ws_authed(&token).await;
    let mut pushes = env.ws_authed(&token).await;

    let r = call_tool_quiet(
        &mut ws,
        "delegate",
        json!({"project": "nope", "prompt": "hi"}),
    )
    .await;
    assert!(
        r["error"].as_str().unwrap().contains("unknown project"),
        "{r}"
    );
    assert!(call_tool_quiet(
        &mut ws,
        "delegate",
        json!({"project": "demo", "prompt": "  "})
    )
    .await["error"]
        .is_string());

    // No session yet: spawn, then prompt.
    let r = call_tool_quiet(
        &mut ws,
        "delegate",
        json!({"project": "demo", "prompt": "add a note about tea"}),
    )
    .await;
    assert_eq!(
        (&r["status"], &r["project"], &r["state"], &r["queue_pos"]),
        (&json!("sent"), &json!("demo"), &json!("queued"), &json!(0))
    );
    let ev = next_agent(&mut pushes).await;
    assert_eq!(
        (
            &ev["event"],
            &ev["payload"]["project"],
            &ev["payload"]["kind"]
        ),
        (&json!("agent.event"), &json!("demo"), &json!("spawned"))
    );
    let ev = next_agent(&mut pushes).await;
    assert_eq!(
        (&ev["payload"]["kind"], &ev["payload"]["detail"]),
        (&json!("prompt"), &json!("add a note about tea"))
    );
    assert_eq!(prompts.recv().await.unwrap(), "add a note about tea");

    let calls = std::fs::read_to_string(env.channel.join("calls.log")).unwrap();
    let vault = env.running.app.cfg.projects["demo"]
        .dir
        .display()
        .to_string();
    assert_eq!(
        calls.trim(),
        format!(
            "--name tb-demo --project demo --dir {vault} --port {shim_port} --server http://127.0.0.1:{} --permission-mode acceptEdits",
            env.running.addr.port()
        )
    );

    // Session is up now: prompt only, the script is not run again.
    let r = call_tool_quiet(
        &mut ws,
        "delegate",
        json!({"project": "demo", "prompt": "second"}),
    )
    .await;
    assert_eq!(r["status"], "sent");
    assert_eq!(next_agent(&mut pushes).await["payload"]["kind"], "prompt");
    assert_eq!(prompts.recv().await.unwrap(), "second");
    assert_eq!(
        std::fs::read_to_string(env.channel.join("calls.log"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[tokio::test]
async fn delegate_failures_become_error_events() {
    // A port with nothing behind it, and a spawn script that fails.
    let (dead_port, listener) = free_listener().await;
    drop(listener);
    let env = boot("#!/bin/sh\necho 'tmux exploded' >&2\nexit 3\n", dead_port).await;
    let (token, _) = env.pair().await;
    let mut ws = env.ws_authed(&token).await;
    let mut pushes = env.ws_authed(&token).await;

    let r = call_tool_quiet(
        &mut ws,
        "delegate",
        json!({"project": "demo", "prompt": "hi"}),
    )
    .await;
    assert_eq!(r["status"], "sent");
    let ev = next_agent(&mut pushes).await;
    assert_eq!(
        (&ev["event"], &ev["payload"]["kind"]),
        (&json!("agent.event"), &json!("error"))
    );
    assert!(
        ev["payload"]["detail"]
            .as_str()
            .unwrap()
            .contains("tmux exploded"),
        "{ev}"
    );
    // The failure is also spoken: `delegate` had already answered "sent".
    let say = next_agent(&mut pushes).await;
    assert_eq!(say["event"], "agent.say");
    assert!(
        say["payload"]["text"]
            .as_str()
            .unwrap()
            .contains("tmux exploded"),
        "{say}"
    );

    // Script claims success but no shim answers: the prompt POST fails.
    std::fs::write(env.channel.join("spawn-session.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    call_tool_quiet(
        &mut ws,
        "delegate",
        json!({"project": "demo", "prompt": "hi"}),
    )
    .await;
    assert_eq!(next_agent(&mut pushes).await["payload"]["kind"], "spawned");
    let ev = next_agent(&mut pushes).await;
    assert_eq!(ev["payload"]["kind"], "error");
    assert!(
        ev["payload"]["detail"]
            .as_str()
            .unwrap()
            .contains("not delivered"),
        "{ev}"
    );
}

/// `call_tool` on a socket that also hears pushes: they are skipped.
async fn call_tool_quiet(ws: &mut Ws, name: &str, args: Value) -> Value {
    request_quiet(ws, "tool.call", json!({"name": name, "args": args})).await["result"].clone()
}

/// Send a request, skip pushes, return the payload of the reply.
async fn request_quiet(ws: &mut Ws, event: &str, payload: Value) -> Value {
    send(
        ws,
        json!({"event": event, "id": "q", "ts": 0, "payload": payload}),
    )
    .await;
    loop {
        let frame = next(ws).await.expect("socket closed");
        if frame.get("ref_id").is_some() {
            assert_eq!(frame["ref_id"], "q", "{frame}");
            return frame["payload"].clone();
        }
    }
}

/// Next push that is not `tasks.changed` or `memory.project` (the task log
/// line of a finished task).
async fn next_agent(ws: &mut Ws) -> Value {
    loop {
        let push = next(ws).await.expect("socket closed");
        if push["event"] != "tasks.changed" && push["event"] != "memory.project" {
            return push;
        }
    }
}

/// The `Task` of the next `tasks.changed` push; other pushes are skipped.
async fn next_task(ws: &mut Ws) -> Value {
    loop {
        let push = next(ws).await.expect("socket closed");
        if push["event"] == "tasks.changed" {
            assert!(push.get("ref_id").is_none());
            return push["payload"]["task"].clone();
        }
    }
}

#[tokio::test]
async fn delegate_task_status_is_pushed() {
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};
    let (shim_port, listener) = free_listener().await;
    let env = boot(OK_SCRIPT, shim_port).await;
    // A shim that also records the task id it was handed.
    let ready = env.channel.join("ready");
    let (tx, mut prompts) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
    let shim = axum::Router::new()
        .route(
            "/health",
            get({
                let ready = ready.clone();
                move || async move {
                    if ready.exists() {
                        StatusCode::OK
                    } else {
                        StatusCode::SERVICE_UNAVAILABLE
                    }
                }
            }),
        )
        .route(
            "/prompt",
            post(move |headers: HeaderMap, body: String| async move {
                let id = headers["x-task-id"].to_str().unwrap().to_string();
                tx.send((id, body)).unwrap();
                StatusCode::OK
            }),
        );
    tokio::spawn(async move { axum::serve(listener, shim).await.unwrap() });
    let (token, _) = env.pair().await;
    let mut ws = env.ws_authed(&token).await;
    let mut pushes = env.ws_authed(&token).await;
    let shim_says = |path: &'static str, body: Value| async {
        let (st, _) = env.post(path, body).await;
        assert_eq!(st, 200);
    };

    let list = request_quiet(&mut ws, "tasks.list", json!({})).await;
    assert_eq!(list, json!({"tasks": []}));

    // First task: no session, so queued -> spawning -> sent.
    let r = request_quiet(
        &mut ws,
        "tool.call",
        json!({"name": "delegate", "thread_id": "th-1",
               "args": {"project": "demo", "prompt": "add a note about tea"}}),
    )
    .await["result"]
        .clone();
    let first = r["task_id"].as_str().unwrap().to_string();
    assert_eq!(first.len(), 16);
    assert_eq!(
        (&r["state"], &r["queue_pos"]),
        (&json!("queued"), &json!(0))
    );
    let t = next_task(&mut pushes).await;
    assert_eq!(
        t,
        json!({"id": first, "project": "demo", "thread_id": "th-1", "prompt": "add a note about tea",
               "state": "queued", "queue_pos": 0, "created_at": t["created_at"], "updated_at": t["created_at"]})
    );
    assert!(t["created_at"].as_i64().unwrap() > 0);
    assert_eq!(next_task(&mut pushes).await["state"], "spawning");
    let t = next_task(&mut pushes).await;
    assert_eq!((&t["state"], &t["queue_pos"]), (&json!("sent"), &json!(0)));
    assert_eq!(
        prompts.recv().await.unwrap(),
        (first.clone(), "add a note about tea".to_string())
    );

    // Second task behind it: live session, no spawning.
    let r = call_tool_quiet(
        &mut ws,
        "delegate",
        json!({"project": "demo", "prompt": "second"}),
    )
    .await;
    let second = r["task_id"].as_str().unwrap().to_string();
    assert_eq!(
        (&r["state"], &r["queue_pos"]),
        (&json!("queued"), &json!(1))
    );
    let t = next_task(&mut pushes).await;
    assert_eq!(
        (&t["id"], &t["state"], &t["queue_pos"], t.get("thread_id")),
        (&json!(second), &json!("queued"), &json!(1), None)
    );
    let t = next_task(&mut pushes).await;
    assert_eq!((&t["state"], &t["queue_pos"]), (&json!("sent"), &json!(1)));
    assert_eq!(prompts.recv().await.unwrap().0, second);

    // The shim reports the turn of the first task.
    let ev = |kind: &str, detail: &str| json!({"project": "demo", "kind": kind, "detail": detail, "tasks": [first]});
    shim_says("/internal/agent/event", ev("start", "")).await;
    let t = next_task(&mut pushes).await;
    assert_eq!(
        (&t["id"], &t["state"], t.get("queue_pos")),
        (&json!(first), &json!("working"), None)
    );
    assert!(t["started_at"].is_i64());
    shim_says("/internal/agent/event", ev("activity", "Edit notes/tea.md")).await;
    let t = next_task(&mut pushes).await;
    assert_eq!(
        (&t["state"], &t["last_activity"]),
        (&json!("working"), &json!("Edit notes/tea.md"))
    );
    shim_says("/internal/agent/event", ev("waiting", "permission")).await;
    let t = next_task(&mut pushes).await;
    assert_eq!(
        (&t["state"], t.get("last_activity")),
        (&json!("waiting"), None)
    );
    shim_says("/internal/agent/event", ev("resume", "")).await;
    assert_eq!(next_task(&mut pushes).await["state"], "working");

    // A notice without task ids is spoken but is nobody's answer.
    shim_says(
        "/internal/agent/say",
        json!({"project": "demo", "text": "just a notice"}),
    )
    .await;
    shim_says(
        "/internal/agent/say",
        json!({"project": "demo", "text": "Tea note added.", "tasks": [first]}),
    )
    .await;
    let t = next_task(&mut pushes).await;
    assert_eq!(
        (&t["state"], &t["answer"]),
        (&json!("working"), &json!("Tea note added."))
    );
    // Done, and the second task moves up.
    shim_says("/internal/agent/event", ev("stop", "answered")).await;
    let t = next_task(&mut pushes).await;
    assert_eq!(
        (&t["id"], &t["state"], &t["answer"]),
        (&json!(first), &json!("done"), &json!("Tea note added."))
    );
    let t = next_task(&mut pushes).await;
    assert_eq!(
        (&t["id"], &t["state"], &t["queue_pos"]),
        (&json!(second), &json!("sent"), &json!(0))
    );

    // Pull side: `tasks.list` and the `tasks` tool, newest first.
    let ids = |v: &Value| -> Vec<String> {
        v["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_string())
            .collect()
    };
    let list = request_quiet(&mut ws, "tasks.list", json!({})).await;
    assert_eq!(ids(&list), [second.clone(), first.clone()]);
    assert_eq!(list["tasks"][1]["state"], "done");
    let tool = call_tool_quiet(&mut ws, "tasks", json!({"project": "demo"})).await;
    assert_eq!(ids(&tool), [second.clone(), first.clone()]);
    let tool = call_tool_quiet(&mut ws, "tasks", json!({"project": "other"})).await;
    assert_eq!(tool, json!({"tasks": []}));

    // The shim dies with the second task in it: two failed checks, then failed + spoken.
    std::fs::remove_file(&ready).unwrap();
    taltosbot::tasks::check_sessions(&env.running.app).await;
    taltosbot::tasks::check_sessions(&env.running.app).await;
    let t = next_task(&mut pushes).await;
    assert_eq!(
        (&t["id"], &t["state"], &t["error"]),
        (
            &json!(second),
            &json!("failed"),
            &json!("agent session is gone")
        )
    );
    let say = next_agent(&mut pushes).await;
    assert_eq!(say["event"], "agent.say");
    assert!(say["payload"]["text"].as_str().unwrap().contains("is gone"));
}

/// A delivery that fails ends the task as `failed`, with the reason.
#[tokio::test]
async fn delegate_failure_fails_the_task() {
    let (dead_port, listener) = free_listener().await;
    drop(listener);
    let env = boot("#!/bin/sh\necho 'tmux exploded' >&2\nexit 3\n", dead_port).await;
    let (token, _) = env.pair().await;
    let mut ws = env.ws_authed(&token).await;
    let mut pushes = env.ws_authed(&token).await;
    let r = call_tool_quiet(
        &mut ws,
        "delegate",
        json!({"project": "demo", "prompt": "hi"}),
    )
    .await;
    assert_eq!(next_task(&mut pushes).await["state"], "queued");
    assert_eq!(next_task(&mut pushes).await["state"], "spawning");
    let t = next_task(&mut pushes).await;
    assert_eq!((&t["id"], &t["state"]), (&r["task_id"], &json!("failed")));
    assert!(
        t["error"].as_str().unwrap().contains("tmux exploded"),
        "{t}"
    );
    assert!(t.get("queue_pos").is_none() && t.get("started_at").is_none());
}

#[tokio::test]
async fn state_dir_inside_vault_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("vault")).unwrap();
    let cfg = Config::parse(&format!(
        "listen_addr='127.0.0.1'\nlisten_port=0\nvault_path='{}/vault'",
        tmp.path().display()
    ))
    .unwrap();
    let vault = Path::new(&cfg.vault_path).canonicalize().unwrap();
    assert!(server::start(cfg, &vault.join(".state")).await.is_err());
}
