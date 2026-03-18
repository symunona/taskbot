use base64::{engine::general_purpose::STANDARD, Engine as _};
use clap::{Parser, Subcommand};
use local_ip_address::local_ip;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tx::config::Config;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Pair {
        #[arg(short, long)]
        ttl: Option<u64>,
    },
    Serve,
    Log {
        #[arg(short, long)]
        follow: bool,
    },
    Status,
    Session {
        #[command(subcommand)]
        command: Option<SessionCommands>,
    },
    Clients {
        #[command(subcommand)]
        command: Option<ClientsCommands>,
    },
}

#[derive(Subcommand)]
enum ClientsCommands {
    List,
    Revoke { id: String },
}

#[derive(Subcommand)]
enum SessionCommands {
    List,
    Delete { id: String },
}

fn get_log_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let dir = PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("hermes");
        std::fs::create_dir_all(&dir).ok();
        dir
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }
}

fn init_logging() {
    let log_dir = get_log_dir();
    let file_appender = tracing_appender::rolling::never(log_dir, "tx.log");

    // We use a direct writer for the file to ensure logs are persisted immediately
    // even if the service is killed (e.g. SIGTERM by systemd).
    // For stdout, we could use non-blocking, but for simplicity we'll just use both directly.
    let (non_blocking_stdout, _guard) = tracing_appender::non_blocking(std::io::stdout());

    // Leak the guard so stdout non-blocking thread stays alive for the lifetime of the program
    std::mem::forget(_guard);

    tracing_subscriber::fmt()
        .with_writer(file_appender.and(non_blocking_stdout))
        .init();
}

fn candidate_config_paths() -> Vec<(PathBuf, &'static str)> {
    let mut candidates = Vec::new();

    if let Ok(explicit_path) = std::env::var("HERMES_CONFIG") {
        let explicit_path = explicit_path.trim();
        if !explicit_path.is_empty() {
            candidates.push((PathBuf::from(explicit_path), "HERMES_CONFIG env var"));
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push((cwd.join("config.toml"), "current working directory"));
        candidates.push((cwd.join("server").join("config.toml"), "server/ directory in current working directory"));
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            candidates.push((exe_dir.join("config.toml"), "executable directory"));
            for ancestor in exe_dir.ancestors() {
                candidates.push((ancestor.join("config.toml"), "ancestor of executable directory"));
                candidates.push((ancestor.join("server").join("config.toml"), "server/ in ancestor of executable directory"));
            }
        }
    }

    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let manifest_dir = PathBuf::from(manifest_dir);
        candidates.push((manifest_dir.join("config.toml"), "cargo manifest directory"));
        if let Some(parent) = manifest_dir.parent() {
            candidates.push((parent.join("server").join("config.toml"), "server/ in parent of cargo manifest directory"));
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        candidates.push((home.join(".config").join("taskbot").join("config.toml"), "user config directory"));
        candidates.push((
            home.join("dev")
                .join("hermes")
                .join("taskbot")
                .join("server")
                .join("config.toml"),
            "dev taskbot directory"
        ));
    }

    let mut seen = BTreeSet::new();
    candidates.retain(|(path, _)| seen.insert(path.clone()));
    candidates
}

fn resolve_config_path() -> (PathBuf, &'static str) {
    let candidates = candidate_config_paths();
    
    // Sort to prefer local configs in debug mode, and ~/.config/taskbot in release mode (or service)
    let is_release = !cfg!(debug_assertions);
    if is_release {
        // In release mode, prefer ~/.config/taskbot/config.toml
        if let Ok(home) = std::env::var("HOME") {
            let default_path = PathBuf::from(home).join(".config").join("taskbot").join("config.toml");
            if default_path.exists() {
                return (default_path, "user config directory (default for release/service)");
            }
        }
    }

    candidates
        .into_iter()
        .find(|(path, _)| path.exists())
        .unwrap_or_else(|| (Path::new("config.toml").to_path_buf(), "fallback default path"))
}

