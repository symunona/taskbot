use crate::pkm::Pkm;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
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
    pkm: Arc<Pkm>,
    google_api_key: Option<String>,
}

impl WsServer {
    pub fn new(addr: String, token: String, pkm: Arc<Pkm>, google_api_key: Option<String>) -> Self {
        Self {
            addr,
            token,
            pkm,
            google_api_key,
        }
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(&self.addr).await?;
        info!("Listening on: {}", self.addr);

        while let Ok((stream, addr)) = listener.accept().await {
            let token = self.token.clone();
            let pkm = self.pkm.clone();
            let api_key = self.google_api_key.clone();

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
                                        if !crate::auth::is_valid_auth_message(text, &token) {
                                            warn!("Authentication failed for {}", addr);
                                            let _ = ws_stream
                                                .send(
                                                    tokio_tungstenite::tungstenite::Message::Text(
                                                        "Auth failed".to_string(),
                                                    ),
                                                )
                                                .await;
                                            break;
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

                                        let _ = ws_stream
                                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                                "Auth OK".to_string(),
                                            ))
                                            .await;
                                        continue;
                                    }

                                    if let Ok(envelope) =
                                        serde_json::from_str::<EventEnvelope>(text)
                                    {
                                        info!("Received event: {} from {}", envelope.event, addr);
                                        let response = match envelope.event.as_str() {
                                            "kb.get" => Self::handle_kb_get(envelope),
                                            "kb.create" => Self::handle_kb_create(envelope),
                                            "kb.update" => Self::handle_kb_update(envelope),
                                            "kb.delete" => Self::handle_kb_delete(envelope),
                                            "kb.search" => Self::handle_kb_search(envelope),
                                            "system.info" => {
                                                Self::handle_system_info(envelope, &pkm)
                                            }
                                            "system.config.get" => {
                                                info!(
                                                    "Client requested system config (API Key exchange)"
                                                );
                                                Self::handle_system_config_get(envelope, &api_key)
                                            }
                                            "thread.list" => {
                                                Self::handle_thread_list(envelope, &pkm)
                                            }
                                            "thread.create" => {
                                                Self::handle_thread_create(envelope, &pkm)
                                            }
                                            "thread.get" => Self::handle_thread_get(envelope, &pkm),
                                            "thread.append" => {
                                                Self::handle_thread_append(envelope, &pkm)
                                            }
                                            "thread.close" => {
                                                Self::handle_thread_close(envelope, &pkm)
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

    fn handle_kb_get(req: EventEnvelope) -> EventEnvelope {
        Self::create_response(
            req.id,
            "kb.result",
            serde_json::json!({"status": "ok", "content": "mock content"}),
        )
    }

    fn handle_kb_create(req: EventEnvelope) -> EventEnvelope {
        Self::create_response(req.id, "kb.created", serde_json::json!({"status": "ok"}))
    }

    fn handle_kb_update(req: EventEnvelope) -> EventEnvelope {
        Self::create_response(req.id, "kb.updated", serde_json::json!({"status": "ok"}))
    }

    fn handle_kb_delete(req: EventEnvelope) -> EventEnvelope {
        Self::create_response(req.id, "kb.deleted", serde_json::json!({"status": "ok"}))
    }

    fn handle_kb_search(req: EventEnvelope) -> EventEnvelope {
        Self::create_response(
            req.id,
            "kb.search.result",
            serde_json::json!({"status": "ok", "results": []}),
        )
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
        let uptime = System::uptime();

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
                "uptime": uptime
            }),
        )
    }

    fn handle_system_config_get(req: EventEnvelope, api_key: &Option<String>) -> EventEnvelope {
        Self::create_response(
            req.id,
            "system.config.result",
            serde_json::json!({"google_api_key": api_key}),
        )
    }

    fn handle_thread_list(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
        match pkm.threads.list_threads() {
            Ok(threads) => Self::create_response(
                req.id,
                "thread.list.result",
                serde_json::json!({"threads": threads}),
            ),
            Err(e) => Self::error_response(req.id, e.to_string()),
        }
    }

    fn handle_thread_create(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
        match pkm.threads.create_thread() {
            Ok(thread) => Self::create_response(
                req.id,
                "thread.create.result",
                serde_json::json!({"thread": thread}),
            ),
            Err(e) => Self::error_response(req.id, e.to_string()),
        }
    }

    fn handle_thread_get(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
        let thread_id = req
            .payload
            .get("thread_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match pkm.threads.get_thread(thread_id) {
            Ok(Some((thread, content))) => Self::create_response(
                req.id,
                "thread.get.result",
                serde_json::json!({"thread": thread, "content": content}),
            ),
            Ok(None) => Self::error_response(req.id, "Thread not found".to_string()),
            Err(e) => Self::error_response(req.id, e.to_string()),
        }
    }

    fn handle_thread_append(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
        let thread_id = req
            .payload
            .get("thread_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let message = req
            .payload
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match pkm.threads.append_to_thread(thread_id, message) {
            Ok(_) => Self::create_response(
                req.id,
                "thread.append.result",
                serde_json::json!({"status": "ok"}),
            ),
            Err(e) => Self::error_response(req.id, e.to_string()),
        }
    }

    fn handle_thread_close(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
        let thread_id = req
            .payload
            .get("thread_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match pkm.threads.close_thread(thread_id) {
            Ok(_) => Self::create_response(
                req.id,
                "thread.close.result",
                serde_json::json!({"status": "ok"}),
            ),
            Err(e) => Self::error_response(req.id, e.to_string()),
        }
    }

    fn create_response(ref_id: String, event: &str, payload: Value) -> EventEnvelope {
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

    fn error_response(ref_id: String, error: String) -> EventEnvelope {
        Self::create_response(ref_id, "error", serde_json::json!({"error": error}))
    }
}
