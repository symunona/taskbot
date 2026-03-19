pub mod kb_handlers;
pub mod thread_handlers;
pub mod history_handlers;

use crate::pkm::Pkm;
use crate::pairing_store::PairingStore;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_tungstenite::accept_async;
use tracing::{info, warn};

#[derive(Debug, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
    pub ts: u64,
    pub payload: Value,
}

pub struct WsServer {
    addr: String,
    token: String,
    legacy_auth: bool,
    pkm: Arc<Pkm>,
    pairing_store: Arc<Mutex<PairingStore>>,
    google_api_key: Option<String>,
    keys: std::collections::HashMap<String, String>,
}

impl WsServer {
    pub fn new(
        addr: String,
        token: String,
        legacy_auth: bool,
        pkm: Arc<Pkm>,
        pairing_store: Arc<Mutex<PairingStore>>,
        google_api_key: Option<String>,
        keys: std::collections::HashMap<String, String>
    ) -> Self {
        Self {
            addr,
            token,
            legacy_auth,
            pkm,
            pairing_store,
            google_api_key,
            keys,
        }
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(&self.addr).await?;
        info!("Listening on: {}", self.addr);

        while let Ok((stream, addr)) = listener.accept().await {
            let token = self.token.clone();
            let legacy_auth = self.legacy_auth;
            let pkm = self.pkm.clone();
            let pairing_store = self.pairing_store.clone();
            let api_key = self.google_api_key.clone();
            let keys = self.keys.clone();

            tokio::spawn(async move {
                match accept_async(stream).await {
                    Ok(mut ws_stream) => {
                        info!("New WebSocket connection from {}", addr);

                        let mut authenticated = false;

                        while let Some(msg) = ws_stream.next().await {
                            if let Ok(msg) = msg {
                                if msg.is_text() {
                                    let text = msg.to_text().unwrap();

                                    if !authenticated {
                                        let mut auth_ok = false;
                                        let mut auth_response = None;

                                        if let Ok(event) = serde_json::from_str::<crate::auth::AuthEvent>(text) {
                                            if event.event == "auth.pair" {
                                                if let Some(pairing_token) = event.payload.pairing_token {
                                                    let mut store = pairing_store.lock().await;
                                                    match store.validate_and_consume_pairing_token(&pairing_token) {
                                                        Ok(ac) => {
                                                            auth_ok = true;
                                                            auth_response = Some(serde_json::json!({
                                                                "event": "auth.paired",
                                                                "payload": {
                                                                    "client_token": ac.client_token
                                                                }
                                                            }));
                                                        }
                                                        Err(reason) => {
                                                            auth_response = Some(serde_json::json!({
                                                                "event": "auth.error",
                                                                "payload": {
                                                                    "reason": reason
                                                                }
                                                            }));
                                                        }
                                                    }
                                                }
                                            } else if event.event == "auth.token" {
                                                if let Some(client_token) = event.payload.client_token {
                                                    let mut store = pairing_store.lock().await;
                                                    if store.validate_client_token(&client_token).is_ok() {
                                                        auth_ok = true;
                                                        auth_response = Some(serde_json::json!({
                                                            "event": "auth.ok"
                                                        }));
                                                    } else {
                                                        auth_response = Some(serde_json::json!({
                                                            "event": "auth.error",
                                                            "payload": {
                                                                "reason": "invalid_token"
                                                            }
                                                        }));
                                                    }
                                                } else if legacy_auth {
                                                    if let Some(legacy_token) = event.payload.token {
                                                        if legacy_token == token {
                                                            auth_ok = true;
                                                            auth_response = Some(serde_json::json!({
                                                                "event": "auth.ok"
                                                            }));
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        if !auth_ok {
                                            warn!("Authentication failed for {}", addr);
                                            let resp = auth_response.unwrap_or_else(|| serde_json::json!({
                                                "event": "auth.error",
                                                "payload": {
                                                    "reason": "invalid_auth_request"
                                                }
                                            }));
                                            let _ = ws_stream
                                                .send(
                                                    tokio_tungstenite::tungstenite::Message::Text(
                                                        serde_json::to_string(&resp).unwrap()
                                                    ),
                                                )
                                                .await;
                                            // Don't break immediately, let the client read the error
                                            continue;
                                        }
                                        
                                        authenticated = true;
                                        info!("Client {} authenticated successfully", addr);

                                        // Register session
                                        let client_id = format!("client_{}", addr.port());
                                        if let Ok(session) = pkm.sessions.create_session(
                                            &client_id,
                                            Some(&addr.ip().to_string()),
                                            None,
                                        ) {
                                            info!("Created session: {}", session.id);
                                        }

                                        if let Some(resp) = auth_response {
                                            let _ = ws_stream
                                                .send(tokio_tungstenite::tungstenite::Message::Text(
                                                    serde_json::to_string(&resp).unwrap()
                                                ))
                                                .await;
                                        }
                                        continue;
                                    }

                                    if let Ok(envelope) =
                                        serde_json::from_str::<EventEnvelope>(text)
                                    {
                                        info!("Received event: {} from {}", envelope.event, addr);
                                        let response = match envelope.event.as_str() {
                                            "kb.get" => kb_handlers::handle_kb_get(envelope, &pkm),
                                            "kb.create" => kb_handlers::handle_kb_create(envelope, &pkm),
                                            "kb.update" => kb_handlers::handle_kb_update(envelope, &pkm),
                                            "kb.edit" => kb_handlers::handle_kb_edit(envelope, &pkm),
                                            "kb.delete" => kb_handlers::handle_kb_delete(envelope, &pkm),
                                            "kb.rename" => kb_handlers::handle_kb_rename(envelope, &pkm),
                                            "kb.move" => kb_handlers::handle_kb_move(envelope, &pkm),
                                            "kb.search" => kb_handlers::handle_kb_search(envelope, &pkm),
                                            "kb.list" => kb_handlers::handle_kb_list(envelope, &pkm),
                                            "kb.list_all" => kb_handlers::handle_kb_list_all(envelope, &pkm),
                                            "kb.tree" => kb_handlers::handle_kb_tree(envelope, &pkm),
                                            "kb.mkdir" => kb_handlers::handle_kb_mkdir(envelope, &pkm),
                                            "kb.search_re" => kb_handlers::handle_kb_search_re(envelope, &pkm),
                                            "kb.sr_file" => kb_handlers::handle_kb_sr_file(envelope, &pkm),
                                            "kb.sr_global" => kb_handlers::handle_kb_sr_global(envelope, &pkm),
                                            "kb.read_notes" => kb_handlers::handle_kb_read_notes(envelope, &pkm),
                                            "system.info" => {
                                                Self::handle_system_info(envelope, &pkm)
                                            }
                                            "system.config.get" => {
                                                info!(
                                                    "Client requested system config (API Key exchange)"
                                                );
                                                Self::handle_system_config_get(envelope, &api_key, &keys)
                                            }
                                            "thread.list" => {
                                                thread_handlers::handle_thread_list(envelope, &pkm)
                                            }
                                            "thread.create" => {
                                                thread_handlers::handle_thread_create(envelope, &pkm)
                                            }
                                            "thread.get" => thread_handlers::handle_thread_get(envelope, &pkm),
                                            "thread.append" => {
                                                thread_handlers::handle_thread_append(envelope, &pkm)
                                            }
                                            "thread.close" => {
                                                thread_handlers::handle_thread_close(envelope, &pkm)
                                            }
                                            "thread.archive_segment" => {
                                                thread_handlers::handle_thread_archive_segment(envelope, &pkm)
                                            }
                                            "history.save_batch" => {
                                                history_handlers::handle_history_save_batch(envelope, &pkm)
                                            }
                                            _ => {
                                                warn!("Unknown event received: {}", envelope.event);
                                                Self::error_response(
                                                    envelope.id,
                                                    "Unknown event".to_string(),
                                                )
                                            }
                                        };

                                        let _ = ws_stream
                                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                                serde_json::to_string(&response).unwrap(),
                                            ))
                                            .await;
                                    } else {
                                        warn!("Failed to parse message from {}", addr);
                                    }
                                }
                            } else if let Err(e) = msg {
                                warn!("Error reading WebSocket message from {}: {}", addr, e);
                                break;
                            }
                        }
                        info!("Client {} disconnected", addr);
                    }
                    Err(e) => {
                        warn!("WebSocket handshake failed for {}: {}", addr, e);
                    }
                }
            });
        }
        Ok(())
    }

    fn handle_system_info(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
        use sysinfo::System;
        let mut sys = System::new_all();
        sys.refresh_all();

        let hostname = System::host_name().unwrap_or_else(|| "unknown".to_string());
        let cpu_info = sys
            .cpus()
            .first()
            .map(|c| c.brand())
            .unwrap_or("unknown")
            .to_string();
            
        let uptime_secs = System::uptime();
        let days = uptime_secs / 86400;
        let hours = (uptime_secs % 86400) / 3600;
        let minutes = (uptime_secs % 3600) / 60;
        let uptime_str = if days > 0 {
            format!("{}d {}h {}m", days, hours, minutes)
        } else if hours > 0 {
            format!("{}h {}m", hours, minutes)
        } else {
            format!("{}m", minutes)
        };

        fn format_bytes(bytes: u64) -> String {
            let gb = bytes as f64 / 1_073_741_824.0;
            if gb >= 1.0 {
                format!("{:.2} GB", gb)
            } else {
                let mb = bytes as f64 / 1_048_576.0;
                format!("{:.2} MB", mb)
            }
        }

        let ram_total = format_bytes(sys.total_memory());
        let ram_used = format_bytes(sys.used_memory());
        let ram_available = format_bytes(sys.available_memory());

        let mut gpu_info = serde_json::Value::Null;
        if let Ok(output) = std::process::Command::new("nvidia-smi")
            .arg("--query-gpu=memory.used,memory.total")
            .arg("--format=csv,noheader,nounits")
            .output()
        {
            if let Ok(stdout) = String::from_utf8(output.stdout) {
                if let Some(line) = stdout.lines().next() {
                    let parts: Vec<&str> = line.split(',').collect();
                    if parts.len() == 2 {
                        gpu_info = serde_json::json!({
                            "used": format!("{} MB", parts[0].trim()),
                            "total": format!("{} MB", parts[1].trim())
                        });
                    }
                }
            }
        }

        let vault_file_count = pkm
            .db
            .conn
            .lock()
            .map(|conn| {
                conn.query_row("SELECT COUNT(*) FROM files", [], |row| row.get::<_, i64>(0))
                    .unwrap_or(0)
            })
            .unwrap_or(0);

        Self::create_response(
            req.id,
            "system.info.result",
            serde_json::json!({
                "hostname": hostname,
                "cpu_info": cpu_info,
                "vault_file_count": vault_file_count,
                "uptime": uptime_str,
                "ram_total": ram_total,
                "ram_used": ram_used,
                "ram_available": ram_available,
                "gpu": gpu_info
            }),
        )
    }

    fn handle_system_config_get(req: EventEnvelope, api_key: &Option<String>, keys: &std::collections::HashMap<String, String>) -> EventEnvelope {
        Self::create_response(
            req.id,
            "system.config.result",
            serde_json::json!({
                "google_api_key": api_key,
                "keys": keys
            }),
        )
    }

    pub fn create_response(ref_id: String, event: &str, payload: Value) -> EventEnvelope {
        EventEnvelope {
            event: event.to_string(),
            id: uuid::Uuid::new_v4().to_string(),
            ref_id: Some(ref_id),
            ts: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            payload,
        }
    }

    pub fn error_response(ref_id: String, error: String) -> EventEnvelope {
        Self::create_response(ref_id, "error", serde_json::json!({"error": error}))
    }
}
