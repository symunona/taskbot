//! HTTP + WebSocket on one port, as laid out in docs/protocol.md.

use crate::agents::{self, Session};
use crate::compact::{self, Compactor};
use crate::config::Config;
use crate::state::{self, Device, Store};
use crate::tasks::{self, Tasks};
use crate::vault::Vault;
use crate::{iso, memory, now_ms, now_secs, tools};
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path as UrlPath, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const AUTH_DEADLINE: Duration = Duration::from_secs(10);
const PING_EVERY: Duration = Duration::from_secs(30);
const PAIR_MAX_FAILURES: usize = 5;
const PAIR_WINDOW: Duration = Duration::from_secs(60);
const MAX_WS_MESSAGE: usize = 1024 * 1024;

/// What every authenticated socket hears about.
#[derive(Clone, Debug)]
pub enum Push {
    /// A ready-to-send frame (`agent.say`, `agent.event`, `threads.changed`, `threads.memory`,
    /// `memory.always`, `memory.project`, `tasks.changed`).
    Frame(String),
    /// This device was revoked: its sockets must go.
    Revoked(String),
}

pub struct App {
    pub cfg: Config,
    pub vault: Vault,
    pub store: Store,
    pub tasks: Tasks,
    pub compactor: Compactor,
    /// The port actually bound (differs from the config when that says 0).
    pub port: u16,
    pub http: reqwest::Client,
    hub: broadcast::Sender<Push>,
    limiter: Mutex<RateLimiter>,
    project_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl App {
    pub fn server_urls(&self) -> Vec<String> {
        crate::net::server_urls(&self.cfg.public_urls, self.cfg.listen_ip(), self.port)
    }

    pub fn push(&self, event: &str, payload: Value) {
        // No receivers just means no phone is connected.
        let _ = self.hub.send(Push::Frame(envelope(event, None, payload)));
    }

    pub fn push_event(&self, project: &str, kind: &str, detail: Value) {
        self.push(
            "agent.event",
            json!({"project": project, "kind": kind, "detail": detail}),
        );
    }

    pub fn project_lock(&self, project: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.project_locks.lock().unwrap_or_else(|e| e.into_inner());
        locks.entry(project.to_string()).or_default().clone()
    }

    pub async fn sessions(self: &Arc<Self>) -> Vec<Session> {
        let app = self.clone();
        tokio::task::spawn_blocking(move || agents::sessions(&app.cfg))
            .await
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------- start-up

pub struct Running {
    pub addr: SocketAddr,
    pub app: Arc<App>,
    tasks: Vec<tokio::task::JoinHandle<std::io::Result<()>>>,
}

impl Running {
    /// Serve until a listener fails.
    pub async fn wait(self) -> Result<(), String> {
        let (res, _, _) = futures_util::future::select_all(self.tasks).await;
        match res {
            Ok(Ok(())) => Err("listener stopped".into()),
            Ok(Err(e)) => Err(format!("listener failed: {e}")),
            Err(e) => Err(format!("listener task died: {e}")),
        }
    }
}

/// Open the state db, bind, start serving. `listen_port = 0` picks a free port.
pub async fn start(cfg: Config, state_dir: &Path) -> Result<Running, String> {
    let vault = Vault::new(&cfg.vault_path)?;
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(state_dir)
        .map_err(|e| format!("state dir {}: {e}", state_dir.display()))?;
    let state_dir = &state_dir
        .canonicalize()
        .map_err(|e| format!("state dir {}: {e}", state_dir.display()))?;
    let db_path = state_dir.join("taltosbot.db");
    if db_path.starts_with(vault.root()) {
        return Err(format!(
            "state dir {} is inside the vault; move it out",
            state_dir.display()
        ));
    }
    let store =
        Store::open(&db_path).map_err(|e| format!("state db {}: {e}", db_path.display()))?;
    let task_store = Tasks::open(&db_path, now_ms())
        .map_err(|e| format!("state db {}: {e}", db_path.display()))?;

    let ip = cfg.listen_ip();
    let bind = |addr: SocketAddr| async move {
        tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| format!("cannot listen on {addr}: {e}"))
    };
    let main = bind(SocketAddr::new(ip, cfg.listen_port)).await?;
    let addr = main.local_addr().map_err(|e| e.to_string())?;
    let mut listeners = vec![main];
    // The CLI and the channel shims talk to 127.0.0.1, whatever the bind address.
    if !ip.is_unspecified() && !ip.is_loopback() {
        listeners.push(bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), addr.port())).await?);
    }

    let app = Arc::new(App {
        cfg,
        vault,
        store,
        tasks: task_store,
        compactor: Compactor::default(),
        port: addr.port(),
        http: reqwest::Client::new(),
        hub: broadcast::channel(256).0,
        limiter: Mutex::new(RateLimiter::default()),
        project_locks: Mutex::new(HashMap::new()),
    });
    tasks::watch(&app);
    let tasks = listeners
        .into_iter()
        .map(|l| {
            let service = router(app.clone()).into_make_service_with_connect_info::<SocketAddr>();
            tokio::spawn(async move { axum::serve(l, service).await })
        })
        .collect();
    Ok(Running { addr, app, tasks })
}

