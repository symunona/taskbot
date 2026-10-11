//! Thread memory end-to-end: a real server, a fake Gemini `generateContent`.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use taltosbot::config::Config;
use taltosbot::server::{self, Running};
use tokio_tungstenite::tungstenite::Message;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// What the fake Gemini answers next, and what it was asked.
struct Gemini {
    /// `Ok` = answer text, `Err` = HTTP status.
    answer: Result<String, u16>,
    delay: Duration,
    /// `(path, api key header, body)` per call.
    calls: Vec<(String, String, Value)>,
}

struct Env {
    _tmp: tempfile::TempDir,
    running: Running,
    gemini: Arc<Mutex<Gemini>>,
    token: String,
}

async fn boot() -> Env {
    use axum::extract::{Request, State};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let gemini = Arc::new(Mutex::new(Gemini {
        answer: Ok("## State\n- first".into()),
        delay: Duration::ZERO,
        calls: Vec::new(),
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gemini_port = listener.local_addr().unwrap().port();
    let fake = axum::Router::new()
        .fallback(
            |State(gemini): State<Arc<Mutex<Gemini>>>, req: Request| async move {
                let path = req.uri().path().to_string();
                let key = req.headers()["x-goog-api-key"]
                    .to_str()
                    .unwrap()
                    .to_string();
                let body = axum::body::to_bytes(req.into_body(), 1 << 20)
                    .await
                    .unwrap();
                let (answer, delay) = {
                    let mut g = gemini.lock().unwrap();
                    g.calls
                        .push((path, key, serde_json::from_slice(&body).unwrap()));
                    (g.answer.clone(), g.delay)
                };
                tokio::time::sleep(delay).await;
                match answer {
                    Ok(text) => axum::Json(json!({
                        "candidates": [{"content": {"parts": [
                            {"text": "thinking out loud", "thought": true},
                            {"text": text},
                        ]}}],
                        "usageMetadata": {"promptTokenCount": 100, "totalTokenCount": 130},
                    }))
                    .into_response(),
                    Err(status) => StatusCode::from_u16(status).unwrap().into_response(),
                }
            },
        )
        .with_state(gemini.clone());
    tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });

    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("vault")).unwrap();
    let cfg = Config::parse(&format!(
        r#"
listen_addr = "127.0.0.1"
listen_port = 0
vault_path = "{root}/vault"
gemini_summary_model = "fold-model"
gemini_base_url = "http://127.0.0.1:{gemini_port}/"
[keys]
gemini_api_key = "test-key"
"#,
        root = tmp.path().display()
    ))
    .unwrap();
    let running = server::start(cfg, &tmp.path().join("state")).await.unwrap();

    let http = reqwest::Client::new();
    let base = format!("http://{}", running.addr);
    let code: Value = http
        .post(format!("{base}/api/v1/admin/pair/new"))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let paired: Value = http
        .post(format!("{base}/api/v1/pair"))
        .json(&json!({"code": code["code"], "name": "test"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    Env {
        _tmp: tmp,
        running,
        gemini,
        token: paired["device_token"].as_str().unwrap().to_string(),
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

    fn calls(&self) -> usize {
        self.gemini.lock().unwrap().calls.len()
    }

    /// The text the fake Gemini got as the user turn of call `n`.
    fn asked(&self, n: usize) -> String {
        let g = self.gemini.lock().unwrap();
        g.calls[n].2["contents"][0]["parts"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn answer(&self, answer: Result<&str, u16>) {
        self.gemini.lock().unwrap().answer = answer.map(str::to_string);
    }

    /// Give a wrongly started compact time to reach the fake, then count.
    async fn calls_after_a_moment(&self) -> usize {
        tokio::time::sleep(Duration::from_millis(300)).await;
        self.calls()
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

/// The next `threads.memory` push.
async fn next_memory(ws: &mut Ws) -> Value {
    loop {
        let frame = next(ws).await;
        if frame["event"] == "threads.memory" {
            assert!(frame.get("ref_id").is_none());
            return frame["payload"].clone();
        }
    }
}

async fn new_thread(ws: &mut Ws) -> String {
    let made = request(ws, "threads.create", json!({})).await.unwrap();
    made["thread"]["id"].as_str().unwrap().to_string()
}

/// Turns `from..=to`: entries `u<n>` (user) and `m<n>` (model).
fn turns(from: usize, to: usize) -> Vec<Value> {
    (from..=to)
        .flat_map(|n| {
            [
                json!({"id": format!("u{n}"), "ts": n, "kind": "user", "text": format!("question {n}")}),
                json!({"id": format!("m{n}"), "ts": n, "kind": "model", "text": format!("answer {n}")}),
            ]
        })
        .collect()
}

async fn append(ws: &mut Ws, thread: &str, entries: Vec<Value>) {
    request(
        ws,
        "threads.append",
        json!({"id": thread, "entries": entries}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn turn_trigger_folds_all_but_the_tail() {
    let env = boot().await;
    let mut ws = env.ws().await;
    let mut watcher = env.ws().await;
    let thread = new_thread(&mut ws).await;

    append(&mut ws, &thread, turns(1, 15)).await;
    assert_eq!(env.calls_after_a_moment().await, 0, "15 turns: no compact");
    let got = request(&mut ws, "threads.get", json!({"id": thread}))
        .await
        .unwrap();
    assert!(got.get("memory").is_none());
    assert_eq!(got["usage"], json!({"input": 0, "output": 0}));

    append(&mut ws, &thread, turns(16, 16)).await;
    let push = next_memory(&mut watcher).await;
    assert_eq!(push["thread_id"], json!(thread));
    assert_eq!(push["memory"]["text"], "## State\n- first");
    // 32 counted entries, the last 4 (u15 m15 u16 m16) stay.
    assert_eq!(push["memory"]["upto_id"], "m14");
    assert!(push["memory"]["at"].as_i64().unwrap() > 1_600_000_000_000);
    assert_eq!(push["usage"], json!({"input": 100, "output": 30}));
    assert_eq!(env.calls_after_a_moment().await, 1);

    {
        let g = env.gemini.lock().unwrap();
        let (path, key, body) = &g.calls[0];
        assert_eq!(path, "/v1beta/models/fold-model:generateContent");
        assert_eq!(key, "test-key");
        assert!(body["systemInstruction"]["parts"][0]["text"]
            .as_str()
            .unwrap()
            .contains("## State"));
    }
    let asked = env.asked(0);
    assert!(asked.starts_with("<memory>\n\n</memory>"), "{asked}");
    assert!(asked.contains("User: question 1\nAssistant: answer 1\n"));
    assert!(asked.contains("Assistant: answer 14") && !asked.contains("question 15"));

    // Entries are untouched; the memory rides along on threads.get.
    let got = request(&mut ws, "threads.get", json!({"id": thread}))
        .await
        .unwrap();
    assert_eq!(got["entries"].as_array().unwrap().len(), 32);
    assert_eq!(got["memory"], push["memory"]);
    assert_eq!(got["usage"], push["usage"]);

    // Next compact: the old memory goes in, an oversized answer is cut.
    env.answer(Ok(&"- bullet line\n".repeat(400)));
    append(&mut ws, &thread, turns(17, 18)).await;
    let done = request(&mut ws, "threads.compact", json!({"id": thread}))
        .await
        .unwrap();
    let asked = env.asked(1);
    assert!(
        asked.starts_with("<memory>\n## State\n- first\n</memory>"),
        "{asked}"
    );
    assert!(asked.contains("User: question 15") && !asked.contains("answer 14"));
    assert!(!asked.contains("question 17"), "tail of 4 stays out");
    let text = done["memory"]["text"].as_str().unwrap();
    assert!(text.len() <= 1500 && text.ends_with("- bullet line"));
    assert_eq!(done["memory"]["upto_id"], "m16");
    assert_eq!(done["usage"], json!({"input": 200, "output": 60}));
    assert_eq!(next_memory(&mut watcher).await["memory"], done["memory"]);

    // Nothing new to fold: the current memory comes back, no call.
    let again = request(&mut ws, "threads.compact", json!({"id": thread}))
        .await
        .unwrap();
    assert_eq!(again, done);
    assert_eq!(env.calls(), 2);
}

#[tokio::test]
async fn voice_stop_triggers_and_a_pending_tool_stops_the_fold() {
    let env = boot().await;
    let mut ws = env.ws().await;
    let mut watcher = env.ws().await;
    let thread = new_thread(&mut ws).await;

    // No user turn yet: a voice stop folds nothing.
    append(
        &mut ws,
        &thread,
        vec![json!({"id": "k0", "ts": 0, "kind": "marker", "text": "voice_ended"})],
    )
    .await;
    assert_eq!(env.calls_after_a_moment().await, 0);

    let mut entries = turns(1, 2);
    entries.push(json!({"id": "t1", "ts": 3, "kind": "tool", "text": "",
        "tool": {"name": "read_file", "args": {"path": "note.md"}, "called_at": 3}}));
    entries.extend(turns(3, 5));
    entries.push(json!({"id": "k1", "ts": 9, "kind": "marker", "text": "voice_ended"}));
    append(&mut ws, &thread, entries).await;
    let push = next_memory(&mut watcher).await;
    assert_eq!(
        push["memory"]["upto_id"], "m2",
        "stops before the running tool"
    );
    assert!(!env.asked(0).contains("read_file"));

    // The result arrives: the tool call is folded as one clipped line.
    append(
        &mut ws,
        &thread,
        vec![json!({"id": "t1", "ts": 3, "kind": "tool", "text": "",
        "tool": {"name": "read_file", "args": {"path": "note.md"}, "called_at": 3,
                 "result": {"content": "x".repeat(4000)}, "result_at": 4}})],
    )
    .await;
    let done = request(&mut ws, "threads.compact", json!({"id": thread}))
        .await
        .unwrap();
    assert_eq!(done["memory"]["upto_id"], "m3");
    let asked = env.asked(1);
    assert!(asked.contains(r#"[tool read_file({"path":"note.md"}) -> {"content":"xxx"#));
    assert!(asked.len() < 1000, "tool result clipped: {}", asked.len());
}

#[tokio::test]
async fn failure_keeps_the_memory_and_backs_off() {
    let env = boot().await;
    let mut ws = env.ws().await;
    let thread = new_thread(&mut ws).await;
    append(&mut ws, &thread, turns(1, 6)).await;
    let first = request(&mut ws, "threads.compact", json!({"id": thread}))
        .await
        .unwrap();
    assert_eq!(first["memory"]["upto_id"], "m4");

    append(&mut ws, &thread, turns(7, 10)).await;
    for (answer, expect) in [(Err(500), "500"), (Ok(" \n"), "no text")] {
        env.answer(answer);
        let e = request(&mut ws, "threads.compact", json!({"id": thread}))
            .await
            .unwrap_err();
        assert!(e.contains(expect) && !e.contains("test-key"), "{e}");
    }
    assert_eq!(env.calls(), 3);
    let got = request(&mut ws, "threads.get", json!({"id": thread}))
        .await
        .unwrap();
    assert_eq!(got["memory"], first["memory"], "old memory stays");
    assert_eq!(got["usage"], first["usage"]);
    assert_eq!(got["entries"].as_array().unwrap().len(), 20);

    // Automatic triggers leave the thread alone right after a failure.
    env.answer(Ok("## State\n- second"));
    let mut entries = turns(11, 30);
    entries.push(json!({"id": "k1", "ts": 99, "kind": "marker", "text": "voice_ended"}));
    append(&mut ws, &thread, entries).await;
    assert_eq!(env.calls_after_a_moment().await, 3);

    // The button always tries.
    let done = request(&mut ws, "threads.compact", json!({"id": thread}))
        .await
        .unwrap();
    assert_eq!(done["memory"]["text"], "## State\n- second");
    assert_eq!(done["memory"]["upto_id"], "m28");
    assert_eq!(env.calls(), 4);

    let e = request(&mut ws, "threads.compact", json!({"id": "nope"}))
        .await
        .unwrap_err();
    assert!(e.contains("unknown thread"), "{e}");
    assert!(request(&mut ws, "threads.compact", json!({}))
        .await
        .is_err());
}

#[tokio::test]
async fn concurrent_compacts_make_one_call() {
    let env = boot().await;
    let mut a = env.ws().await;
    let mut b = env.ws().await;
    let thread = new_thread(&mut a).await;
    append(&mut a, &thread, turns(1, 6)).await;
    env.gemini.lock().unwrap().delay = Duration::from_millis(300);
    let (ra, rb) = tokio::join!(
        request(&mut a, "threads.compact", json!({"id": thread})),
        request(&mut b, "threads.compact", json!({"id": thread})),
    );
    assert_eq!(
        ra.unwrap(),
        rb.unwrap(),
        "the second waits, then finds it done"
    );
    assert_eq!(env.calls(), 1);
}

#[tokio::test]
async fn usage_adds_up() {
    let env = boot().await;
    let mut ws = env.ws().await;
    let thread = new_thread(&mut ws).await;
    let usage = |input: i64, output: i64| json!({"id": thread, "input": input, "output": output});
    let got = request(&mut ws, "threads.usage", usage(10, 2)).await;
    assert_eq!(got.unwrap(), json!({"usage": {"input": 10, "output": 2}}));
    let got = request(&mut ws, "threads.usage", usage(5, 0)).await;
    assert_eq!(got.unwrap(), json!({"usage": {"input": 15, "output": 2}}));

    for bad in [
        usage(-1, 0),
        json!({"id": thread, "input": 1}),
        json!({"id": thread, "input": "1", "output": 1}),
        json!({"id": "nope", "input": 1, "output": 1}),
    ] {
        assert!(
            request(&mut ws, "threads.usage", bad.clone())
                .await
                .is_err(),
            "{bad}"
        );
    }
    let got = request(&mut ws, "threads.get", json!({"id": thread}))
        .await
        .unwrap();
    assert_eq!(got["usage"], json!({"input": 15, "output": 2}));
}
