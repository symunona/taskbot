//! Thread memory: fold the entries of a thread into a short "where things
//! stand" text (docs/protocol.md "Thread memory"). One Gemini call per fold,
//! never on the path of a `threads.append`.

use crate::now_ms;
use crate::server::App;
use crate::state::{Memory, Usage};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Counted entries that stay out of every fold.
pub const KEEP: usize = 4;
/// More unfolded `user` entries than this: compact.
pub const TURNS_TRIGGER: i64 = 15;
pub const MEMORY_MAX_CHARS: usize = 1500;
/// Fold input per Gemini call. More: oldest part first, then again.
const INPUT_MAX_CHARS: usize = 24_000;
const TEXT_MAX_CHARS: usize = 2000;
const TOOL_ARGS_MAX_CHARS: usize = 120;
const TOOL_RESULT_MAX_CHARS: usize = 300;
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// After a failed fold, automatic triggers leave the thread alone this long.
const BACKOFF: Duration = Duration::from_secs(60);

const INSTRUCTION: &str = "\
You maintain the memory file of a conversation between a user and a voice assistant. \
The assistant reads this file instead of the old conversation, so it must say where things stand now. It is not a history.

Input: the current memory (may be empty) and the new part of the conversation. Output: the full updated memory, nothing else.

Format, exactly two sections:
## State
- one bullet per thing agreed: decisions, goals, open questions (prefix `open:`), delegated tasks with project and outcome, exact file paths that matter
## Recent
- the last 5 to 10 user turns, one line each: `asked X -> Y`

Rules:
- Terse keywords. No articles, no filler, no full sentences. No dates, no narration.
- State holds only what the user decided, agreed to, or wants. Latest wins: when a decision changes, replace its bullet; add `(was: ...)` only if the change matters.
- A question for information never goes into State. It goes into Recent with its answer in a few words.
- Recent rolls: keep the newest 10 lines at most, drop older ones.
- Drop greetings, chit-chat and the bodies of tool results. A file path named in a decision stays, exact.
- Keep every State bullet from the current memory that is still true.
- Same language as the conversation.
- At most 1400 characters. No preamble, no code fence.";

/// Why a compact runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Why {
    /// More than [`TURNS_TRIGGER`] unfolded user turns.
    Turns,
    /// A `voice_ended` marker arrived.
    VoiceStop,
    /// `threads.compact`.
    Request,
}

/// Per-thread compact state: one fold at a time, and when the last one failed.
#[derive(Default)]
pub struct Compactor {
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    failed: Mutex<HashMap<String, Instant>>,
}

impl Compactor {
    fn lock(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());
        locks.entry(id.to_string()).or_default().clone()
    }

    fn backing_off(&self, id: &str) -> bool {
        let failed = self.failed.lock().unwrap_or_else(|e| e.into_inner());
        failed.get(id).is_some_and(|at| at.elapsed() < BACKOFF)
    }

    fn set_failed(&self, id: &str, failed: bool) {
        let mut map = self.failed.lock().unwrap_or_else(|e| e.into_inner());
        if failed {
            map.insert(id.to_string(), Instant::now());
        } else {
            map.remove(id);
        }
    }
}

fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// A counted entry as one line of fold input. `None` = not counted (marker,
/// system, empty text, not an entry).
pub fn line(entry: &Value) -> Option<String> {
    let text = entry["text"].as_str().unwrap_or("").trim();
    let said =
        |who: &str| (!text.is_empty()).then(|| format!("{who}: {}", clip(text, TEXT_MAX_CHARS)));
    match entry["kind"].as_str()? {
        "user" => said("User"),
        "model" => said("Assistant"),
        "agent" => said(&format!(
            "Agent ({})",
            entry["project"].as_str().unwrap_or("?")
        )),
        "tool" => {
            let tool = entry["tool"].as_object()?;
            let name = tool.get("name").and_then(Value::as_str).unwrap_or("?");
            let args = tool.get("args").map(Value::to_string).unwrap_or_default();
            let result = match tool.get("result") {
                Some(r) if !r.is_null() => clip(&r.to_string(), TOOL_RESULT_MAX_CHARS),
                _ => "no result".into(),
            };
            Some(format!(
                "[tool {name}({}) -> {result}]",
                clip(&args, TOOL_ARGS_MAX_CHARS)
            ))
        }
        _ => None,
    }
}

