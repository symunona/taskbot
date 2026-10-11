use base64::{engine::general_purpose::STANDARD, Engine as _};
use clap::{Parser, Subcommand};
use serde::Serialize;
use serde_json::Value;
use std::process::ExitCode;
use std::time::Duration;
use taltosbot::config::{self, Config, LoadError};
use taltosbot::state::Store;
use taltosbot::{agents, iso, net, server};

#[derive(Parser)]
#[command(
    name = "taltosbot",
    version,
    about = "Taltosbot server: pairing, read-only vault tools, agent sessions"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server
    Serve,
    /// Mint a pairing code on the running server and show it as text and QR
    Pair,
    /// Paired devices
    Devices {
        #[command(subcommand)]
        command: DeviceCommand,
    },
    /// Show config, state and whether the server is up
    Status,
}

#[derive(Subcommand)]
enum DeviceCommand {
    List,
    Revoke { id: String },
}

/// The pairing QR payload, field order as in docs/protocol.md.
#[derive(Serialize)]
struct ConnectString<'a> {
    v: u8,
    code: &'a str,
    urls: &'a [String],
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let path = config::config_path();
    let cfg = match Config::load(&path) {
        Ok(cfg) => cfg,
        Err(LoadError::Created(p)) => {
            eprintln!(
                "No config found. Wrote a commented default to {}",
                p.display()
            );
            eprintln!("Set vault_path and [keys].gemini_api_key there, then run this again.");
            return ExitCode::from(2);
        }
        Err(LoadError::Invalid(msg)) => {
            eprintln!("config error: {msg}");
            return ExitCode::from(2);
        }
    };
    let result = match cli.command {
        Command::Serve => serve(cfg).await,
        Command::Pair => pair(&cfg).await,
        Command::Devices {
            command: DeviceCommand::List,
        } => devices_list(&cfg).await,
        Command::Devices {
            command: DeviceCommand::Revoke { id },
        } => devices_revoke(&cfg, &id).await,
        Command::Status => status(&cfg, &path).await,
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

async fn serve(cfg: Config) -> Result<(), String> {
    use std::io::IsTerminal;
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    let state_dir = config::state_dir();
    let running = server::start(cfg, &state_dir).await?;
    let app = &running.app;
    tracing::info!(
        "taltosbot {} listening on {}",
        server::VERSION,
        running.addr
    );
    tracing::info!(
        "vault {} (read-only), state {}",
        app.vault.root().display(),
        state_dir.display()
    );
    tracing::info!("reachable at {:?}", app.server_urls());
    if app.cfg.gemini_api_key().is_none() {
        tracing::warn!("no gemini_api_key set: GET /api/v1/config will answer 503");
    }
    if app.cfg.channel_dir.is_none() {
        tracing::warn!("no channel_dir set: the delegate tool will return an error");
    }
    running.wait().await
}

// ------------------------------------------------- client side of the admin API

fn base_url(cfg: &Config) -> String {
    match cfg.listen_ip() {
        std::net::IpAddr::V6(ip) if ip.is_loopback() => format!("http://[::1]:{}", cfg.listen_port),
        _ => format!("http://127.0.0.1:{}", cfg.listen_port),
    }
}

enum CallError {
    /// Nothing is listening: the server is not running.
    Down(String),
    Failed(String),
}

impl From<CallError> for String {
    fn from(e: CallError) -> String {
        match e {
            CallError::Down(m) | CallError::Failed(m) => m,
        }
    }
}

/// One request to the local server. `Ok(None)` is a body-less success (204).
async fn call(
    cfg: &Config,
    method: reqwest::Method,
    path: &str,
) -> Result<Option<Value>, CallError> {
    let url = format!("{}{path}", base_url(cfg));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .no_proxy()
        .build()
        .expect("http client");
    let mut req = client.request(method.clone(), &url);
    if method == reqwest::Method::POST {
        req = req.json(&serde_json::json!({}));
    }
    let resp = req.send().await.map_err(|e| {
        let msg = format!(
            "no taltosbot server answering at {} ({e}); start it with `taltosbot serve`",
            base_url(cfg)
        );
        if e.is_connect() {
            CallError::Down(msg)
        } else {
            CallError::Failed(msg)
        }
    })?;
    let status = resp.status();
    let body: Option<Value> = resp.json().await.ok();
    if status.is_success() {
        return Ok(body);
    }
    let detail = body
        .as_ref()
        .and_then(|b| b["error"].as_str())
        .unwrap_or("no detail");
    Err(CallError::Failed(format!("{url}: {status}: {detail}")))
}

async fn pair(cfg: &Config) -> Result<(), String> {
    let resp = call(cfg, reqwest::Method::POST, "/api/v1/admin/pair/new")
        .await?
        .ok_or("empty answer")?;
    let code = resp["code"].as_str().ok_or("answer has no code")?;
    let urls: Vec<String> =
        serde_json::from_value(resp["server_urls"].clone()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string(&ConnectString {
        v: 1,
        code,
        urls: &urls,
    })
    .map_err(|e| e.to_string())?;
    let connect = STANDARD.encode(json);

    if urls.is_empty() {
        eprintln!("warning: no tailscale or LAN address found; set public_urls in the config or the phone has nothing to connect to");
    }
    qr2term::print_qr(&connect).map_err(|e| format!("cannot draw QR: {e}"))?;
    println!();
    println!("code:    {code}");
    println!(
        "expires: {} (5 minutes, single use)",
        resp["expires_at"].as_str().unwrap_or("?")
    );
    for url in &urls {
        println!("url:     {url}");
    }
    println!();
    println!("Scan the QR in the app, or paste this:");
    println!("{connect}");
    Ok(())
}

fn print_devices(devices: &[(String, String, String, String)]) {
    if devices.is_empty() {
        println!("no paired devices");
        return;
    }
    println!("{:<18} {:<24} {:<22} LAST SEEN", "ID", "NAME", "CREATED");
    for (id, name, created, seen) in devices {
        println!("{id:<18} {name:<24} {created:<22} {seen}");
    }
}

/// The running server answers; with the server down, read the state db directly.
async fn devices_list(cfg: &Config) -> Result<(), String> {
    let text = |v: &Value| v.as_str().unwrap_or("-").to_string();
    let rows = match call(cfg, reqwest::Method::GET, "/api/v1/admin/devices").await {
        Ok(body) => body.unwrap_or_default()["devices"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|d| {
                        (
                            text(&d["id"]),
                            text(&d["name"]),
                            text(&d["created_at"]),
                            text(&d["last_seen_at"]),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        Err(CallError::Down(_)) => open_store()?
            .devices()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|d| {
                (
                    d.id,
                    d.name,
                    iso(d.created_at),
                    d.last_seen_at.map_or("-".into(), iso),
                )
            })
            .collect::<Vec<_>>(),
        Err(e) => return Err(e.into()),
    };
    print_devices(&rows);
    Ok(())
}

async fn devices_revoke(cfg: &Config, id: &str) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!(
            "{id:?} is not a device id (see `taltosbot devices list`)"
        ));
    }
    match call(
        cfg,
        reqwest::Method::DELETE,
        &format!("/api/v1/admin/devices/{id}"),
    )
    .await
    {
        Ok(_) => {}
        Err(CallError::Down(_)) => {
            if !open_store()?.revoke(id).map_err(|e| e.to_string())? {
                return Err(format!("no such device {id}"));
            }
        }
        Err(e) => return Err(e.into()),
    }
    println!("revoked {id}");
    Ok(())
}

fn open_store() -> Result<Store, String> {
    let path = config::state_dir().join("taltosbot.db");
    if !path.exists() {
        return Err(format!(
            "no state db at {} and no server running",
            path.display()
        ));
    }
    Store::open(&path).map_err(|e| format!("{}: {e}", path.display()))
}

async fn status(cfg: &Config, config_path: &std::path::Path) -> Result<(), String> {
    println!("config:  {}", config_path.display());
    println!(
        "state:   {}",
        config::state_dir().join("taltosbot.db").display()
    );
    println!("vault:   {}", cfg.vault_path.display());
    println!("listen:  {}:{}", cfg.listen_addr, cfg.listen_port);
    match call(cfg, reqwest::Method::GET, "/api/v1/health").await {
        Ok(Some(h)) if h["app"] == "taltosbot" => {
            println!(
                "server:  running, version {}",
                h["version"].as_str().unwrap_or("?")
            );
            if let Ok(Some(d)) = call(cfg, reqwest::Method::GET, "/api/v1/admin/devices").await {
                println!("devices: {}", d["devices"].as_array().map_or(0, Vec::len));
            }
        }
        Ok(_) => println!(
            "server:  something else is answering on port {}",
            cfg.listen_port
        ),
        Err(_) => println!("server:  not running"),
    }
    println!(
        "gemini:  key {}, model {}, text model {}",
        if cfg.gemini_api_key().is_some() {
            "set"
        } else {
            "NOT set"
        },
        cfg.gemini_model,
        cfg.gemini_text_model
    );
    for url in net::server_urls(&cfg.public_urls, cfg.listen_ip(), cfg.listen_port) {
        println!("url:     {url}");
    }
    for s in agents::sessions(cfg) {
        println!(
            "session: {} ({}) {} {}",
            s.project,
            s.tmux,
            s.status,
            s.url.unwrap_or_default()
        );
    }
    Ok(())
}