fn public_address_candidates(config: &Config) -> Vec<String> {
    let mut addrs = Vec::new();
    let port = config.listen_addr.split(':').next_back().unwrap_or("55566");

    if let Some(domain) = &config.public_domain {
        let domain = domain.trim();
        if !domain.is_empty() {
            if domain.contains(':') {
                addrs.push(domain.to_string());
            } else {
                addrs.push(format!("{}:443", domain));
            }
        }
    }

    if let Ok(my_local_ip) = local_ip() {
        addrs.push(format!("{}:{}", my_local_ip, port));
    }

    let mut seen = BTreeSet::new();
    addrs.retain(|addr| seen.insert(addr.clone()));
    addrs
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let (config_path, config_source) = resolve_config_path();
    let config = Config::load(&config_path)?;

    match &cli.command {
        Commands::Log { follow } => {
            let log_path = get_log_dir().join("tx.log");
            if !log_path.exists() {
                // Fallback to current directory for backwards compatibility during transition
                let fallback = std::env::current_dir()?.join("tx.log");
                if fallback.exists() {
                    if *follow {
                        let err = std::process::Command::new("tail")
                            .args(["-f", fallback.to_str().unwrap()])
                            .exec();
                        return Err(err.into());
                    } else {
                        let mut file = File::open(&fallback)?;
                        let mut contents = String::new();
                        file.read_to_string(&mut contents)?;
                        print!("{}", contents);
                        return Ok(());
                    }
                }
                println!("No logs found at {:?}.", log_path);
                return Ok(());
            }
            if *follow {
                let err = std::process::Command::new("tail")
                    .args(["-f", log_path.to_str().unwrap()])
                    .exec();
                return Err(err.into());
            } else {
                let mut file = File::open(&log_path)?;
                let mut contents = String::new();
                file.read_to_string(&mut contents)?;
                print!("{}", contents);
            }
        }
        Commands::Pair { ttl } => {
            let pubkey = "mock_pubkey"; // Replace with actual pubkey logic
            let pubkey_base64 = STANDARD.encode(pubkey);
            let addrs = public_address_candidates(&config);

            let addrs_query = addrs
                .iter()
                .map(|a| format!("addrs={}", a))
                .collect::<Vec<_>>()
                .join("&");
                
            let mut pairing_store = tx::pairing_store::PairingStore::new(config.vault_path.join("pairing_store.json"))?;
            let ttl_secs = ttl.unwrap_or(config.pairing_ttl_secs);
            let pt = pairing_store.create_pairing_token(ttl_secs)?;

            let conn_str = format!(
                "hermes://{}/{}?{}&exp={}",
                pubkey_base64, pt.token, addrs_query, pt.expires_at
            );

            println!("Connection string:\n{}", conn_str);
            qr2term::print_qr(&conn_str).unwrap();
        }
        Commands::Status => {
            println!("Hermes TX Service Health Condition:");
            println!("----------------------------------");
            println!("- Config file: {} (resolved via: {})", config_path.display(), config_source);

            // Check if listening
            println!("- Service should be listening on {}", config.listen_addr);

            // Check UFW status for the port
            let port = config.listen_addr.split(':').next_back().unwrap_or("55566");
            let ufw_output = std::process::Command::new("sudo")
                .args(["-n", "ufw", "status"])
                .output()
                .ok();

            if let Some(output) = ufw_output {
                let status_str = String::from_utf8_lossy(&output.stdout);
                if status_str.contains(port) {
                    println!("- UFW is open on port {}", port);
                } else {
                    println!("- WARNING: UFW does not appear to be open on port {}", port);
                }
            } else {
                println!("- Unable to check UFW status (requires sudo/ufw installed)");
            }

            // Check domain and certs
            if let Some(domain) = &config.public_domain {
                if !domain.is_empty() {
                    let domain_name = if domain.contains(':') {
                        domain.split(':').next().unwrap()
                    } else {
                        domain.as_str()
                    };

                    println!("- Configured public domain: {}", domain);

                    // Check certs
                    let cert_path = format!("/etc/letsencrypt/live/{}/fullchain.pem", domain_name);
                    if std::path::Path::new(&cert_path).exists() {
                        println!("- Certs installed for the domain ({})", cert_path);
                    } else {
                        println!("- WARNING: Certs not found at {}", cert_path);
                    }
                } else {
                    println!("- No public domain configured");
                }
            } else {
                println!("- No public domain configured");
            }
        }
        Commands::Session { command } => {
            let pkm = tx::pkm::Pkm::new(config.vault_path.clone(), PathBuf::from("hermes.db"))?;

            match command {
                Some(SessionCommands::List) | None => {
                    let sessions = pkm.sessions.list_sessions()?;
                    if sessions.is_empty() {
                        println!("No active sessions found.");
                    } else {
                        println!(
                            "{:<36} | {:<20} | {:<15} | Last Seen",
                            "ID", "Client ID", "IP Address"
                        );
                        println!("{:-<36}-+-{:-<20}-+-{:-<15}-+-{:-<20}", "", "", "", "");
                        for s in sessions {
                            let ip = s.ip_address.unwrap_or_else(|| "Unknown".to_string());
                            let last_seen = chrono::DateTime::from_timestamp(s.last_seen as i64, 0)
                                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                                .unwrap_or_else(|| s.last_seen.to_string());

                            println!(
                                "{:<36} | {:<20} | {:<15} | {}",
                                s.id, s.client_id, ip, last_seen
                            );
                        }
                    }
                }
                Some(SessionCommands::Delete { id }) => {
                    pkm.sessions.delete_session(id)?;
                    println!("Session {} deleted.", id);
                }
            }
        }
        Commands::Clients { command } => {
            let mut pairing_store = tx::pairing_store::PairingStore::new(config.vault_path.join("pairing_store.json"))?;
            match command {
                Some(ClientsCommands::List) | None => {
                    let clients = pairing_store.list_clients();
                    if clients.is_empty() {
                        println!("No authorized clients found.");
                    } else {
                        println!(
                            "{:<36} | {:<20} | {:<20} | {}",
                            "Client ID", "Paired At", "Last Seen", "Label"
                        );
                        println!("{:-<36}-+-{:-<20}-+-{:-<20}-+-{:-<20}", "", "", "", "");
                        for c in clients {
                            let paired_at = chrono::DateTime::from_timestamp(c.paired_at as i64, 0)
                                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                                .unwrap_or_else(|| c.paired_at.to_string());
                            let last_seen = c.last_seen.and_then(|ts| {
                                chrono::DateTime::from_timestamp(ts as i64, 0)
                                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                            }).unwrap_or_else(|| "Never".to_string());
                            let label = c.label.unwrap_or_else(|| "None".to_string());

                            println!(
                                "{:<36} | {:<20} | {:<20} | {}",
                                c.client_id, paired_at, last_seen, label
                            );
                        }
                    }
                }
                Some(ClientsCommands::Revoke { id }) => {
                    pairing_store.revoke_client(id)?;
                    println!("Client {} revoked.", id);
                }
            }
        }
        Commands::Serve => {
            init_logging();
            let now = chrono::Local::now();
            info!("============================================================");
            info!(
                "Booting Hermes Server at {}",
                now.format("%Y-%m-%d %H:%M:%S")
            );
            info!("============================================================");
            info!("Using config file {} (resolved via: {})", config_path.display(), config_source);

            let resolved_api_key = config.resolved_google_api_key_with_source();
            match &resolved_api_key {
                Some((_, source)) => {
                    info!("Gemini API key loaded from {}", source.description());
                }
                None => {
                    warn!(
                        "No Gemini API key configured. Checked GEMINI_API_KEY, GOOGLE_API_KEY, [keys].gemini_api_key, [keys].google_api_key, and legacy google_api_key."
                    );
                }
            }

            if !config.keys.is_empty() {
                let mut key_names: Vec<&String> = config.keys.keys().collect();
                key_names.sort();
                let names_str = key_names
                    .into_iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                info!("Available keys loaded: {}", names_str);
            } else {
                info!("Available keys loaded: None");
            }

            let pkm = tx::pkm::Pkm::new(config.vault_path.clone(), PathBuf::from("hermes.db"))?;
            pkm.index_vault()?;
            pkm.watcher.watch()?;

            let pkm_arc = Arc::new(pkm);
            let pairing_store = tx::pairing_store::PairingStore::new(config.vault_path.join("pairing_store.json"))?;
            let pairing_store_arc = Arc::new(tokio::sync::Mutex::new(pairing_store));
            
            let ws_server = tx::ws::WsServer::new(
                config.listen_addr.clone(),
                config.token.clone(),
                config.legacy_auth,
                pkm_arc,
                pairing_store_arc,
                resolved_api_key.map(|(key, _)| key),
                config.keys.clone(),
            );
            ws_server.run().await?;
        }
    }

    Ok(())
}