/// A tool call whose result has not arrived: the fold stops before it.
fn pending_tool(entry: &Value) -> bool {
    entry["kind"] == "tool" && entry["tool"].get("result").is_none_or(Value::is_null)
}

/// What one Gemini call folds.
#[derive(Debug, PartialEq)]
pub struct Chunk {
    pub lines: Vec<String>,
    /// `seq` of the last entry in `lines`: the new fold point.
    pub upto: i64,
    /// Foldable entries are left over: fold again.
    pub more: bool,
}

/// The oldest foldable part of `entries` (those after the fold point, in
/// `seq` order). `None` = nothing to fold.
pub fn chunk(entries: &[(i64, Value)]) -> Option<Chunk> {
    let counted: Vec<(i64, &Value, String)> = entries
        .iter()
        .filter_map(|(seq, e)| Some((*seq, e, line(e)?)))
        .collect();
    let foldable = &counted[..counted.len().saturating_sub(KEEP)];
    let settled = foldable
        .iter()
        .position(|(_, e, _)| pending_tool(e))
        .unwrap_or(foldable.len());
    let foldable = &foldable[..settled];

    let mut used = 0;
    let mut lines = Vec::new();
    let mut upto = 0;
    for (seq, _, line) in foldable {
        used += line.chars().count() + 1;
        if used > INPUT_MAX_CHARS && !lines.is_empty() {
            break;
        }
        lines.push(line.clone());
        upto = *seq;
    }
    (!lines.is_empty()).then_some(Chunk {
        more: lines.len() < foldable.len(),
        lines,
        upto,
    })
}

