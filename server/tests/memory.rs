//! Pins and project memory end-to-end: a real server, a fake Gemini for the
//! fold, a fake channel shim, project folders in a temp dir.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use taltosbot::config::Config;
use taltosbot::server::{self, Running};
use tokio_tungstenite::tungstenite::Message;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// `(system instruction, user turn)` of every fold call.
type Folds = Arc<Mutex<Vec<(String, String)>>>;

const NOTES_LINE: &str = "Project notes kept by taskbot: .taskbot.md. Read it first.";
const STATE: &str = "## State\n- watering plan: drip";

struct Env {
    _tmp: tempfile::TempDir,
    running: Running,
    token: String,
    folds: Folds,
    /// `(task id, body)` of every prompt the shim of `garden` got.
    prompts: tokio::sync::mpsc::UnboundedReceiver<(String, String)>,
    /// Folder of project `garden`. Project `shed` sits next to it.
    garden: PathBuf,
}

async fn boot() -> Env {
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};

    let folds: Folds = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gemini_port = listener.local_addr().unwrap().port();
    let fake = axum::Router::new()
        .fallback(
            |State(folds): State<Folds>, axum::Json(body): axum::Json<Value>| async move {
                let text = |v: &Value| v["parts"][0]["text"].as_str().unwrap().to_string();
                folds
                    .lock()
                    .unwrap()
                    .push((text(&body["systemInstruction"]), text(&body["contents"][0])));
                axum::Json(json!({
                    "candidates": [{"content": {"parts": [{"text": STATE}]}}],
                    "usageMetadata": {"promptTokenCount": 10, "totalTokenCount": 15},
                }))
            },
        )
        .with_state(folds.clone());
    tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });

    // The shim of `garden` (first project to ask for a port): always up. A
    // prompt that says "explode" is refused, so its task fails.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let shim_port = listener.local_addr().unwrap().port();
    let (tx, prompts) = tokio::sync::mpsc::unbounded_channel();
    let shim = axum::Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .route(
            "/prompt",
            post(move |headers: HeaderMap, body: String| async move {
                let refused = body.contains("explode");
                let id = headers["x-task-id"].to_str().unwrap().to_string();
                tx.send((id, body)).unwrap();
                if refused {
                    StatusCode::INTERNAL_SERVER_ERROR
                } else {
                    StatusCode::OK
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, shim).await.unwrap() });

    // tempfile honours $TMPDIR: the folders land on the data drive.
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    for d in ["vault", "channel", "garden", "shed"] {
        std::fs::create_dir_all(p.join(d)).unwrap();
    }
    let cfg = Config::parse(&format!(
        r#"
listen_addr = "127.0.0.1"
listen_port = 0
vault_path = "{root}/vault"
gemini_base_url = "http://127.0.0.1:{gemini_port}/"
channel_dir = "{root}/channel"
channel_port_base = {shim_port}
[keys]
gemini_api_key = "test-key"
[projects.garden]
dir = "{root}/garden"
[projects.shed]
dir = "{root}/shed"
"#,
        root = p.display()
    ))
    .unwrap();
    let running = server::start(cfg, &p.join("state")).await.unwrap();

    let http = reqwest::Client::new();
    let base = format!("http://{}", running.addr);
    let post_json = |path: &'static str, body: Value| {
        let (http, base) = (http.clone(), base.clone());
        async move {
            let sent = http.post(format!("{base}{path}")).json(&body).send();
            sent.await.unwrap().json::<Value>().await.unwrap()
        }
    };
    let code = post_json("/api/v1/admin/pair/new", json!({})).await;
    let paired = post_json("/api/v1/pair", json!({"code": code["code"], "name": "t"})).await;
    Env {
        garden: p.join("garden"),
        _tmp: tmp,
        running,
        token: paired["device_token"].as_str().unwrap().to_string(),
        folds,
        prompts,
    }
}

impl Env {
    async fn ws(&self) -> Ws {
        let url = format!("ws://{}/api/v1/ws", self.running.addr);
        let mut ws = tokio_tungstenite::connect_async(url).await.unwrap().0;
        let auth = json!({"event": "auth", "payload": {"token": self.token}});
        ws.send(Message::Text(auth.to_string().into()))
            .await
            .unwrap();
        assert_eq!(next(&mut ws).await["event"], "auth.ok");
        ws
    }

    fn file(&self) -> PathBuf {
        self.garden.join(".taskbot.md")
    }

    fn file_text(&self) -> String {
        std::fs::read_to_string(self.file()).unwrap()
    }

    /// What the shim would send for a finished turn.
    async fn shim_says(&self, path: &str, body: Value) {
        let url = format!("http://{}{path}", self.running.addr);
        let sent = reqwest::Client::new().post(url).json(&body).send();
        assert_eq!(sent.await.unwrap().status(), 200);
    }
}