pub fn router(app: Arc<App>) -> Router {
    use tower_http::cors::{Any, CorsLayer};
    // CORS `*` is for the web app. Loopback routes stay without it, so no web
    // page gets to read their answers.
    let public = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/pair", post(pair))
        .route("/api/v1/me", get(me))
        .route("/api/v1/config", get(client_config))
        .route("/api/v1/ws", get(ws_upgrade))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );
    let local = Router::new()
        .route("/api/v1/admin/pair/new", post(admin_pair_new))
        .route("/api/v1/admin/devices", get(admin_devices))
        .route("/api/v1/admin/devices/{id}", delete(admin_revoke))
        .route("/internal/agent/say", post(agent_say))
        .route("/internal/agent/event", post(agent_event))
        .layer(middleware::from_fn(loopback_only));
    public
        .merge(local)
        .fallback(|| async { err(StatusCode::NOT_FOUND, "not found") })
        .with_state(app)
}

// ------------------------------------------------------------------ guards

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

fn internal(e: rusqlite::Error) -> Response {
    tracing::error!("state db: {e}");
    err(StatusCode::INTERNAL_SERVER_ERROR, "state db error")
}

/// A refusal that has not been turned into a response yet.
struct Fail(StatusCode, String);

impl IntoResponse for Fail {
    fn into_response(self) -> Response {
        err(self.0, self.1)
    }
}

fn parse_body<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, Fail> {
    serde_json::from_slice(body)
        .map_err(|e| Fail(StatusCode::BAD_REQUEST, format!("bad request body: {e}")))
}

/// True only for a request made directly from this machine.
///
/// The decision starts from the real TCP peer. A loopback peer is still
/// refused when the request shows signs of having been relayed (a reverse
/// proxy or `tailscale serve` on this host), of coming from a web page
/// (`Origin`), or of DNS rebinding (a `Host` that is not a loopback name).
pub fn is_local_request(peer: SocketAddr, headers: &HeaderMap) -> bool {
    if !peer.ip().to_canonical().is_loopback() {
        return false;
    }
    const RELAY_MARKS: [&str; 5] = [
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-real-ip",
        "origin",
    ];
    if RELAY_MARKS.iter().any(|h| headers.contains_key(*h)) {
        return false;
    }
    match headers.get(header::HOST).map(|h| h.to_str()) {
        None => true,
        Some(Err(_)) => false,
        Some(Ok(host)) => {
            let name = match host.strip_prefix('[') {
                Some(v6) => v6.split(']').next().unwrap_or(""),
                None => host.rsplit_once(':').map_or(host, |(name, _port)| name),
            };
            name.eq_ignore_ascii_case("localhost")
                || name.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
        }
    }
}

async fn loopback_only(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    if is_local_request(peer, req.headers()) {
        next.run(req).await
    } else {
        err(StatusCode::FORBIDDEN, "loopback only")
    }
}