/// Cut a model answer down to a storable memory: no code fence, whole lines,
/// at most [`MEMORY_MAX_CHARS`].
pub fn tidy(raw: &str) -> String {
    let text = raw
        .lines()
        .filter(|l| !l.trim_start().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim();
    let Some((cut, _)) = text.char_indices().nth(MEMORY_MAX_CHARS) else {
        return text.to_string();
    };
    let head = &text[..cut];
    // Whole bullets only, unless the text is one huge line.
    match head.rfind('\n') {
        Some(nl) if nl > 0 => head[..nl].trim_end().to_string(),
        _ => head.to_string(),
    }
}

fn prompt(memory: Option<&str>, lines: &[String]) -> String {
    format!(
        "<memory>\n{}\n</memory>\n<conversation>\n{}\n</conversation>",
        memory.unwrap_or(""),
        lines.join("\n")
    )
}

/// One `generateContent` call: old memory + new lines -> new memory text.
async fn fold(
    app: &App,
    memory: Option<&str>,
    lines: &[String],
) -> Result<(String, Usage), String> {
    let key = app
        .cfg
        .gemini_api_key()
        .ok_or("gemini_api_key is not set on the server")?;
    let url = format!(
        "{}/v1beta/models/{}:generateContent",
        app.cfg.gemini_base_url.trim_end_matches('/'),
        app.cfg.summary_model()
    );
    let body = json!({
        "systemInstruction": {"parts": [{"text": INSTRUCTION}]},
        "contents": [{"role": "user", "parts": [{"text": prompt(memory, lines)}]}],
        "generationConfig": {"temperature": 0.2},
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
        .map_err(|e| format!("Gemini call failed: {}", e.without_url()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("Gemini answered {status}"));
    }
    let reply: Value = resp
        .json()
        .await
        .map_err(|e| format!("Gemini answer unreadable: {}", e.without_url()))?;
    let text: String = reply["candidates"][0]["content"]["parts"]
        .as_array()
        .into_iter()
        .flatten()
        // Thought parts are not the answer.
        .filter(|p| p["thought"] != true)
        .filter_map(|p| p["text"].as_str())
        .collect();
    let text = tidy(&text);
    if text.is_empty() {
        return Err("Gemini answered with no text".into());
    }
    let meta = &reply["usageMetadata"];
    let input = meta["promptTokenCount"].as_i64().unwrap_or(0);
    let total = meta["totalTokenCount"].as_i64().unwrap_or(input);
    Ok((
        text,
        Usage {
            input,
            output: (total - input).max(0),
        },
    ))
}

fn db(e: rusqlite::Error) -> String {
    tracing::error!("state db: {e}");
    "state db error".into()
}

/// Memory + usage of a thread as the `{memory?, usage}` payload fields.
pub fn payload(memory: Option<Memory>, usage: Usage) -> Value {
    let mut v = json!({ "usage": usage });
    if let Some(memory) = memory {
        v["memory"] = json!(memory);
    }
    v
}

/// Compact a thread if `why` still holds once it is this call's turn.
/// `Ok` = `{memory?, usage}` as it stands afterwards.
pub async fn run(app: &App, id: &str, why: Why) -> Result<Value, String> {
    let lock = app.compactor.lock(id);
    let _one_at_a_time = lock.lock().await;
    let current = || -> Result<Value, String> {
        let (memory, usage) = app
            .store
            .thread_memory(id)
            .map_err(db)?
            .ok_or_else(|| format!("unknown thread {id:?}"))?;
        Ok(payload(memory, usage))
    };
    if why != Why::Request && app.compactor.backing_off(id) {
        return current();
    }
    // The triggers are re-read here: a compact that ran while this one waited
    // may have left nothing to do.
    let turns = app.store.unfolded_turns(id).map_err(db)?;
    let due = match why {
        Why::Turns => turns > TURNS_TRIGGER,
        Why::VoiceStop => turns >= 1,
        Why::Request => true,
    };
    if !due {
        return current();
    }
    loop {
        let Some(state) = app.store.fold_state(id).map_err(db)? else {
            return current();
        };
        let Some(chunk) = chunk(&state.entries) else {
            return current();
        };
        let (text, used) = match fold(app, state.summary.as_deref(), &chunk.lines).await {
            Ok(done) => done,
            Err(e) => {
                tracing::warn!("compact of thread {id} failed: {e}");
                app.compactor.set_failed(id, true);
                return Err(e);
            }
        };
        app.compactor.set_failed(id, false);
        let stored = app
            .store
            .store_memory(id, state.upto, chunk.upto, &text, now_ms(), used)
            .map_err(db)?;
        if !stored {
            return current();
        }
        let now = current()?;
        let mut push = now.clone();
        push["thread_id"] = json!(id);
        app.push("threads.memory", push);
        if !chunk.more {
            return Ok(now);
        }
    }
}

/// After a `threads.append`: start a compact when a trigger holds. Returns at once.
pub fn after_append(app: &Arc<App>, id: &str, voice_ended: bool) {
    let why = match app.store.unfolded_turns(id) {
        Ok(turns) if turns > TURNS_TRIGGER => Why::Turns,
        Ok(turns) if voice_ended && turns >= 1 => Why::VoiceStop,
        _ => return,
    };
    if app.compactor.backing_off(id) {
        return;
    }
    let (app, id) = (app.clone(), id.to_string());
    tokio::spawn(async move {
        // Failures are logged in `run`; the old memory stays.
        let _ = run(&app, &id, why).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(seq: i64, kind: &str, text: &str) -> (i64, Value) {
        (
            seq,
            json!({"id": format!("e{seq}"), "ts": seq, "kind": kind, "text": text}),
        )
    }

    fn tool(seq: i64, result: Option<Value>) -> (i64, Value) {
        let mut tool = json!({"name": "read_file", "args": {"path": "a.md"}, "called_at": 1});
        if let Some(r) = result {
            tool["result"] = r;
        }
        (
            seq,
            json!({"id": format!("e{seq}"), "ts": seq, "kind": "tool", "text": "", "tool": tool}),
        )
    }

    #[test]
    fn lines_per_kind() {
        assert_eq!(line(&said(1, "user", " hi ").1).unwrap(), "User: hi");
        assert_eq!(line(&said(1, "model", "yo").1).unwrap(), "Assistant: yo");
        let mut agent = said(1, "agent", "done").1;
        agent["project"] = json!("vault");
        assert_eq!(line(&agent).unwrap(), "Agent (vault): done");
        for skipped in [
            said(1, "marker", "voice_ended").1,
            said(1, "system", "note").1,
            said(1, "user", "  ").1,
            Value::Null,
        ] {
            assert_eq!(line(&skipped), None, "{skipped}");
        }
        let big = json!({"content": "x".repeat(5000)});
        let got = line(&tool(1, Some(big)).1).unwrap();
        assert!(got.starts_with(r#"[tool read_file({"path":"a.md"}) -> {"content":"xxx"#));
        assert!(got.chars().count() < 400, "result clipped");
        assert!(line(&tool(1, None).1).unwrap().ends_with("-> no result]"));
        assert!(line(&said(1, "user", &"é".repeat(3000)).1)
            .unwrap()
            .ends_with('…'));
    }

    #[test]
    fn chunk_keeps_the_tail_and_skips_uncounted() {
        let mut entries: Vec<_> = (1..=6).map(|i| said(i, "user", "x")).collect();
        entries.insert(2, said(100, "marker", "voice_started"));
        entries.push(said(7, "system", "note"));
        let got = chunk(&entries).unwrap();
        assert_eq!((got.lines.len(), got.upto, got.more), (2, 2, false));

        let few: Vec<_> = (1..=KEEP as i64).map(|i| said(i, "user", "x")).collect();
        assert_eq!(chunk(&few), None);
        assert_eq!(chunk(&[]), None);
    }

    #[test]
    fn chunk_stops_before_a_pending_tool() {
        let mut entries = vec![said(1, "user", "a"), tool(2, Some(json!({"ok": true})))];
        entries.push(tool(3, None));
        entries.extend((4..=9).map(|i| said(i, "user", "x")));
        let got = chunk(&entries).unwrap();
        assert_eq!((got.lines.len(), got.upto, got.more), (2, 2, false));
        // Pending tool first: nothing settled to fold.
        let mut blocked = vec![tool(1, None)];
        blocked.extend((2..=9).map(|i| said(i, "user", "x")));
        assert_eq!(chunk(&blocked), None);
    }

    #[test]
    fn chunk_splits_big_input() {
        let text = "y".repeat(TEXT_MAX_CHARS);
        let entries: Vec<_> = (1..=30).map(|i| said(i, "user", &text)).collect();
        let first = chunk(&entries).unwrap();
        assert!(first.more);
        assert_eq!(first.lines.len(), 11, "11 * ~2k fits 24k, 12 does not");
        assert_eq!(first.upto, 11);
        let rest = chunk(&entries[11..]).unwrap();
        assert_eq!((rest.lines.len(), rest.upto, rest.more), (11, 22, true));
        let last = chunk(&entries[22..]).unwrap();
        assert_eq!((last.lines.len(), last.upto, last.more), (4, 26, false));
    }

    #[test]
    fn tidy_caps_on_a_line_end() {
        assert_eq!(tidy("```\n## State\n- a\n```\n"), "## State\n- a");
        let long: String = (0..400).map(|i| format!("- bullet {i}\n")).collect();
        let got = tidy(&long);
        assert!(got.chars().count() <= MEMORY_MAX_CHARS);
        assert!(got.ends_with(char::is_numeric) && got.lines().all(|l| l.starts_with("- bullet ")));
        assert_eq!(tidy(&"é".repeat(5000)).chars().count(), MEMORY_MAX_CHARS);
        assert_eq!(tidy("  \n "), "");
    }
}
