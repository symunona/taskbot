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
    Pair,
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

fn candidate_config_paths() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(explicit_path) = std::env::var("HERMES_CONFIG") {
        let explicit_path = explicit_path.trim();
        if !explicit_path.is_empty() {
            candidates.push(PathBuf::from(explicit_path));
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("config.toml"));
        candidates.push(cwd.join("server").join("config.toml"));
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            candidates.push(exe_dir.join("config.toml"));
            for ancestor in exe_dir.ancestors() {
                candidates.push(ancestor.join("config.toml"));
                candidates.push(ancestor.join("server").join("config.toml"));
            }
        }
    }

    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let manifest_dir = PathBuf::from(manifest_dir);
        candidates.push(manifest_dir.join("config.toml"));
        if let Some(parent) = manifest_dir.parent() {
            candidates.push(parent.join("server").join("config.toml"));
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".config").join("hermes").join("config.toml"));
        candidates.push(
            home.join("dev")
                .join("hermes")
                .join("taskbot")
                .join("server")
                .join("config.toml"),
        );
    }

    let mut seen = BTreeSet::new();
    candidates.retain(|path| seen.insert(path.clone()));
    candidates
}

fn resolve_config_path() -> PathBuf {
    candidate_config_paths()
        .into_iter()
        .find(|path| path.exists())
        .unwrap_or_else(|| Path::new("config.toml").to_path_buf())
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
    let config_path = resolve_config_path();
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
        Commands::Pair => {
            let pubkey = "mock_pubkey"; // Replace with actual pubkey logic
            let pubkey_base64 = STANDARD.encode(pubkey);
            let addrs = public_address_candidates(&config);

            let addrs_query = addrs
                .iter()
                .map(|a| format!("addrs={}", a))
                .collect::<Vec<_>>()
                .join("&");
            let conn_str = format!(
                "hermes://{}/{}?{}",
                pubkey_base64, config.token, addrs_query
            );

            println!("Connection string:\n{}", conn_str);
            qr2term::print_qr(&conn_str).unwrap();
        }
        Commands::Status => {
            println!("Hermes TX Service Health Condition:");
            println!("----------------------------------");

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
        Commands::Serve => {
            init_logging();
            let now = chrono::Local::now();
            info!("============================================================");
            info!(
                "Booting Hermes Server at {}",
                now.format("%Y-%m-%d %H:%M:%S")
            );
            info!("============================================================");
            info!("Using config file {}", config_path.display());

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

            let pkm = tx::pkm::Pkm::new(config.vault_path.clone(), PathBuf::from("hermes.db"))?;
            pkm.index_vault()?;
            pkm.watcher.watch()?;

            let pkm_arc = Arc::new(pkm);
            let ws_server = tx::ws::WsServer::new(
                config.listen_addr.clone(),
                config.token.clone(),
                pkm_arc,
                resolved_api_key.map(|(key, _)| key),
            );
            ws_server.run().await?;
        }
    }

    Ok(())
}
