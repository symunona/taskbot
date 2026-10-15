//! Tools offered to the voice model. Declarations use the Gemini
//! `functionDeclarations[].parameters` shape. Vault tools only read. The
//! `memory_*` tools write pins: sqlite, or the project's `.taskbot.md`.
//! `end_conversation` is acted on by the app: here it only answers.
//! `switch_thread` finds or creates the thread; the app opens it.

use crate::server::App;
use serde_json::{json, Value};
use std::sync::Arc;

fn decl(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    // Gemini rejects an OBJECT with no properties: no-arg tools carry no `parameters`.
    if props.as_object().is_some_and(|p| p.is_empty()) {
        return json!({"name": name, "description": description});
    }
    json!({
        "name": name,
        "description": description,
        "parameters": {"type": "OBJECT", "properties": props, "required": required},
    })
}

fn string(description: &str) -> Value {
    json!({"type": "STRING", "description": description})
}

fn integer(description: &str) -> Value {
    json!({"type": "INTEGER", "description": description})
}

pub fn declarations() -> Vec<Value> {
    vec![
        decl(
            "read_file",
            "Read one text file from user's notes vault. Use when exact path known (from list_directory, \
             search_regexp). Small file: whole content. Big file: outline instead (markdown headings, code \
             definitions, each with line number); then read the part you need with start_line and end_line. \
             Page max 300 lines; next_line present: more to read, call again with start_line = next_line. \
             Several files: several calls. Read-only.",
            json!({
                "path": string("Relative to vault root, no leading slash. E.g. \"projects/ideas.md\"."),
                "start_line": integer("First line to read, 1-based. Omit for whole file or outline."),
                "end_line": integer("Last line to read, inclusive. Omit = to end of file or page cap."),
            }),
            &["path"],
        ),
        decl(
            "list_directory",
            "List one directory of user's notes vault. Directory names end with \"/\". Hidden entries skipped. \
             Max 200 entries; truncated=true: more exist, use search_regexp. Read-only.",
            json!({"path": string("Directory relative to vault root, no leading slash. \"\" = vault root.")}),
            &["path"],
        ),
        decl(
            "search_regexp",
            "Regex search over content of all files in user's notes vault. Use to find notes on a topic when path \
             unknown. Max 50 files; each: path + first matching line. truncated=true: more files matched, narrow \
             the pattern. Read-only.",
            json!({"pattern": string("Rust regex syntax. Prefix (?i) = ignore case.")}),
            &["pattern"],
        ),
        decl(
            "projects",
            "List projects a coding agent can work in: name + directory. Call before delegate when project name \
             unsure.",
            json!({}),
            &[],
        ),
        decl(
            "sessions",
            "Coding agent session per project, with status. idle: ready for a task. busy: working now. waiting: \
             stuck on a permission prompt, user must answer it in the Claude app. dead: not running, delegate \
             starts it. Use to answer whether an agent still works.",
            json!({}),
            &[],
        ),
        decl(
            "tasks",
            "Tasks handed to coding agents with delegate: every unfinished one + the last few finished, newest \
             first. state: queued, spawning, sent = not started yet. working: agent on it now, last_activity = \
             what it does. waiting: stuck on a permission prompt. done: answer holds what the agent said. \
             failed: error holds why. Use when user asks how a task goes or what an agent answered.",
            json!({"project": string("Only this project's tasks. Omit = all projects.")}),
            &[],
        ),
        decl(
            "rename_thread",
            "Rename current conversation thread. Use when user asks, or topic became clear.",
            json!({"name": string("New title. Short, max 80 chars.")}),
            &["name"],
        ),
        decl(
            "switch_thread",
            "Move the user to another conversation thread. No thread: makes a new one and moves there; use when \
             user asks for a new thread or conversation, or starts a clearly separate topic and wants it apart. \
             thread: open that existing one. Voice restarts in the new thread with its own context: say nothing \
             after the call. Not for renaming: rename_thread.",
            json!({
                "thread": string("Existing thread to open: id, or part of its name. Omit = new thread."),
                "name": string("New thread only: short title. Omit = \"New thread\"."),
                "project": string("New thread only: its active project, exact name from the projects tool."),
            }),
            &[],
        ),
        decl(
            "memory_note",
            "Pin one note so it is never forgotten. Call when user says remember, note, keep in mind, do not \
             forget; or sets a standing rule (then call without mentioning it). scope: thread = this conversation \
             (default, use when unsure); always = user says always, everywhere, in every conversation, from now \
             on; project = rule or fact of the active project. One short keyword line per note, in user's \
             language. Changed rule: pass replaces. Not for ordinary decisions, goals, questions or chit-chat. \
             Result has warning: tell user once that this memory is nearly full, offer to clean up. error \
             \"memory full\": tell user, offer to drop notes from the pins listed.",
            json!({
                "text": string("The note, one short line. E.g. \"budget max 200 EUR\"."),
                "scope": string("thread (default), project or always."),
                "replaces": string("Part of the old note this one replaces, same scope. Omit for a new note."),
            }),
            &["text"],
        ),
        decl(
            "memory_forget",
            "Remove one pinned note. Call when user says forget, drop, or that a pinned note is no longer true. \
             Searches this conversation's notes, then the project's, then the always notes. Result names the \
             note removed: tell user. Error with matches: several notes fit, ask user which or retry with a \
             longer part. Only pinned notes can be removed.",
            json!({"match": string("Part of the note to remove, case ignored. Enough words to hit one note.")}),
            &["match"],
        ),
        decl(
            "memory_read",
            "Read what is remembered elsewhere. Call when user refers to another conversation or project, e.g. \
             \"what did we say about the garden\". No argument: list of other conversations that hold notes \
             or a memory + projects that have notes. thread: pinned notes and memory of that conversation. \
             project: notes file of that project. Notes of this conversation and its active project are already \
             in your context. Read-only.",
            json!({
                "thread": string("Conversation id, or part of its name."),
                "project": string("Project name, exact."),
            }),
            &[],
        ),
        decl(
            "set_project",
            "Set the project this conversation is about. Call when user says what project they work on, or \
             switches to another. Its notes then come with every later turn. Project names: projects tool.",
            json!({"project": string("Project name, exact. \"\" = no project.")}),
            &["project"],
        ),
        decl(
            "delegate",
            "Hand a task to the project's coding agent. Returns at once; agent's answer arrives later and is read \
             out; progress shows on user's screen, see it with tasks. Only way to change anything: create or edit notes, write code, run commands. You cannot write \
             files. After the call: tell user the task was handed over. Never invent a result.",
            json!({
                "project": string("Project name, exact, from the projects tool."),
                "prompt": string(
                    "Complete task, plain language. Agent does not hear this conversation: include every detail."
                ),
            }),
            &["project", "prompt"],
        ),
        decl(
            "web_search",
            "Search the internet with Google. Use for anything not in user's notes vault: news, current events, \
             weather, prices, facts you are unsure of. Returns text = answer built from the found pages, sources = \
             those pages. Slow, a few seconds. Do not read out URLs unless asked.",
            json!({"query": string("What to look up, full question or keywords. Include place and date when they matter.")}),
            &["query"],
        ),
        decl(
            "end_conversation",
            "End the voice conversation: mic and speaker go off at once. Use when user says goodbye or wants to \
             stop talking. Say nothing after the call, nobody hears it.",
            json!({}),
            &[],
        ),
    ]
}

