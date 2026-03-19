use crate::pkm::Pkm;
use crate::ws::EventEnvelope;
use std::sync::Arc;

pub fn handle_thread_list(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    match pkm.threads.list_threads() {
        Ok(threads) => super::WsServer::create_response(
            req.id,
            "thread.list.result",
            serde_json::json!({"threads": threads}),
        ),
        Err(e) => super::WsServer::error_response(req.id, e.to_string()),
    }
}

pub fn handle_thread_create(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    match pkm.threads.create_thread() {
        Ok(thread) => super::WsServer::create_response(
            req.id,
            "thread.create.result",
            serde_json::json!({"thread": thread}),
        ),
        Err(e) => super::WsServer::error_response(req.id, e.to_string()),
    }
}

pub fn handle_thread_get(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let thread_id = req
        .payload
        .get("thread_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match pkm.threads.get_thread(thread_id) {
        Ok(Some((thread, content))) => super::WsServer::create_response(
            req.id,
            "thread.get.result",
            serde_json::json!({"thread": thread, "content": content}),
        ),
        Ok(None) => super::WsServer::error_response(req.id, "Thread not found".to_string()),
        Err(e) => super::WsServer::error_response(req.id, e.to_string()),
    }
}

pub fn handle_thread_append(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
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
        Ok(_) => super::WsServer::create_response(
            req.id,
            "thread.append.result",
            serde_json::json!({"status": "ok"}),
        ),
        Err(e) => super::WsServer::error_response(req.id, e.to_string()),
    }
}

pub fn handle_thread_close(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let thread_id = req
        .payload
        .get("thread_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match pkm.threads.close_thread(thread_id) {
        Ok(_) => super::WsServer::create_response(
            req.id,
            "thread.closed",
            serde_json::json!({"status": "ok"}),
        ),
        Err(e) => super::WsServer::error_response(req.id, e.to_string()),
    }
}

pub fn handle_thread_archive_segment(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let thread_id = req
        .payload
        .get("thread_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let summary = req
        .payload
        .get("summary")
        .and_then(|v| v.as_str());

    match pkm.threads.archive_segment(thread_id, summary) {
        Ok(_) => super::WsServer::create_response(
            req.id,
            "thread.archived",
            serde_json::json!({"status": "ok"}),
        ),
        Err(e) => super::WsServer::error_response(req.id, e.to_string()),
    }
}
