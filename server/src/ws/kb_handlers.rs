use crate::ws::EventEnvelope;
use crate::pkm::Pkm;
use std::sync::Arc;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn handle_kb_get(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let path = req.payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
    match pkm.fs.read_file(path) {
        Ok(content) => create_response(req.id, "kb.result", serde_json::json!({"status": "ok", "content": content})),
        Err(e) => error_response(req.id, e),
    }
}

pub fn handle_kb_create(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let path = req.payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let content = req.payload.get("content").and_then(|v| v.as_str()).unwrap_or("");
    match pkm.fs.create_file(path, content) {
        Ok(_) => create_response(req.id, "kb.created", serde_json::json!({"status": "ok"})),
        Err(e) => error_response(req.id, e),
    }
}

pub fn handle_kb_update(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let path = req.payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let content = req.payload.get("content").and_then(|v| v.as_str()).unwrap_or("");
    match pkm.fs.update_file(path, content) {
        Ok(_) => create_response(req.id, "kb.updated", serde_json::json!({"status": "ok"})),
        Err(e) => error_response(req.id, e),
    }
}

pub fn handle_kb_edit(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let path = req.payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let start_line = req.payload.get("start_line").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let end_line = req.payload.get("end_line").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let new_content = req.payload.get("new_content").and_then(|v| v.as_str()).unwrap_or("");
    match pkm.fs.edit_file(path, start_line, end_line, new_content) {
        Ok(_) => create_response(req.id, "kb.updated", serde_json::json!({"status": "ok"})),
        Err(e) => error_response(req.id, e),
    }
}

pub fn handle_kb_delete(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let path = req.payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
    match pkm.fs.delete_file(path) {
        Ok(_) => create_response(req.id, "kb.deleted", serde_json::json!({"status": "ok"})),
        Err(e) => error_response(req.id, e),
    }
}

pub fn handle_kb_rename(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let path = req.payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let new_path = req.payload.get("new_path").and_then(|v| v.as_str()).unwrap_or("");
    match pkm.fs.rename_file(path, new_path) {
        Ok(_) => create_response(req.id, "kb.updated", serde_json::json!({"status": "ok"})),
        Err(e) => error_response(req.id, e),
    }
}

pub fn handle_kb_move(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let path = req.payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let dest_dir = req.payload.get("dest_dir").and_then(|v| v.as_str()).unwrap_or("");
    match pkm.fs.move_file(path, dest_dir) {
        Ok(_) => create_response(req.id, "kb.updated", serde_json::json!({"status": "ok"})),
        Err(e) => error_response(req.id, e),
    }
}

pub fn handle_kb_search(req: EventEnvelope, _pkm: &Arc<Pkm>) -> EventEnvelope {
    create_response(
        req.id,
        "kb.search.result",
        serde_json::json!({"status": "ok", "results": []}),
    )
}

pub fn handle_kb_list(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let path = req.payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
    match pkm.fs.list_directory(path) {
        Ok(items) => create_response(req.id, "kb.list.result", serde_json::json!({"status": "ok", "items": items})),
        Err(e) => error_response(req.id, e),
    }
}

pub fn handle_kb_list_all(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let filter = req.payload.get("filter").and_then(|v| v.as_str());
    match pkm.fs.list_vault_files(filter) {
        Ok(files) => create_response(req.id, "kb.list_all.result", serde_json::json!({"status": "ok", "files": files})),
        Err(e) => error_response(req.id, e),
    }
}

pub fn handle_kb_tree(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    match pkm.fs.get_folder_tree() {
        Ok(tree) => create_response(req.id, "kb.tree.result", serde_json::json!({"status": "ok", "tree": tree})),
        Err(e) => error_response(req.id, e),
    }
}

pub fn handle_kb_mkdir(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let path = req.payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
    match pkm.fs.create_directory(path) {
        Ok(_) => create_response(req.id, "kb.updated", serde_json::json!({"status": "ok"})),
        Err(e) => error_response(req.id, e),
    }
}

pub fn handle_kb_search_re(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let pattern = req.payload.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
    match pkm.search.search_regexp(pattern) {
        Ok(results) => create_response(req.id, "kb.search_re.result", serde_json::json!({"status": "ok", "results": results})),
        Err(e) => error_response(req.id, e),
    }
}

pub fn handle_kb_sr_file(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let path = req.payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let search = req.payload.get("search").and_then(|v| v.as_str()).unwrap_or("");
    let replace = req.payload.get("replace").and_then(|v| v.as_str()).unwrap_or("");
    match pkm.search.search_replace_file(path, search, replace) {
        Ok(_) => create_response(req.id, "kb.sr.result", serde_json::json!({"status": "ok"})),
        Err(e) => error_response(req.id, e),
    }
}

pub fn handle_kb_sr_global(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let search = req.payload.get("search").and_then(|v| v.as_str()).unwrap_or("");
    let replace = req.payload.get("replace").and_then(|v| v.as_str()).unwrap_or("");
    match pkm.search.search_replace_global(search, replace) {
        Ok(_) => create_response(req.id, "kb.sr.result", serde_json::json!({"status": "ok"})),
        Err(e) => error_response(req.id, e),
    }
}

pub fn handle_kb_read_notes(req: EventEnvelope, pkm: &Arc<Pkm>) -> EventEnvelope {
    let paths = req.payload.get("paths").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut files = serde_json::Map::new();
    for p in paths {
        if let Some(path_str) = p.as_str() {
            if let Ok(content) = pkm.fs.read_file(path_str) {
                files.insert(path_str.to_string(), serde_json::Value::String(content));
            }
        }
    }
    create_response(req.id, "kb.read_notes.result", serde_json::json!({"status": "ok", "files": files}))
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
    create_response(ref_id, "error", serde_json::json!({"error": error}))
}
