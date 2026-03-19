use crate::pkm::Pkm;
use crate::ws::EventEnvelope;
use std::sync::Arc;

pub fn handle_history_save_batch(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    if let Some(items) = req.payload.get("items").and_then(|v| v.as_array()) {
        for item in items {
            let thread_id = item.get("thread_id").and_then(|v| v.as_str()).unwrap_or("");
            let transcript = item.get("transcript").and_then(|v| v.as_str()).unwrap_or("");
            let summary = item.get("summary").and_then(|v| v.as_str()).unwrap_or("New Thread");
            let is_closed = item.get("is_closed").and_then(|v| v.as_bool()).unwrap_or(false);
            
            let _ = pkm.history.save_transcript(thread_id, transcript, summary, is_closed);
        }
        
        super::WsServer::create_response(
            req.id,
            "history.saved",
            serde_json::json!({"status": "ok"}),
        )
    } else {
        super::WsServer::error_response(req.id, "Invalid items array".to_string())
    }
}