fn bearer(app: &App, headers: &HeaderMap) -> Result<Device, Fail> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .map(str::trim)
        .ok_or_else(|| Fail(StatusCode::UNAUTHORIZED, "missing bearer token".into()))?;
    match app.store.authenticate(token, now_secs()) {
        Ok(Some(d)) => Ok(d),
        Ok(None) => Err(Fail(StatusCode::UNAUTHORIZED, "invalid token".into())),
        Err(e) => {
            tracing::error!("state db: {e}");
            Err(Fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "state db error".into(),
            ))
        }
    }
}

/// Failed pairing attempts per remote address, sliding window.
#[derive(Default)]
pub struct RateLimiter {
    fails: HashMap<IpAddr, VecDeque<Instant>>,
}

impl RateLimiter {
    /// IPv6 clients own a whole /64: count them as one.
    fn key(ip: IpAddr) -> IpAddr {
        match ip.to_canonical() {
            IpAddr::V6(v6) => IpAddr::V6((u128::from(v6) & !0u128 << 64).into()),
            v4 => v4,
        }
    }

    pub fn blocked(&mut self, ip: IpAddr, now: Instant) -> bool {
        self.fails.retain(|_, q| {
            while q
                .front()
                .is_some_and(|t| now.duration_since(*t) >= PAIR_WINDOW)
            {
                q.pop_front();
            }
            !q.is_empty()
        });
        self.fails
            .get(&Self::key(ip))
            .is_some_and(|q| q.len() >= PAIR_MAX_FAILURES)
    }

    pub fn fail(&mut self, ip: IpAddr, now: Instant) {
        self.fails.entry(Self::key(ip)).or_default().push_back(now);
    }
}

// ---------------------------------------------------------------- handlers

async fn health() -> Json<Value> {
    Json(json!({"app": "taltosbot", "status": "ok", "version": VERSION}))
}

async fn admin_pair_new(State(app): State<Arc<App>>) -> Response {
    match app.store.new_pair_code(now_secs()) {
        Ok((code, expires_at)) => Json(
            json!({"code": code, "expires_at": iso(expires_at), "server_urls": app.server_urls()}),
        )
        .into_response(),
        Err(e) => internal(e),
    }
}

async fn admin_devices(State(app): State<Arc<App>>) -> Response {
    match app.store.devices() {
        Ok(devices) => {
            let devices: Vec<Value> = devices
                .into_iter()
                .map(|d| {
                    json!({"id": d.id, "name": d.name, "created_at": iso(d.created_at), "last_seen_at": d.last_seen_at.map(iso)})
                })
                .collect();
            Json(json!({ "devices": devices })).into_response()
        }
        Err(e) => internal(e),
    }
}