/// Run a tool. Failures come back as `{error}` so the model can react to them.
/// `thread_id` = the conversation the call came from (`tool.call.thread_id`).
pub async fn call(app: &Arc<App>, name: &str, args: &Value, thread_id: Option<&str>) -> Value {
    run(app, name, args, thread_id)
        .await
        .unwrap_or_else(|e| json!({ "error": e }))
}

fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args[key]
        .as_str()
        .ok_or_else(|| format!("missing string argument {key:?}"))
}

/// Optional line number. Voice models also send numbers as floats or strings.
fn line(args: &Value, key: &str) -> Result<Option<i64>, String> {
    let v = &args[key];
    if v.is_null() {
        return Ok(None);
    }
    v.as_i64()
        .or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .map(Some)
        .ok_or_else(|| format!("{key} must be a whole number"))
}

/// Optional string argument. Voice models also send `""` or `null` for "none".
fn optional(args: &Value, key: &str) -> Option<String> {
    args[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

/// Memory tools: sqlite and one small file, blocking. Off the async workers.
async fn memory<F>(app: &Arc<App>, f: F) -> Result<Value, String>
where
    F: FnOnce(&App) -> Result<Value, String> + Send + 'static,
{
    let app = app.clone();
    tokio::task::spawn_blocking(move || f(&app))
        .await
        .map_err(|e| format!("tool crashed: {e}"))?
}

/// Vault work is blocking file I/O: keep it off the async workers.
async fn blocking<F>(app: &Arc<App>, f: F) -> Result<Value, String>
where
    F: FnOnce(&crate::vault::Vault) -> Result<Value, String> + Send + 'static,
{
    let app = app.clone();
    tokio::task::spawn_blocking(move || f(&app.vault))
        .await
        .map_err(|e| format!("tool crashed: {e}"))?
}

async fn run(
    app: &Arc<App>,
    name: &str,
    args: &Value,
    thread_id: Option<&str>,
) -> Result<Value, String> {
    match name {
        "read_file" => {
            let path = text(args, "path")?.to_string();
            let (start, end) = (line(args, "start_line")?, line(args, "end_line")?);
            blocking(app, move |v| v.read(&path, start, end)).await
        }
        "list_directory" => {
            let path = args["path"].as_str().unwrap_or("").to_string();
            blocking(app, move |v| v.list_directory(&path)).await
        }
        "search_regexp" => {
            let pattern = text(args, "pattern")?.to_string();
            blocking(app, move |v| v.search_regexp(&pattern)).await
        }
        "projects" => {
            let projects: Vec<Value> = app
                .cfg
                .projects
                .iter()
                .map(|(name, p)| json!({"name": name, "dir": p.dir}))
                .collect();
            Ok(json!({ "projects": projects }))
        }
        "sessions" => Ok(json!({"sessions": app.sessions().await})),
        "rename_thread" => {
            let id = thread_id.ok_or("no thread_id in tool.call: no thread to rename")?;
            crate::server::rename_thread(app, id, text(args, "name")?)
        }
        "switch_thread" => {
            let (wanted, name, project) = (
                optional(args, "thread"),
                optional(args, "name"),
                optional(args, "project"),
            );
            crate::memory::switch_thread(
                app,
                thread_id,
                wanted.as_deref(),
                name.as_deref(),
                project.as_deref(),
            )
        }
        "memory_note" => {
            let text = text(args, "text")?.to_string();
            let (scope, replaces) = (optional(args, "scope"), optional(args, "replaces"));
            let thread = thread_id.map(str::to_string);
            memory(app, move |app| {
                let (scope, replaces) = (scope.as_deref(), replaces.as_deref());
                crate::memory::note(app, thread.as_deref(), &text, scope, replaces)
            })
            .await
        }
        "memory_forget" => {
            let needle = text(args, "match")?.to_string();
            let thread = thread_id.map(str::to_string);
            memory(app, move |app| {
                crate::memory::forget(app, thread.as_deref(), &needle)
            })
            .await
        }
        "memory_read" => {
            let (wanted, project) = (optional(args, "thread"), optional(args, "project"));
            let caller = thread_id.map(str::to_string);
            memory(app, move |app| {
                crate::memory::read(
                    app,
                    caller.as_deref(),
                    wanted.as_deref(),
                    project.as_deref(),
                )
            })
            .await
        }
        "set_project" => {
            let id =
                thread_id.ok_or("no thread_id in tool.call: no thread to set the project of")?;
            crate::memory::set_project(app, id, text(args, "project")?)
        }
        "tasks" => Ok(json!({"tasks": app.tasks.recent(args["project"].as_str())})),
        "delegate" => crate::agents::delegate(
            app,
            text(args, "project")?,
            text(args, "prompt")?,
            thread_id,
        ),
        "web_search" => crate::search::search(app, text(args, "query")?).await,
        "end_conversation" => Ok(json!({"status": "conversation_ended"})),
        other => Err(format!("unknown tool {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_the_contract_tools() {
        let decls = declarations();
        let names: Vec<&str> = decls.iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "read_file",
                "list_directory",
                "search_regexp",
                "projects",
                "sessions",
                "tasks",
                "rename_thread",
                "switch_thread",
                "memory_note",
                "memory_forget",
                "memory_read",
                "set_project",
                "delegate",
                "web_search",
                "end_conversation"
            ]
        );
        for d in &decls {
            assert!(!d["description"].as_str().unwrap().is_empty());
            if d.get("parameters").is_none() {
                continue;
            }
            assert_eq!(d["parameters"]["type"], "OBJECT");
            assert!(!d["parameters"]["properties"]
                .as_object()
                .unwrap()
                .is_empty());
            for r in d["parameters"]["required"].as_array().unwrap() {
                assert!(d["parameters"]["properties"][r.as_str().unwrap()].is_object());
            }
        }
    }
}