async fn next(ws: &mut Ws) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("no frame within 10 s");
        if let Some(Ok(Message::Text(t))) = msg {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

/// Reply to a request, skipping pushes. `Ok` = `<event>.result` payload, `Err` = error text.
async fn request(ws: &mut Ws, event: &str, payload: Value) -> Result<Value, String> {
    let frame = json!({"event": event, "id": "r", "ts": 0, "payload": payload});
    ws.send(Message::Text(frame.to_string().into()))
        .await
        .unwrap();
    loop {
        let reply = next(ws).await;
        if reply.get("ref_id").is_none() {
            continue;
        }
        return if reply["event"] == "error" {
            Err(reply["payload"]["error"].as_str().unwrap().to_string())
        } else {
            assert_eq!(reply["event"], format!("{event}.result"), "{reply}");
            Ok(reply["payload"].clone())
        };
    }
}

/// A tool call from `thread` (`""` = no `thread_id`). Returns the tool result.
async fn call(ws: &mut Ws, thread: &str, name: &str, args: Value) -> Value {
    let mut payload = json!({"name": name, "args": args});
    if !thread.is_empty() {
        payload["thread_id"] = json!(thread);
    }
    let frame = json!({"event": "tool.call", "id": "t", "ts": 0, "payload": payload});
    ws.send(Message::Text(frame.to_string().into()))
        .await
        .unwrap();
    loop {
        let reply = next(ws).await;
        if reply.get("ref_id").is_some() {
            assert_eq!(reply["event"], "tool.result", "{reply}");
            return reply["payload"]["result"].clone();
        }
    }
}

/// Payload of the next push named `event`; other frames are skipped.
async fn next_push(ws: &mut Ws, event: &str) -> Value {
    loop {
        let frame = next(ws).await;
        if frame["event"] == event {
            assert!(frame.get("ref_id").is_none());
            return frame["payload"].clone();
        }
    }
}

async fn new_thread(ws: &mut Ws, name: &str) -> String {
    let made = request(ws, "threads.create", json!({"name": name}))
        .await
        .unwrap();
    made["thread"]["id"].as_str().unwrap().to_string()
}

async fn get(ws: &mut Ws, thread: &str) -> Value {
    request(ws, "threads.get", json!({"id": thread}))
        .await
        .unwrap()
}

async fn note(ws: &mut Ws, thread: &str, text: &str) -> Value {
    call(ws, thread, "memory_note", json!({"text": text})).await
}

fn texts(pins: &Value) -> Vec<&str> {
    pins.as_array()
        .unwrap_or_else(|| panic!("no pin list: {pins}"))
        .iter()
        .map(|p| p["text"].as_str().unwrap())
        .collect()
}

/// Six turns: enough for a compact to fold the first two.
async fn talk(ws: &mut Ws, thread: &str) {
    let entries: Vec<Value> = (1..=6)
        .flat_map(|n| {
            [
                json!({"id": format!("u{n}"), "ts": n, "kind": "user", "text": format!("question {n}")}),
                json!({"id": format!("m{n}"), "ts": n, "kind": "model", "text": format!("answer {n}")}),
            ]
        })
        .collect();
    request(
        ws,
        "threads.append",
        json!({"id": thread, "entries": entries}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn thread_pins_note_dedupe_replace() {
    let env = boot().await;
    let mut ws = env.ws().await;
    let mut watcher = env.ws().await;
    let thread = new_thread(&mut ws, "Garden").await;

    // Pins are there before any compact: empty lists, no memory.
    let got = get(&mut ws, &thread).await;
    assert_eq!(got["pins"], json!({"pins": [], "size": 0, "limit": 1500}));
    assert_eq!(got["always"], json!({"pins": [], "size": 0, "limit": 1200}));
    assert!(got.get("memory").is_none() && got.get("project_memory").is_none());

    let r = note(&mut ws, "", "budget max 200 EUR").await;
    assert!(r["error"].as_str().unwrap().contains("thread_id"), "{r}");
    let r = note(&mut ws, "0000000000000000", "x").await;
    assert!(
        r["error"].as_str().unwrap().contains("unknown thread"),
        "{r}"
    );

    let r = note(&mut ws, &thread, "budget max 200 EUR").await;
    let at = r["pins"][0]["at"].as_i64().unwrap();
    assert!(at > 1_600_000_000_000, "epoch ms");
    assert_eq!(
        r,
        json!({"scope": "thread", "pins": [{"text": "budget max 200 EUR", "at": at}],
               "size": 19, "limit": 1500})
    );
    let push = next_push(&mut watcher, "threads.memory").await;
    assert_eq!(
        push,
        json!({"thread_id": thread, "usage": {"input": 0, "output": 0},
               "pins": {"pins": r["pins"], "size": 19, "limit": 1500}}),
        "no memory yet: the field is absent"
    );
    assert_eq!(get(&mut ws, &thread).await["pins"], push["pins"]);

    // Same text again, any case: one pin.
    let r = note(&mut ws, &thread, " BUDGET max 200 eur\n").await;
    assert_eq!(texts(&r["pins"]), ["budget max 200 EUR"]);
    assert!(r.get("error").is_none());

    note(&mut ws, &thread, "notes go under beds/").await;
    note(&mut ws, &thread, "drip, not spray").await;
    // replaces: one hit swaps in place and keeps the order.
    let r = call(
        &mut ws,
        &thread,
        "memory_note",
        json!({"text": "budget max 300 EUR", "replaces": "BUDGET"}),
    )
    .await;
    assert_eq!(
        texts(&r["pins"]),
        [
            "budget max 300 EUR",
            "notes go under beds/",
            "drip, not spray"
        ]
    );
    assert!(r["pins"][0]["at"].as_i64().unwrap() >= at);
    // No hit: plain add.
    let r = call(
        &mut ws,
        &thread,
        "memory_note",
        json!({"text": "pump: 12 V", "replaces": "hose"}),
    )
    .await;
    assert_eq!(texts(&r["pins"]).len(), 4);
    assert_eq!(texts(&r["pins"])[3], "pump: 12 V");
    // Two hits: nothing changes, the matches come back.
    let r = call(
        &mut ws,
        &thread,
        "memory_note",
        json!({"text": "x", "replaces": "p"}),
    )
    .await;
    assert_eq!(r["error"], "2 notes match", "{r}");
    assert_eq!(r["matches"], json!(["drip, not spray", "pump: 12 V"]));

    // A runaway text is cut to one line of 300 chars.
    let r = note(&mut ws, &thread, &"long lines\n".repeat(100)).await;
    let long = texts(&r["pins"])[4];
    assert_eq!(long.chars().count(), 300);
    assert!(long.starts_with("long lines long lines") && !long.contains('\n'));

    for bad in [
        json!({"text": "  \n "}),
        json!({}),
        json!({"text": "x", "scope": "galaxy"}),
    ] {
        let r = call(&mut ws, &thread, "memory_note", bad.clone()).await;
        assert!(r["error"].is_string(), "{bad}: {r}");
    }
    assert_eq!(texts(&get(&mut ws, &thread).await["pins"]["pins"]).len(), 5);
}

#[tokio::test]
async fn no_count_cap_but_a_size_limit() {
    let env = boot().await;
    let mut ws = env.ws().await;
    let thread = new_thread(&mut ws, "T").await;

    // 50 short pins: all stored, far from the warn mark.
    for i in 0..50 {
        let r = note(&mut ws, &thread, &format!("p{i:02}")).await;
        assert!(
            r.get("warning").is_none() && r.get("error").is_none(),
            "{r}"
        );
    }
    let got = get(&mut ws, &thread).await;
    assert_eq!(texts(&got["pins"]["pins"]).len(), 50);
    assert_eq!(got["pins"]["size"], 200);

    // `always`: limit 1200, warn mark 960.
    let always = |text: String| json!({"text": text, "scope": "always"});
    let pin = |name: &str, chars: usize| format!("{name}{}", "x".repeat(chars - name.len()));
    for name in ["alpha", "bravo", "delta"] {
        let r = call(&mut ws, "", "memory_note", always(pin(name, 299))).await;
        assert!(r.get("warning").is_none(), "{r}");
    }
    let r = call(&mut ws, "", "memory_note", always(pin("echo", 99))).await;
    assert_eq!((&r["size"], &r["limit"]), (&json!(1000), &json!(1200)));
    assert_eq!(
        r["warning"],
        "always memory 83% full: tell the user once, offer to clean it up"
    );
    // A write past the limit stores nothing and hands back the list.
    let r = call(&mut ws, "", "memory_note", always(pin("golf", 299))).await;
    assert_eq!(r["error"], "always memory full", "{r}");
    assert_eq!((&r["size"], &r["limit"]), (&json!(1000), &json!(1200)));
    assert_eq!(texts(&r["pins"]).len(), 4);
    assert_eq!(env.running.app.store.pins(None).unwrap().len(), 4);
    // Exactly full is not over.
    let r = call(&mut ws, "", "memory_note", always(pin("hotel", 199))).await;
    assert_eq!(r["size"], 1200);
    assert!(r["warning"].as_str().unwrap().contains("100% full"));
    // Full: a growing swap is refused, a shrinking one works.
    let r = call(
        &mut ws,
        "",
        "memory_note",
        json!({"text": pin("echo", 100), "scope": "always", "replaces": "echo"}),
    )
    .await;
    assert_eq!(r["error"], "always memory full", "{r}");
    let r = call(
        &mut ws,
        "",
        "memory_note",
        json!({"text": "echo short", "scope": "always", "replaces": "echo"}),
    )
    .await;
    assert_eq!(r["size"], 1200 - 100 + 11, "{r}");
    assert_eq!(texts(&r["pins"])[3], "echo short", "in place");
    // The thread scope has its own limit.
    assert_eq!(get(&mut ws, &thread).await["pins"]["size"], 200);
}

#[tokio::test]
async fn always_pins_reach_every_thread() {
    let env = boot().await;
    let mut ws = env.ws().await;
    let mut watcher = env.ws().await;
    let first = new_thread(&mut ws, "One").await;
    let second = new_thread(&mut ws, "Two").await;

    let r = call(
        &mut ws,
        &first,
        "memory_note",
        json!({"text": "answer in Hungarian", "scope": "always"}),
    )
    .await;
    assert_eq!(r["scope"], "always");
    assert_eq!(texts(&r["pins"]), ["answer in Hungarian"]);
    let push = next_push(&mut watcher, "memory.always").await;
    assert_eq!(
        push,
        json!({"pins": r["pins"], "size": 20, "limit": 1200}),
        "the bare list, no thread"
    );
    // Stored with no thread: no thread holds it as its own pin.
    assert_eq!(env.running.app.store.pins(None).unwrap().len(), 1);
    for thread in [&first, &second] {
        let got = get(&mut ws, thread).await;
        assert_eq!(got["always"], push);
        assert_eq!(got["pins"]["pins"], json!([]));
    }
    let made = request(&mut ws, "threads.create", json!({})).await.unwrap();
    assert_eq!(made["always"], push);
    // The push of a new thread stays `{thread}`.
    let changed = next_push(&mut watcher, "threads.changed").await;
    assert_eq!(changed.as_object().unwrap().len(), 1);

    // No thread_id needed for `always`.
    let r = call(
        &mut ws,
        "",
        "memory_note",
        json!({"text": "vault notes: terse", "scope": "always"}),
    )
    .await;
    assert_eq!(texts(&r["pins"]).len(), 2);
}

#[tokio::test]
async fn forget_searches_thread_then_always_and_never_the_state() {
    let env = boot().await;
    let mut ws = env.ws().await;
    let mut watcher = env.ws().await;
    let thread = new_thread(&mut ws, "Garden").await;
    for text in [
        "budget max 200 EUR",
        "budget talk on Friday",
        "notes go under beds/",
    ] {
        note(&mut ws, &thread, text).await;
    }
    call(
        &mut ws,
        "",
        "memory_note",
        json!({"text": "answer in Hungarian", "scope": "always"}),
    )
    .await;
    let forget = |needle: &str| json!({"match": needle});

    let r = call(&mut ws, &thread, "memory_forget", forget("BEDS")).await;
    assert_eq!(
        (&r["forgotten"], &r["from"], &r["limit"]),
        (
            &json!("notes go under beds/"),
            &json!("thread"),
            &json!(1500)
        )
    );
    assert_eq!(
        texts(&r["pins"]),
        ["budget max 200 EUR", "budget talk on Friday"]
    );
    assert_eq!(r["size"], 19 + 22);
    while texts(&next_push(&mut watcher, "threads.memory").await["pins"]["pins"]).len() != 2 {}

    let r = call(&mut ws, &thread, "memory_forget", forget("budget")).await;
    assert_eq!(r["error"], "2 notes match", "{r}");
    assert_eq!(
        r["matches"],
        json!(["budget max 200 EUR", "budget talk on Friday"])
    );
    let r = call(&mut ws, &thread, "memory_forget", forget("pump")).await;
    assert_eq!(r["error"], "no pinned note matches", "{r}");
    assert_eq!(
        texts(&r["pins"]),
        [
            "budget max 200 EUR",
            "budget talk on Friday",
            "answer in Hungarian"
        ],
        "every pin looked at"
    );

    // A line that sits only in `## State` is not a pin: error, memory untouched.
    talk(&mut ws, &thread).await;
    let done = request(&mut ws, "threads.compact", json!({"id": thread}))
        .await
        .unwrap();
    assert_eq!(done["memory"]["text"], STATE);
    let r = call(&mut ws, &thread, "memory_forget", forget("watering plan")).await;
    assert_eq!(r["error"], "no pinned note matches", "{r}");
    assert_eq!(get(&mut ws, &thread).await["memory"], done["memory"]);

    // No thread_id: only `always` is searched.
    let r = call(&mut ws, "", "memory_forget", forget("budget max")).await;
    assert_eq!(r["error"], "no pinned note matches", "{r}");
    let r = call(&mut ws, &thread, "memory_forget", forget("hungarian")).await;
    assert_eq!(
        (&r["forgotten"], &r["from"], &r["pins"], &r["size"]),
        (
            &json!("answer in Hungarian"),
            &json!("always"),
            &json!([]),
            &json!(0)
        )
    );
    assert_eq!(
        next_push(&mut watcher, "memory.always").await["pins"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "the push of the note"
    );
    assert_eq!(
        next_push(&mut watcher, "memory.always").await,
        json!({"pins": [], "size": 0, "limit": 1200})
    );
    let r = call(&mut ws, &thread, "memory_forget", forget("  ")).await;
    assert!(r["error"].is_string());
}

#[tokio::test]
async fn fold_reads_the_pins_and_leaves_them_alone() {
    let env = boot().await;
    let mut ws = env.ws().await;
    let mut watcher = env.ws().await;
    let thread = new_thread(&mut ws, "Garden").await;
    request(
        &mut ws,
        "threads.set_project",
        json!({"id": thread, "project": "garden"}),
    )
    .await
    .unwrap();
    for (text, scope) in [
        ("answer in Hungarian", "always"),
        ("notes go under beds/", "project"),
        ("budget max 200 EUR", "thread"),
    ] {
        let r = call(
            &mut ws,
            &thread,
            "memory_note",
            json!({"text": text, "scope": scope}),
        )
        .await;
        assert_eq!(r["scope"], scope, "{r}");
    }
    let before = get(&mut ws, &thread).await;

    talk(&mut ws, &thread).await;
    let done = request(&mut ws, "threads.compact", json!({"id": thread}))
        .await
        .unwrap();
    assert_eq!(done["memory"]["text"], STATE);
    assert_eq!(done["pins"], before["pins"], "the fold never writes a pin");
    let push = loop {
        let push = next_push(&mut watcher, "threads.memory").await;
        if push.get("memory").is_some() {
            break push;
        }
    };
    assert_eq!(push["pins"], before["pins"]);
    assert_eq!(push["memory"], done["memory"]);
    let after = get(&mut ws, &thread).await;
    assert_eq!(
        (&after["pins"], &after["always"]),
        (&before["pins"], &before["always"])
    );

    let folds = env.folds.lock().unwrap();
    let (system, asked) = &folds[0];
    assert!(system.contains("Never repeat one in State") && system.contains("contradicts"));
    assert!(
        asked.starts_with(
            "<memory>\n\n</memory>\n<pinned>\n- answer in Hungarian\n- notes go under beds/\n\
             - budget max 200 EUR\n</pinned>\n<conversation>\nUser: question 1\n"
        ),
        "{asked}"
    );
}

#[tokio::test]
async fn memory_read_lists_and_reads_other_threads() {
    let env = boot().await;
    let mut ws = env.ws().await;
    let me = new_thread(&mut ws, "Kitchen").await;
    let beds = new_thread(&mut ws, "Garden beds").await;
    let shed = new_thread(&mut ws, "Garden shed").await;
    let _empty = new_thread(&mut ws, "Nothing here").await;
    note(&mut ws, &me, "oven: 180 C").await;
    note(&mut ws, &beds, "budget max 200 EUR").await;
    request(
        &mut ws,
        "threads.set_project",
        json!({"id": shed, "project": "shed"}),
    )
    .await
    .unwrap();
    talk(&mut ws, &shed).await;
    request(&mut ws, "threads.compact", json!({"id": shed}))
        .await
        .unwrap();

    // The list: threads with pins or a memory, the asking one left out.
    let r = call(&mut ws, &me, "memory_read", json!({})).await;
    assert_eq!(
        r,
        json!({"threads": [
            {"id": shed, "name": "Garden shed", "project": "shed", "pins": 0, "memory": true},
            {"id": beds, "name": "Garden beds", "pins": 1, "memory": false},
        ], "projects": []}),
        "newest access first"
    );
    let r = call(
        &mut ws,
        "",
        "memory_read",
        json!({"thread": "", "project": ""}),
    )
    .await;
    assert_eq!(r["threads"].as_array().unwrap().len(), 3, "no caller: all");

    // One thread: by id, by part of the name.
    let r = call(&mut ws, &me, "memory_read", json!({"thread": beds})).await;
    assert_eq!(r["thread"], json!({"id": beds, "name": "Garden beds"}));
    assert_eq!(texts(&r["pins"]), ["budget max 200 EUR"]);
    assert!(r.get("memory").is_none());
    let r = call(&mut ws, &me, "memory_read", json!({"thread": "SHED"})).await;
    assert_eq!(r["thread"]["id"], json!(shed));
    assert_eq!((&r["pins"], &r["memory"]), (&json!([]), &json!(STATE)));
    let r = call(&mut ws, &me, "memory_read", json!({"thread": "garden"})).await;
    let e = r["error"].as_str().unwrap();
    assert!(
        e.starts_with("2 threads match") && e.contains("Garden beds") && e.contains("Garden shed"),
        "{e}"
    );
    let r = call(&mut ws, &me, "memory_read", json!({"thread": "attic"})).await;
    assert!(r["error"].as_str().unwrap().contains("no thread matches"));
    // Reading changes nothing.
    assert_eq!(
        texts(&get(&mut ws, &beds).await["pins"]["pins"]),
        ["budget max 200 EUR"]
    );

    // Projects: listed once they have the file, read by name.
    let r = call(&mut ws, &me, "memory_read", json!({"project": "garden"})).await;
    assert!(r["error"].as_str().unwrap().contains(".taskbot.md"), "{r}");
    std::fs::write(env.file(), "# Taskbot: garden\n\nfree text\n").unwrap();
    let r = call(&mut ws, &me, "memory_read", json!({"project": "garden"})).await;
    assert_eq!(
        r,
        json!({"project": "garden", "text": "# Taskbot: garden\n\nfree text\n"})
    );
    let r = call(&mut ws, &me, "memory_read", json!({})).await;
    assert_eq!(r["projects"], json!(["garden"]));
    let r = call(&mut ws, &me, "memory_read", json!({"project": "nope"})).await;
    assert!(r["error"].as_str().unwrap().contains("unknown project"));
}

#[tokio::test]
async fn active_project_set_clear_and_adopted_once() {
    let mut env = boot().await;
    let mut ws = env.ws().await;
    let mut watcher = env.ws().await;
    let thread = new_thread(&mut ws, "T").await;
    next_push(&mut watcher, "threads.changed").await;
    let project = |v: &Value| v["thread"].get("project").cloned();

    // `delegate` from a thread without a project: that project becomes active.
    let r = call(
        &mut ws,
        &thread,
        "delegate",
        json!({"project": "garden", "prompt": "weed the beds"}),
    )
    .await;
    assert_eq!(r["status"], "sent", "{r}");
    let changed = next_push(&mut watcher, "threads.changed").await;
    assert_eq!(changed["thread"]["id"], json!(thread));
    assert_eq!(project(&changed), Some(json!("garden")));
    // No project file yet: the agent gets the prompt as given.
    assert_eq!(env.prompts.recv().await.unwrap().1, "weed the beds");

    // The tool: set, clear, unknown name, no thread.
    let r = call(&mut ws, &thread, "set_project", json!({"project": "shed"})).await;
    assert_eq!(project(&r), Some(json!("shed")));
    assert_eq!(next_push(&mut watcher, "threads.changed").await, r);
    // Has one already: a `delegate` elsewhere does not move it.
    call(
        &mut ws,
        &thread,
        "delegate",
        json!({"project": "garden", "prompt": "second"}),
    )
    .await;
    env.prompts.recv().await.unwrap();
    assert_eq!(project(&get(&mut ws, &thread).await), Some(json!("shed")));

    let r = call(&mut ws, &thread, "set_project", json!({"project": ""})).await;
    assert_eq!(project(&r), None, "{r}");
    assert_eq!(r["thread"].as_object().unwrap().len(), 4);
    assert_eq!(next_push(&mut watcher, "threads.changed").await, r);
    let r = call(&mut ws, &thread, "set_project", json!({"project": "attic"})).await;
    let e = r["error"].as_str().unwrap();
    assert!(e.contains("unknown project") && e.contains("garden") && e.contains("shed"));
    let r = call(&mut ws, "", "set_project", json!({"project": "shed"})).await;
    assert!(r["error"].as_str().unwrap().contains("thread_id"));
    assert!(call(&mut ws, &thread, "set_project", json!({})).await["error"].is_string());

    // The request of the picker.
    let set = |project: &str| json!({"id": thread, "project": project});
    let r = request(&mut ws, "threads.set_project", set("garden"))
        .await
        .unwrap();
    assert_eq!(project(&r), Some(json!("garden")));
    assert_eq!(next_push(&mut watcher, "threads.changed").await, r);
    let listed = request(&mut ws, "threads.list", json!({})).await.unwrap();
    assert_eq!(listed["threads"][0]["project"], "garden");
    let r = request(&mut ws, "threads.set_project", set(""))
        .await
        .unwrap();
    assert_eq!(project(&r), None);
    assert_eq!(next_push(&mut watcher, "threads.changed").await, r);
    for bad in [
        set("attic"),
        json!({"id": thread}),
        json!({"id": "nope", "project": "garden"}),
        json!({"project": "garden"}),
    ] {
        let e = request(&mut ws, "threads.set_project", bad.clone()).await;
        assert!(e.is_err(), "{bad}");
    }
    assert_eq!(project(&get(&mut ws, &thread).await), None);
}

#[tokio::test]
async fn project_notes_live_in_the_file() {
    let mut env = boot().await;
    let mut ws = env.ws().await;
    let mut watcher = env.ws().await;
    let thread = new_thread(&mut ws, "Garden").await;
    let project_note = |text: &str| json!({"text": text, "scope": "project"});

    // No active project: nothing to write to.
    let r = call(&mut ws, &thread, "memory_note", project_note("x")).await;
    assert_eq!(r["error"], "no active project", "{r}");
    let r = call(&mut ws, "", "memory_note", project_note("x")).await;
    assert!(r["error"].is_string());
    call(
        &mut ws,
        &thread,
        "set_project",
        json!({"project": "garden"}),
    )
    .await;
    assert!(get(&mut ws, &thread).await.get("project_memory").is_none());
    assert!(!env.file().exists(), "never made on read");

    // The first write makes the file.
    let r = call(
        &mut ws,
        &thread,
        "memory_note",
        project_note("notes go under beds/"),
    )
    .await;
    let made = "# Taskbot: garden\n\n## Pinned\n- notes go under beds/\n";
    assert_eq!(env.file_text(), made);
    assert_eq!(
        (&r["scope"], &r["size"], &r["limit"]),
        (&json!("project"), &json!(made.len()), &json!(3000))
    );
    assert_eq!(texts(&r["pins"]), ["notes go under beds/"]);
    assert!(r["pins"][0]["at"].as_i64().unwrap() > 1_600_000_000_000);
    let memory = json!({"project": "garden", "text": made, "size": made.len(), "limit": 3000});
    assert_eq!(next_push(&mut watcher, "memory.project").await, memory);
    assert_eq!(get(&mut ws, &thread).await["project_memory"], memory);

    // A file the user and the agent wrote in: only one line moves per write.
    let by_hand = |pinned: &str| {
        format!(
            "# Taskbot: garden\n\nSoil is clay. Árvíztűrő.\n\n## Pinned\n{pinned}\n## Layout\n\
             - bed 1: north\n\n```\n## Pinned\n- inside a fence\n```\n"
        )
    };
    std::fs::write(env.file(), by_hand("- notes go under beds/\n")).unwrap();
    let r = call(
        &mut ws,
        &thread,
        "memory_note",
        project_note("budget max 200 EUR"),
    )
    .await;
    assert_eq!(
        texts(&r["pins"]),
        ["notes go under beds/", "budget max 200 EUR"]
    );
    assert_eq!(
        env.file_text(),
        by_hand("- notes go under beds/\n- budget max 200 EUR\n")
    );
    // Same text: no write. Swap in place.
    call(
        &mut ws,
        &thread,
        "memory_note",
        project_note("BUDGET max 200 eur"),
    )
    .await;
    let r = call(
        &mut ws,
        &thread,
        "memory_note",
        json!({"text": "notes go under plots/", "scope": "project", "replaces": "beds"}),
    )
    .await;
    assert_eq!(
        texts(&r["pins"]),
        ["notes go under plots/", "budget max 200 EUR"]
    );
    assert_eq!(
        env.file_text(),
        by_hand("- notes go under plots/\n- budget max 200 EUR\n")
    );
    // Forget: thread pins first, then the project's.
    let r = call(
        &mut ws,
        &thread,
        "memory_forget",
        json!({"match": "budget"}),
    )
    .await;
    assert_eq!(
        (&r["forgotten"], &r["from"]),
        (&json!("budget max 200 EUR"), &json!("project"))
    );
    assert_eq!(texts(&r["pins"]), ["notes go under plots/"]);
    assert_eq!(env.file_text(), by_hand("- notes go under plots/\n"));
    let r = call(
        &mut ws,
        &thread,
        "memory_forget",
        json!({"match": "inside a fence"}),
    )
    .await;
    assert_eq!(r["error"], "no pinned note matches", "{r}");

    // A file without the section: it goes to the end.
    std::fs::write(env.file(), "Just my notes.\n").unwrap();
    call(&mut ws, &thread, "memory_note", project_note("a")).await;
    assert_eq!(env.file_text(), "Just my notes.\n\n## Pinned\n- a\n");

    // The file is there: the agent is told to read it. `Task.prompt` stays.
    let r = call(
        &mut ws,
        &thread,
        "delegate",
        json!({"project": "garden", "prompt": "split plan.md\nper bed"}),
    )
    .await;
    let done_id = r["task_id"].as_str().unwrap().to_string();
    let (id, body) = env.prompts.recv().await.unwrap();
    assert_eq!(id, done_id);
    assert_eq!(body, format!("{NOTES_LINE}\n\nsplit plan.md\nper bed"));
    let listed = request(&mut ws, "tasks.list", json!({})).await.unwrap();
    assert_eq!(listed["tasks"][0]["prompt"], "split plan.md\nper bed");

    // A task that ends leaves one line under `## Tasks`, newest first.
    while next_push(&mut watcher, "tasks.changed").await["task"]["state"] != "sent" {}
    env.shim_says(
        "/internal/agent/say",
        json!({"project": "garden", "text": "4 files\nmade", "tasks": [done_id]}),
    )
    .await;
    env.shim_says(
        "/internal/agent/event",
        json!({"project": "garden", "kind": "stop", "detail": "", "tasks": [done_id]}),
    )
    .await;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let done_line = format!("- {today} split plan.md per bed -> done: 4 files made\n");
    let with_done = format!("Just my notes.\n\n## Pinned\n- a\n\n## Tasks\n{done_line}");
    let push = loop {
        let push = next_push(&mut watcher, "memory.project").await;
        if push["text"].as_str().unwrap().contains("## Tasks") {
            break push;
        }
    };
    assert_eq!(
        push,
        json!({"project": "garden", "text": with_done, "size": with_done.len(), "limit": 3000})
    );
    assert_eq!(env.file_text(), with_done);

    call(
        &mut ws,
        &thread,
        "delegate",
        json!({"project": "garden", "prompt": "explode"}),
    )
    .await;
    let push = next_push(&mut watcher, "memory.project").await;
    let text = push["text"].as_str().unwrap();
    let failed = format!("## Tasks\n- {today} explode -> failed: shim on port ");
    assert!(
        text.contains(&failed) && text.ends_with(&done_line),
        "{text}"
    );
    assert_eq!(env.file_text(), text);
}

#[tokio::test]
async fn project_file_past_its_limit_or_unreadable() {
    let env = boot().await;
    let mut ws = env.ws().await;
    let thread = new_thread(&mut ws, "Garden").await;
    call(
        &mut ws,
        &thread,
        "set_project",
        json!({"project": "garden"}),
    )
    .await;
    let project_note = |text: &str| json!({"text": text, "scope": "project"});

    // Under the warn mark, then over it.
    let lines =
        |n: usize| format!("{}\n", "- 48 chars of hand-made project notes, line......").repeat(n);
    assert_eq!(lines(1).len(), 50);
    std::fs::write(env.file(), lines(40)).unwrap();
    let r = call(&mut ws, &thread, "memory_note", project_note("a")).await;
    assert!(
        r.get("warning").is_none() && r.get("error").is_none(),
        "{r}"
    );
    std::fs::write(env.file(), lines(50)).unwrap();
    let r = call(&mut ws, &thread, "memory_note", project_note("a")).await;
    assert_eq!(
        r["size"],
        2500 + "\n## Pinned\n- a\n".len(),
        "the whole file counts"
    );
    assert_eq!(
        r["warning"],
        "project memory 83% full: tell the user once, offer to clean it up"
    );

    // Grown past the limit by hand: left alone, served cut on a line end.
    let big = lines(100);
    std::fs::write(env.file(), &big).unwrap();
    let got = get(&mut ws, &thread).await;
    let memory = &got["project_memory"];
    assert_eq!(
        (&memory["project"], &memory["size"], &memory["limit"]),
        (&json!("garden"), &json!(5000), &json!(3000))
    );
    assert_eq!(memory["text"], lines(60), "3000 chars, whole lines");
    let r = call(&mut ws, &thread, "memory_note", project_note("one more")).await;
    assert_eq!(r["error"], "project memory full", "{r}");
    assert_eq!((&r["size"], &r["limit"]), (&json!(5000), &json!(3000)));
    assert_eq!(env.file_text(), big, "file unchanged");
    let r = call(
        &mut ws,
        &thread,
        "memory_read",
        json!({"project": "garden"}),
    )
    .await;
    assert_eq!(r["text"], lines(60));

    // Not UTF-8: no write, and reads go on as if there was no file.
    let raw = [b'#', b' ', 0xff, 0xfe, b'\n'];
    std::fs::write(env.file(), raw).unwrap();
    let r = call(&mut ws, &thread, "memory_note", project_note("x")).await;
    assert!(r["error"].as_str().unwrap().contains("not UTF-8"), "{r}");
    assert!(get(&mut ws, &thread).await.get("project_memory").is_none());
    let r = call(&mut ws, &thread, "memory_forget", json!({"match": "x"})).await;
    assert_eq!(r["error"], "no pinned note matches", "{r}");
    assert_eq!(std::fs::read(env.file()).unwrap(), raw, "left alone");
}
