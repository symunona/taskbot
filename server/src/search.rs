//! `web_search` tool: one Gemini `generateContent` call grounded with Google Search.
//! The answer text goes back to the voice model, with the pages it rests on.

use crate::server::App;
use serde_json::{json, Value};
use std::time::Duration;

/// Under the app's 30 s tool timeout: a slow search must answer as an error, not time out there.
const CALL_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_SOURCES: usize = 8;

pub async fn search(app: &App, query: &str) -> Result<Value, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("query is empty".into());
    }
    let key = app
        .cfg
        .gemini_api_key()
        .ok_or("gemini_api_key is not set on the server")?;
    let url = format!(
        "{}/v1beta/models/{}:generateContent",
        app.cfg.gemini_base_url.trim_end_matches('/'),
        app.cfg.gemini_text_model
    );
    let body = json!({
        // The answer is read out loud by the voice model.
        "systemInstruction": {"parts": [{"text": "Answer short and factual, plain text, no markdown."}]},
        "contents": [{"role": "user", "parts": [{"text": query}]}],
        "tools": [{"google_search": {}}],
    });
    let resp = app
        .http
        .post(url)
        .header("x-goog-api-key", key)
        .timeout(CALL_TIMEOUT)
        .json(&body)
        .send()
        .await
        // No URL in the message: keep it short, and a key must never ride along.
        .map_err(|e| format!("web search failed: {}", e.without_url()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("web search: Gemini answered {status}"));
    }
    let reply: Value = resp
        .json()
        .await
        .map_err(|e| format!("web search answer unreadable: {}", e.without_url()))?;
    result(query, &reply)
}

/// `{query, text, sources:[{title, uri}]}` out of a grounded `generateContent` reply.
fn result(query: &str, reply: &Value) -> Result<Value, String> {
    let candidate = &reply["candidates"][0];
    let text: String = candidate["content"]["parts"]
        .as_array()
        .into_iter()
        .flatten()
        // Thought parts are not the answer.
        .filter(|p| p["thought"] != true)
        .filter_map(|p| p["text"].as_str())
        .collect();
    let text = text.trim();
    if text.is_empty() {
        return Err("web search gave no answer".into());
    }
    let sources: Vec<Value> = candidate["groundingMetadata"]["groundingChunks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let uri = c["web"]["uri"].as_str()?;
            Some(json!({"title": c["web"]["title"].as_str().unwrap_or(""), "uri": uri}))
        })
        .take(MAX_SOURCES)
        .collect();
    Ok(json!({"query": query, "text": text, "sources": sources}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_and_sources() {
        let reply = json!({"candidates": [{
            "content": {"parts": [
                {"text": "thinking", "thought": true},
                {"text": "Rain "},
                {"text": "tomorrow.\n"},
            ]},
            "groundingMetadata": {"groundingChunks": [
                {"web": {"uri": "https://a.example/x", "title": "a.example"}},
                {"web": {"title": "no uri"}},
                {"web": {"uri": "https://b.example/y"}},
            ]},
        }]});
        assert_eq!(
            result("weather", &reply).unwrap(),
            json!({
                "query": "weather",
                "text": "Rain tomorrow.",
                "sources": [
                    {"title": "a.example", "uri": "https://a.example/x"},
                    {"title": "", "uri": "https://b.example/y"},
                ],
            })
        );
    }

    #[test]
    fn no_text_is_an_error() {
        assert!(result("q", &json!({"candidates": []})).is_err());
        let ungrounded = json!({"candidates": [{"content": {"parts": [{"text": "Hi"}]}}]});
        assert_eq!(result("q", &ungrounded).unwrap()["sources"], json!([]));
    }
}