async fn admin_revoke(State(app): State<Arc<App>>, UrlPath(id): UrlPath<String>) -> Response {
    match app.store.revoke(&id) {
        Ok(true) => {
            let _ = app.hub.send(Push::Revoked(id));
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => err(StatusCode::NOT_FOUND, "no such device"),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct PairReq {
    code: String,
    #[serde(default)]
    name: String,
}

async fn pair(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    body: Bytes,
) -> Response {
    let req: PairReq = match parse_body(&body) {
        Ok(r) => r,
        Err(fail) => return fail.into_response(),
    };
    let code = req.code.trim().to_ascii_uppercase();
    let name: String = req
        .name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect();
    let name = if name.is_empty() {
        "device".to_string()
    } else {
        name
    };

    // Check, claim and count under one lock, so parallel guesses cannot slip
    // past the limit. Nothing in here awaits.
    let paired = {
        let mut limiter = app.limiter.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if limiter.blocked(peer.ip(), now) {
            return err(
                StatusCode::TOO_MANY_REQUESTS,
                "too many failed pairing attempts, wait a minute",
            );
        }
        match app.store.claim(&code, &name, now_secs()) {
            Ok(Some(p)) => p,
            Ok(None) => {
                limiter.fail(peer.ip(), now);
                return err(StatusCode::UNAUTHORIZED, "invalid or expired code");
            }
            Err(e) => return internal(e),
        }
    };
    tracing::info!(device = %paired.device_id, %name, peer = %peer.ip(), "device paired");
    Json(json!({"device_token": paired.device_token, "device_id": paired.device_id, "server_urls": app.server_urls()}))
        .into_response()
}

async fn me(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    match bearer(&app, &headers) {
        Ok(d) => Json(json!({"device_id": d.id, "name": d.name, "server_version": VERSION}))
            .into_response(),
        Err(fail) => fail.into_response(),
    }
}

/// Only the three values the app needs. Nothing else from the config leaves the server.
async fn client_config(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if let Err(fail) = bearer(&app, &headers) {
        return fail.into_response();
    }
    match app.cfg.gemini_api_key() {
        Some(key) => Json(json!({
            "gemini_api_key": key,
            "gemini_model": app.cfg.gemini_model,
            "gemini_text_model": app.cfg.gemini_text_model,
        }))
        .into_response(),
        None => err(
            StatusCode::SERVICE_UNAVAILABLE,
            "gemini_api_key is not set on the server",
        ),
    }
}

#[derive(Deserialize)]
struct SayReq {
    project: String,
    text: String,
    /// Tasks of the agent's turn. Absent: a notice, or a shim without task ids.
    tasks: Option<Vec<String>>,
}

async fn agent_say(State(app): State<Arc<App>>, body: Bytes) -> Response {
    let req: SayReq = match parse_body(&body) {
        Ok(r) => r,
        Err(fail) => return fail.into_response(),
    };
    if let Some(ids) = &req.tasks {
        let changed = app.tasks.say(&req.project, ids, &req.text, now_ms());
        tasks::push(&app, changed);
    }
    app.push(
        "agent.say",
        json!({"project": req.project, "text": req.text}),
    );
    Json(json!({"ok": true})).into_response()
}

#[derive(Deserialize)]
struct EventReq {
    project: String,
    kind: String,
    #[serde(default)]
    detail: Value,
    tasks: Option<Vec<String>>,
}

async fn agent_event(State(app): State<Arc<App>>, body: Bytes) -> Response {
    let req: EventReq = match parse_body(&body) {
        Ok(r) => r,
        Err(fail) => return fail.into_response(),
    };
    let changed = app.tasks.event(
        &req.project,
        req.tasks.as_deref(),
        &req.kind,
        req.detail.as_str().unwrap_or(""),
        now_ms(),
    );
    app.push_event(&req.project, &req.kind, req.detail);
    tasks::push(&app, changed);
    Json(json!({"ok": true})).into_response()
}

// --------------------------------------------------------------- websocket

fn envelope(event: &str, ref_id: Option<&Value>, payload: Value) -> String {
    let mut m = json!({
        "event": event,
        "id": hex::encode(crate::random_bytes::<8>()),
        "ts": chrono::Utc::now().timestamp_millis(),
        "payload": payload,
    });
    if let Some(r) = ref_id {
        m["ref_id"] = r.clone();
    }
    m.to_string()
}

async fn ws_upgrade(State(app): State<Arc<App>>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(MAX_WS_MESSAGE)
        .on_upgrade(move |socket| socket_loop(app, socket))
}

/// Wait for the auth frame. `Err` carries the `auth.error` reason.
async fn wait_auth(app: &App, socket: &mut WebSocket) -> Result<Device, &'static str> {
    loop {
        match socket.recv().await {
            Some(Ok(Message::Text(text))) => {
                let frame: Value = serde_json::from_str(&text).map_err(|_| "auth_required")?;
                if frame["event"] != "auth" {
                    return Err("auth_required");
                }
                let token = frame["payload"]["token"].as_str().ok_or("invalid_token")?;
                return match app.store.authenticate(token, now_secs()) {
                    Ok(Some(d)) => Ok(d),
                    Ok(None) => Err("invalid_token"),
                    Err(e) => {
                        tracing::error!("state db: {e}");
                        Err("server_error")
                    }
                };
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            _ => return Err("auth_required"),
        }
    }
}

async fn socket_loop(app: Arc<App>, mut socket: WebSocket) {
    // One attempt, one deadline: a wrong first frame ends the connection.
    let auth = tokio::time::timeout(AUTH_DEADLINE, wait_auth(&app, &mut socket)).await;
    let device = match auth.unwrap_or(Err("timeout")) {
        Ok(d) => d,
        Err(reason) => {
            let frame = json!({"event": "auth.error", "payload": {"reason": reason}}).to_string();
            let _ = socket.send(Message::Text(frame.into())).await;
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    // Subscribe before confirming, so nothing pushed after `auth.ok` is missed.
    let mut hub = app.hub.subscribe();
    let ok = json!({"event": "auth.ok", "payload": {"device_id": device.id}}).to_string();
    if socket.send(Message::Text(ok.into())).await.is_err() {
        return;
    }

    let (mut sink, mut stream) = socket.split();
    let (out, mut out_rx) = mpsc::channel::<Message>(64);
    let writer = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if sink.send(msg).await.is_err() {
                return;
            }
        }
        let _ = sink.send(Message::Close(None)).await;
    });

    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    loop {
        let outgoing = tokio::select! {
            frame = stream.next() => match frame {
                Some(Ok(Message::Text(text))) => {
                    // Handlers run off the socket loop; replies come back through `out`.
                    let (app, out) = (app.clone(), out.clone());
                    tokio::spawn(async move {
                        let reply = handle_frame(&app, &text).await;
                        let _ = out.send(Message::Text(reply.into())).await;
                    });
                    continue;
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => continue,
            },
            push = hub.recv() => match push {
                Ok(Push::Frame(frame)) => Message::Text(frame.into()),
                Ok(Push::Revoked(id)) if id == device.id => break,
                Ok(Push::Revoked(_)) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = ping.tick() => Message::Ping(Bytes::new()),
        };
        if out.send(outgoing).await.is_err() {
            break;
        }
    }
    drop(out);
    // Replies still being computed keep `out` open for a moment; do not wait for them.
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
}

async fn handle_frame(app: &Arc<App>, text: &str) -> String {
    let Ok(frame) = serde_json::from_str::<Value>(text) else {
        return envelope("error", None, json!({"error": "frame is not JSON"}));
    };
    let ref_id = frame.get("id").filter(|v| !v.is_null());
    let payload = &frame["payload"];
    match frame["event"].as_str() {
        Some("tools.list") => envelope(
            "tools.list.result",
            ref_id,
            json!({"tools": tools::declarations()}),
        ),
        Some("tool.call") => match payload["name"].as_str() {
            Some(name) => {
                let thread_id = payload["thread_id"].as_str();
                let result = tools::call(app, name, &payload["args"], thread_id).await;
                envelope("tool.result", ref_id, json!({ "result": result }))
            }
            None => envelope(
                "error",
                ref_id,
                json!({"error": "tool.call needs payload.name"}),
            ),
        },
        Some("sessions.list") => envelope(
            "sessions.list.result",
            ref_id,
            json!({"sessions": app.sessions().await}),
        ),
        Some("tasks.list") => envelope(
            "tasks.list.result",
            ref_id,
            json!({"tasks": app.tasks.list(now_ms())}),
        ),
        Some(
            event @ ("threads.list"
            | "threads.create"
            | "threads.get"
            | "threads.rename"
            | "threads.append"
            | "threads.usage"
            | "threads.set_project"),
        ) => match threads(app, event, payload) {
            Ok(result) => {
                if event == "threads.append" {
                    if let Some(id) = payload["id"].as_str() {
                        compact::after_append(app, id, has_voice_ended(&payload["entries"]));
                    }
                }
                envelope(&format!("{event}.result"), ref_id, result)
            }
            Err(e) => envelope("error", ref_id, json!({ "error": e })),
        },
        Some("threads.compact") => match payload["id"].as_str() {
            Some(id) => match compact::run(app, id, compact::Why::Request).await {
                Ok(result) => envelope("threads.compact.result", ref_id, result),
                Err(e) => envelope("error", ref_id, json!({ "error": e })),
            },
            None => envelope(
                "error",
                ref_id,
                json!({"error": "threads.compact needs payload.id"}),
            ),
        },
        Some(other) => envelope(
            "error",
            ref_id,
            json!({"error": format!("unknown event {other:?}")}),
        ),
        None => envelope("error", ref_id, json!({"error": "frame has no event"})),
    }
}

// ----------------------------------------------------------------- threads

fn db_error(e: rusqlite::Error) -> String {
    tracing::error!("state db: {e}");
    "state db error".into()
}

fn no_thread(id: &str) -> String {
    format!("unknown thread {id:?}")
}

/// Rename a thread and tell every socket. Shared by `threads.rename` and the
/// `rename_thread` tool. Returns `{thread}`.
pub fn rename_thread(app: &App, id: &str, name: &str) -> Result<Value, String> {
    let name = state::thread_name(name)?;
    let thread = app
        .store
        .rename_thread(id, &name)
        .map_err(db_error)?
        .ok_or_else(|| no_thread(id))?;
    let payload = json!({ "thread": thread });
    app.push("threads.changed", payload.clone());
    Ok(payload)
}

/// A `threads.append` batch that carries the voice stop marker.
fn has_voice_ended(entries: &Value) -> bool {
    entries
        .as_array()
        .into_iter()
        .flatten()
        .any(|e| e["kind"] == "marker" && e["text"] == "voice_ended")
}

/// The `threads.*` requests. `Ok` = payload of `<event>.result`.
fn threads(app: &App, event: &str, payload: &Value) -> Result<Value, String> {
    let id = || {
        payload["id"]
            .as_str()
            .ok_or_else(|| format!("{event} needs payload.id"))
    };
    match event {
        "threads.list" => {
            let threads = app.store.threads().map_err(db_error)?;
            Ok(json!({ "threads": threads }))
        }
        "threads.create" => {
            let name = match &payload["name"] {
                Value::Null => state::THREAD_NAME_DEFAULT.to_string(),
                Value::String(raw) => state::thread_name(raw)?,
                _ => return Err("threads.create: name must be a string".into()),
            };
            let thread = app.store.create_thread(&name, now_ms()).map_err(db_error)?;
            let mut payload = json!({ "thread": thread });
            app.push("threads.changed", payload.clone());
            payload["always"] = memory::always_pins(app)?;
            Ok(payload)
        }
        "threads.get" => {
            let id = id()?;
            let (thread, entries) = app
                .store
                .thread(id, now_ms())
                .map_err(db_error)?
                .ok_or_else(|| no_thread(id))?;
            let mut reply = json!({"thread": thread, "entries": entries});
            if let Some((memory, usage)) = app.store.thread_memory(id).map_err(db_error)? {
                let extra = compact::payload(memory, usage);
                reply["usage"] = extra["usage"].clone();
                if let Some(memory) = extra.get("memory") {
                    reply["memory"] = memory.clone();
                }
            }
            reply["pins"] = memory::thread_pins(app, id)?;
            reply["always"] = memory::always_pins(app)?;
            // Read fresh: hand edits and agent edits of the file count at once.
            let project_memory = thread
                .project
                .as_deref()
                .and_then(|project| memory::project_memory(app, project));
            if let Some(project_memory) = project_memory {
                reply["project_memory"] = project_memory;
            }
            Ok(reply)
        }
        "threads.rename" => {
            let name = payload["name"]
                .as_str()
                .ok_or("threads.rename needs payload.name")?;
            rename_thread(app, id()?, name)
        }
        "threads.set_project" => {
            let project = payload["project"]
                .as_str()
                .ok_or("threads.set_project needs payload.project")?;
            memory::set_project(app, id()?, project)
        }
        "threads.append" => {
            let id = id()?;
            let raw = payload["entries"]
                .as_array()
                .ok_or("threads.append needs payload.entries")?;
            if raw.len() > state::APPEND_MAX {
                return Err(format!(
                    "threads.append: {} entries, max {}",
                    raw.len(),
                    state::APPEND_MAX
                ));
            }
            let entries = raw
                .iter()
                .map(state::Entry::parse)
                .collect::<Result<Vec<_>, _>>()?;
            let count = app
                .store
                .append_entries(id, &entries, now_ms())
                .map_err(db_error)?
                .ok_or_else(|| no_thread(id))?;
            Ok(json!({ "count": count }))
        }
        "threads.usage" => {
            let id = id()?;
            let count = |field: &str| {
                payload[field]
                    .as_i64()
                    .filter(|n| *n >= 0)
                    .ok_or_else(|| format!("threads.usage: {field} must be an integer >= 0"))
            };
            let used = state::Usage {
                input: count("input")?,
                output: count("output")?,
            };
            let usage = app
                .store
                .add_usage(id, used)
                .map_err(db_error)?
                .ok_or_else(|| no_thread(id))?;
            Ok(json!({ "usage": usage }))
        }
        other => Err(format!("unknown event {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        pairs
            .iter()
            .map(|(k, v)| (k.parse::<header::HeaderName>().unwrap(), v.parse().unwrap()))
            .collect()
    }

    #[test]
    fn loopback_guard() {
        let local: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let mapped: SocketAddr = "[::ffff:127.0.0.1]:5000".parse().unwrap();
        let v6: SocketAddr = "[::1]:5000".parse().unwrap();
        let lan: SocketAddr = "192.168.1.9:5000".parse().unwrap();
        let tailnet: SocketAddr = "100.81.125.107:5000".parse().unwrap();

        let plain = headers(&[("host", "127.0.0.1:55566")]);
        assert!(is_local_request(local, &plain));
        assert!(is_local_request(mapped, &plain));
        assert!(is_local_request(v6, &headers(&[("host", "[::1]:55566")])));
        assert!(is_local_request(
            local,
            &headers(&[("host", "localhost:55566")])
        ));
        assert!(is_local_request(local, &headers(&[])));

        assert!(!is_local_request(lan, &plain));
        assert!(!is_local_request(tailnet, &plain));
        // A header cannot make a remote peer local.
        assert!(!is_local_request(
            lan,
            &headers(&[("host", "127.0.0.1"), ("x-forwarded-for", "127.0.0.1")])
        ));
        // Relayed by a proxy on this host, a web page, DNS rebinding.
        assert!(!is_local_request(
            local,
            &headers(&[("host", "127.0.0.1"), ("x-forwarded-for", "8.8.8.8")])
        ));
        assert!(!is_local_request(
            local,
            &headers(&[("host", "127.0.0.1"), ("origin", "https://evil.example")])
        ));
        assert!(!is_local_request(
            local,
            &headers(&[("host", "evil.example:55566")])
        ));
        assert!(!is_local_request(
            local,
            &headers(&[("host", "taltos.example.org")])
        ));
    }

    #[test]
    fn rate_limiter_window() {
        let mut rl = RateLimiter::default();
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let b: IpAddr = "10.0.0.2".parse().unwrap();
        let t0 = Instant::now();
        for i in 0..5 {
            assert!(!rl.blocked(a, t0), "attempt {i} allowed");
            rl.fail(a, t0);
        }
        assert!(rl.blocked(a, t0 + Duration::from_secs(59)));
        assert!(!rl.blocked(b, t0), "other addresses unaffected");
        assert!(
            !rl.blocked(a, t0 + Duration::from_secs(60)),
            "window slides"
        );
        assert!(rl.fails.is_empty(), "stale entries are dropped");

        let v6a: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let v6b: IpAddr = "2001:db8:1:2:ffff::9".parse().unwrap();
        for _ in 0..5 {
            rl.fail(v6a, t0);
        }
        assert!(rl.blocked(v6b, t0), "same /64 shares a bucket");
    }
}
