//! Pins and project memory (docs/protocol.md "Pins", "Project memory"): short
//! lines the model stores by tool, never made or changed by a compact. Scopes
//! `always` and `thread` live in sqlite, `project` in the `.taskbot.md` of the
//! project. No model call anywhere in here.

use crate::now_ms;
use crate::project_file::{self, Edit, PINNED};
use crate::server::App;
use crate::state::{self, Pin, Thread};
use crate::tasks::{State, Task};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// One pin. A guard against a runaway call, not a budget.
pub const PIN_MAX_CHARS: usize = 300;
/// From this share of its limit on, a scope asks to be cleaned up.
pub const WARN_PERCENT: usize = 80;
/// Most threads a `memory_read` without arguments lists.
pub const READ_THREADS_MAX: usize = 50;

/// One read-decide-write of the sqlite pins at a time.
static PIN_WRITES: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scope {
    Thread,
    Project,
    Always,
}

impl Scope {
    pub fn name(self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::Project => "project",
            Self::Always => "always",
        }
    }

    /// Upper limit of the scope, chars as it goes into the prompt.
    pub fn limit(self) -> usize {
        match self {
            Self::Thread => 1500,
            Self::Project => 3000,
            Self::Always => 1200,
        }
    }

    fn parse(raw: Option<&str>) -> Result<Self, String> {
        match raw.map(str::trim) {
            None | Some("" | "thread") => Ok(Self::Thread),
            Some("project") => Ok(Self::Project),
            Some("always") => Ok(Self::Always),
            Some(other) => Err(format!(
                "unknown scope {other:?}; use thread, project or always"
            )),
        }
    }
}

/// Clean a pin: one line, control chars dropped, trimmed, cut to
/// [`PIN_MAX_CHARS`] chars. Nothing left is an error.
pub fn pin_text(raw: &str) -> Result<String, String> {
    let line: String = raw
        .replace("\r\n", "\n")
        .chars()
        .map(|c| if matches!(c, '\n' | '\r') { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    let text: String = line.trim().chars().take(PIN_MAX_CHARS).collect();
    let text = text.trim_end();
    if text.is_empty() {
        return Err("note text is empty".into());
    }
    Ok(text.to_string())
}

/// Chars of a pin list as it goes into the prompt: one line per pin.
pub fn size<S: AsRef<str>>(texts: &[S]) -> usize {
    texts.iter().map(|t| t.as_ref().chars().count() + 1).sum()
}

/// The line a scope at or over its warn mark carries.
pub fn warning(scope: Scope, size: usize) -> Option<String> {
    let limit = scope.limit();
    (size * 100 >= limit * WARN_PERCENT).then(|| {
        format!(
            "{} memory {}% full: tell the user once, offer to clean it up",
            scope.name(),
            size * 100 / limit
        )
    })
}

/// Numbers of the pins that contain `needle`, case-insensitive.
pub fn find<S: AsRef<str>>(texts: &[S], needle: &str) -> Vec<usize> {
    let needle = needle.to_lowercase();
    (0..texts.len())
        .filter(|&i| texts[i].as_ref().to_lowercase().contains(&needle))
        .collect()
}

/// What a `memory_note` does to the list of its scope.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    /// The text is there already.
    Keep,
    Add,
    Swap(usize),
    /// `replaces` hit a pin, and the new text is another pin already.
    Remove(usize),
}

/// Decide a `memory_note`. `Err` = `replaces` hit more than one pin: their numbers.
pub fn plan(texts: &[String], text: &str, replaces: Option<&str>) -> Result<Change, Vec<usize>> {
    let same = |i: usize| texts[i].to_lowercase() == text.to_lowercase();
    let hits = replaces.map(|r| find(texts, r)).unwrap_or_default();
    match hits[..] {
        // No hit: plain add.
        [] if (0..texts.len()).any(same) => Ok(Change::Keep),
        [] => Ok(Change::Add),
        [hit] if texts[hit] == text => Ok(Change::Keep),
        [hit] if (0..texts.len()).any(|i| i != hit && same(i)) => Ok(Change::Remove(hit)),
        [hit] => Ok(Change::Swap(hit)),
        _ => Err(hits),
    }
}

fn apply(texts: &mut Vec<String>, change: Change, text: &str) {
    match change {
        Change::Keep => {}
        Change::Add => texts.push(text.to_string()),
        Change::Swap(i) => texts[i] = text.to_string(),
        Change::Remove(i) => {
            texts.remove(i);
        }
    }
}

fn db(e: rusqlite::Error) -> String {
    tracing::error!("state db: {e}");
    "state db error".into()
}

fn no_thread(id: &str) -> String {
    format!("unknown thread {id:?}")
}

fn unknown_project(app: &App, project: &str) -> String {
    let known: Vec<&str> = app.cfg.projects.keys().map(String::as_str).collect();
    format!("unknown project {project:?}; known projects: {known:?}")
}

fn many(texts: &[String], hits: &[usize]) -> Value {
    let matches: Vec<&str> = hits.iter().map(|&i| texts[i].as_str()).collect();
    json!({"error": format!("{} notes match", hits.len()), "matches": matches})
}

/// A refused write: the scope as it stands, plus the reason.
fn full(scope: Scope, mut pins: Value) -> Value {
    pins["error"] = json!(format!("{} memory full", scope.name()));
    pins
}

// ----------------------------------------------------------------- payloads

/// `Pins` = `{pins, size, limit}` of a sqlite scope.
fn pins_payload(pins: &[Pin], scope: Scope) -> Value {
    let texts: Vec<&str> = pins.iter().map(|p| p.text.as_str()).collect();
    json!({"pins": pins, "size": size(&texts), "limit": scope.limit()})
}

/// `Pins` of the `always` scope.
pub fn always_pins(app: &App) -> Result<Value, String> {
    Ok(pins_payload(
        &app.store.pins(None).map_err(db)?,
        Scope::Always,
    ))
}

/// `Pins` of a thread.
pub fn thread_pins(app: &App, id: &str) -> Result<Value, String> {
    Ok(pins_payload(
        &app.store.pins(Some(id)).map_err(db)?,
        Scope::Thread,
    ))
}

/// `{memory?, usage, pins}` of a thread: what `threads.compact.result` and,
/// with `thread_id` added, the `threads.memory` push carry.
pub fn thread_payload(app: &App, id: &str) -> Result<Value, String> {
    let (memory, usage) = app
        .store
        .thread_memory(id)
        .map_err(db)?
        .ok_or_else(|| no_thread(id))?;
    let mut payload = crate::compact::payload(memory, usage);
    payload["pins"] = thread_pins(app, id)?;
    Ok(payload)
}

/// `ProjectMemory` from the text of the file.
fn project_payload(project: &str, text: &str) -> Value {
    let limit = Scope::Project.limit();
    json!({
        "project": project,
        "text": project_file::cut(text, limit),
        "size": text.chars().count(),
        "limit": limit,
    })
}

/// `ProjectMemory` of a project. `None` = not configured, or no usable file.
pub fn project_memory(app: &App, project: &str) -> Option<Value> {
    let dir = &app.cfg.projects.get(project)?.dir;
    Some(project_payload(project, &project_file::load(dir)?))
}

/// The `## Pinned` lines of the file as a flat `{pins, size, limit}`. The
/// file holds no time per line: every pin carries the time of its last write.
fn project_pins(dir: &std::path::Path, text: &str) -> Value {
    let at = project_file::modified_ms(dir);
    let pins: Vec<Value> = project_file::bullets(text, PINNED)
        .into_iter()
        .map(|text| json!({"text": text, "at": at}))
        .collect();
    json!({"pins": pins, "size": text.chars().count(), "limit": Scope::Project.limit()})
}

/// What the compact gets as read-only input: `always` pins, the project's
/// `## Pinned` lines, the thread's pins.
pub fn pinned_for_fold(app: &App, id: &str) -> Result<Vec<String>, String> {
    let texts = |pins: Vec<Pin>| pins.into_iter().map(|p| p.text);
    let mut lines: Vec<String> = texts(app.store.pins(None).map_err(db)?).collect();
    if let Ok((_, dir)) = active_project(app, Some(id)) {
        if let Some(text) = project_file::load(&dir) {
            lines.extend(project_file::bullets(&text, PINNED));
        }
    }
    lines.extend(texts(app.store.pins(Some(id)).map_err(db)?));
    Ok(lines)
}

// ----------------------------------------------------------- active project

/// The active project of the calling thread: name and folder.
fn active_project(app: &App, thread_id: Option<&str>) -> Result<(String, PathBuf), String> {
    let id = thread_id.ok_or("no thread_id in tool.call: no thread, no active project")?;
    let thread = app
        .store
        .thread_info(id)
        .map_err(db)?
        .ok_or_else(|| no_thread(id))?;
    let project = thread.project.ok_or("no active project")?;
    match app.cfg.projects.get(&project) {
        Some(p) => Ok((project, p.dir.clone())),
        None => Err(format!(
            "active project {project:?} is not in the server config"
        )),
    }
}

/// Set the active project of a thread and tell every socket. `""` = none.
/// Shared by `threads.set_project` and the `set_project` tool. Returns `{thread}`.
pub fn set_project(app: &App, id: &str, project: &str) -> Result<Value, String> {
    let project = project.trim();
    if !project.is_empty() && !app.cfg.projects.contains_key(project) {
        return Err(unknown_project(app, project));
    }
    let thread = app
        .store
        .set_thread_project(id, (!project.is_empty()).then_some(project))
        .map_err(db)?
        .ok_or_else(|| no_thread(id))?;
    let payload = json!({ "thread": thread });
    app.push("threads.changed", payload.clone());
    Ok(payload)
}

/// `delegate` from a thread: its project becomes the active one when the
/// thread has none. Never fails the call.
pub fn adopt_project(app: &App, thread_id: Option<&str>, project: &str) {
    let Some(id) = thread_id else { return };
    match app.store.adopt_thread_project(id, project) {
        Ok(Some(thread)) => app.push("threads.changed", json!({ "thread": thread })),
        Ok(None) => {}
        Err(e) => tracing::error!("state db: {e}"),
    }
}

/// A task that just turned `done` or `failed`: one line on top of `## Tasks`
/// of its project file, then `memory.project`. Runs off the caller's path; a
/// file that cannot be written is logged and never fails the task.
pub fn log_task(app: &Arc<App>, task: &Task) {
    let Some(dir) = app.cfg.projects.get(&task.project).map(|p| p.dir.clone()) else {
        return;
    };
    let date = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(task.updated_at)
        .unwrap_or_default()
        .format("%Y-%m-%d")
        .to_string();
    let (done, detail) = match task.state {
        State::Done => (true, task.answer.as_deref()),
        _ => (false, task.error.as_deref()),
    };
    let line = project_file::task_line(&date, &task.prompt, done, detail);
    let (app, project) = (app.clone(), task.project.clone());
    tokio::task::spawn_blocking(move || {
        let written = project_file::update(&dir, &project, |old| {
            Some(project_file::with_task(old, &line))
        });
        match written {
            Ok(Some(text)) => app.push("memory.project", project_payload(&project, &text)),
            Ok(None) => {}
            Err(e) => tracing::warn!(%project, "task not logged in the project file: {e}"),
        }
    });
}

// -------------------------------------------------------------- memory_note

/// The `memory_note` tool. `Ok` may be an `{error, ...}` object: refusals
/// that carry more than a message.
pub fn note(
    app: &App,
    thread_id: Option<&str>,
    text: &str,
    scope: Option<&str>,
    replaces: Option<&str>,
) -> Result<Value, String> {
    let text = pin_text(text)?;
    let replaces = replaces.map(str::trim).filter(|r| !r.is_empty());
    match Scope::parse(scope)? {
        Scope::Thread => {
            let id = thread_id.ok_or("no thread_id in tool.call: no thread to pin the note to")?;
            if app.store.thread_info(id).map_err(db)?.is_none() {
                return Err(no_thread(id));
            }
            note_stored(app, Scope::Thread, Some(id), &text, replaces)
        }
        Scope::Always => note_stored(app, Scope::Always, None, &text, replaces),
        Scope::Project => note_project(app, thread_id, &text, replaces),
    }
}

fn push_pins(app: &App, scope: Scope, thread: Option<&str>) -> Result<(), String> {
    match thread {
        Some(id) if scope == Scope::Thread => {
            let mut payload = thread_payload(app, id)?;
            payload["thread_id"] = json!(id);
            app.push("threads.memory", payload);
        }
        _ => app.push("memory.always", always_pins(app)?),
    }
    Ok(())
}

/// `memory_note` on a sqlite scope. `thread` = `None`: `always`.
fn note_stored(
    app: &App,
    scope: Scope,
    thread: Option<&str>,
    text: &str,
    replaces: Option<&str>,
) -> Result<Value, String> {
    let _one_at_a_time = PIN_WRITES.lock().unwrap_or_else(|e| e.into_inner());
    let pins = app.store.pins(thread).map_err(db)?;
    let mut texts: Vec<String> = pins.iter().map(|p| p.text.clone()).collect();
    let change = match plan(&texts, text, replaces) {
        Ok(change) => change,
        Err(hits) => return Ok(many(&texts, &hits)),
    };
    let before = size(&texts);
    apply(&mut texts, change, text);
    let after = size(&texts);
    // A change that does not grow the scope always works: it is the way out.
    if after > scope.limit() && after > before {
        return Ok(full(scope, pins_payload(&pins, scope)));
    }
    match change {
        Change::Keep => {}
        Change::Add => app.store.add_pin(thread, text, now_ms()).map_err(db)?,
        Change::Swap(i) => app.store.set_pin(pins[i].seq, text, now_ms()).map_err(db)?,
        Change::Remove(i) => app.store.delete_pin(pins[i].seq).map_err(db)?,
    }
    let mut result = pins_payload(&app.store.pins(thread).map_err(db)?, scope);
    result["scope"] = json!(scope.name());
    if let Some(warning) = warning(scope, after) {
        result["warning"] = json!(warning);
    }
    if change != Change::Keep {
        push_pins(app, scope, thread)?;
    }
    Ok(result)
}

/// `memory_note` with scope `project`: one line of `## Pinned` in the file.
fn note_project(
    app: &App,
    thread_id: Option<&str>,
    text: &str,
    replaces: Option<&str>,
) -> Result<Value, String> {
    let (project, dir) = active_project(app, thread_id)?;
    let limit = Scope::Project.limit();
    let mut refused = None;
    let mut wrote = false;
    let file = project_file::update(&dir, &project, |old| {
        let texts = project_file::bullets(old, PINNED);
        let new = match plan(&texts, text, replaces) {
            Ok(Change::Keep) => return None,
            Ok(Change::Add) => project_file::edit(old, PINNED, Edit::Add(text)),
            Ok(Change::Swap(i)) => project_file::edit(old, PINNED, Edit::Swap(i, text)),
            Ok(Change::Remove(i)) => project_file::edit(old, PINNED, Edit::Remove(i)),
            Err(hits) => {
                refused = Some(many(&texts, &hits));
                return None;
            }
        };
        // The limit counts the whole file, not only `## Pinned`.
        let (before, after) = (old.chars().count(), new.chars().count());
        if after > limit && after > before {
            refused = Some(full(Scope::Project, project_pins(&dir, old)));
            return None;
        }
        wrote = true;
        Some(new)
    })?;
    if let Some(refused) = refused {
        return Ok(refused);
    }
    let text = file.unwrap_or_default();
    let mut result = project_pins(&dir, &text);
    result["scope"] = json!(Scope::Project.name());
    if let Some(warning) = warning(Scope::Project, text.chars().count()) {
        result["warning"] = json!(warning);
    }
    if wrote {
        app.push("memory.project", project_payload(&project, &text));
    }
    Ok(result)
}

// ------------------------------------------------------------ memory_forget

/// The `memory_forget` tool: thread pins, then project pins, then `always`.
/// The first scope with a hit decides. Pins only: `## State`, `## Recent`
/// and `## Tasks` are out of reach.
pub fn forget(app: &App, thread_id: Option<&str>, needle: &str) -> Result<Value, String> {
    let needle = needle.trim();
    if needle.is_empty() {
        return Err("match is empty".into());
    }
    // Every pin looked at, for the "no hit" answer.
    let mut seen: Vec<Value> = Vec::new();
    if let Some(id) = thread_id {
        if let Some(done) = forget_stored(app, Scope::Thread, Some(id), needle, &mut seen)? {
            return Ok(done);
        }
        if let Ok((project, dir)) = active_project(app, Some(id)) {
            if let Some(done) = forget_project(app, &project, &dir, needle, &mut seen) {
                return Ok(done);
            }
        }
    }
    if let Some(done) = forget_stored(app, Scope::Always, None, needle, &mut seen)? {
        return Ok(done);
    }
    Ok(json!({"error": "no pinned note matches", "pins": seen}))
}

/// `None` = no hit in this scope.
fn forget_stored(
    app: &App,
    scope: Scope,
    thread: Option<&str>,
    needle: &str,
    seen: &mut Vec<Value>,
) -> Result<Option<Value>, String> {
    let _one_at_a_time = PIN_WRITES.lock().unwrap_or_else(|e| e.into_inner());
    let pins = app.store.pins(thread).map_err(db)?;
    let texts: Vec<String> = pins.iter().map(|p| p.text.clone()).collect();
    let hit = match find(&texts, needle)[..] {
        [] => {
            seen.extend(pins.iter().map(|p| json!(p)));
            return Ok(None);
        }
        [hit] => hit,
        ref hits => return Ok(Some(many(&texts, hits))),
    };
    app.store.delete_pin(pins[hit].seq).map_err(db)?;
    let mut result = pins_payload(&app.store.pins(thread).map_err(db)?, scope);
    result["forgotten"] = json!(texts[hit]);
    result["from"] = json!(scope.name());
    push_pins(app, scope, thread)?;
    Ok(Some(result))
}

/// `None` = no hit in the project file, or no usable file.
fn forget_project(
    app: &App,
    project: &str,
    dir: &std::path::Path,
    needle: &str,
    seen: &mut Vec<Value>,
) -> Option<Value> {
    let mut refused = None;
    let mut forgotten = None;
    let file = project_file::update(dir, project, |old| {
        let texts = project_file::bullets(old, PINNED);
        match find(&texts, needle)[..] {
            [] => {
                if let Some(pins) = project_pins(dir, old)["pins"].as_array() {
                    seen.extend(pins.iter().cloned());
                }
                None
            }
            [hit] => {
                forgotten = Some(texts[hit].clone());
                Some(project_file::edit(old, PINNED, Edit::Remove(hit)))
            }
            ref hits => {
                refused = Some(many(&texts, hits));
                None
            }
        }
    });
    let text = match file {
        Ok(text) => text.unwrap_or_default(),
        Err(e) => {
            tracing::warn!(%project, "project memory skipped: {e}");
            return None;
        }
    };
    if refused.is_some() {
        return refused;
    }
    let forgotten = forgotten?;
    let mut result = project_pins(dir, &text);
    result["forgotten"] = json!(forgotten);
    result["from"] = json!(Scope::Project.name());
    app.push("memory.project", project_payload(project, &text));
    Some(result)
}

// -------------------------------------------------------------- memory_read

/// The `memory_read` tool. Reads only. `caller` = the thread the call came from.
pub fn read(
    app: &App,
    caller: Option<&str>,
    thread: Option<&str>,
    project: Option<&str>,
) -> Result<Value, String> {
    fn given(arg: Option<&str>) -> Option<&str> {
        arg.map(str::trim).filter(|a| !a.is_empty())
    }
    if let Some(wanted) = given(thread) {
        return read_thread(app, wanted);
    }
    if let Some(name) = given(project) {
        let proj = app
            .cfg
            .projects
            .get(name)
            .ok_or_else(|| unknown_project(app, name))?;
        let text = project_file::load(&proj.dir)
            .ok_or_else(|| format!("project {name:?} has no {}", project_file::FILE_NAME))?;
        let text = project_file::cut(&text, Scope::Project.limit());
        return Ok(json!({"project": name, "text": text}));
    }
    let threads: Vec<Value> = app
        .store
        .remembered(caller, READ_THREADS_MAX)
        .map_err(db)?
        .into_iter()
        .map(|r| {
            let mut v = json!({"id": r.thread.id, "name": r.thread.name, "pins": r.pins, "memory": r.memory});
            if let Some(project) = r.thread.project {
                v["project"] = json!(project);
            }
            v
        })
        .collect();
    let projects: Vec<&str> = app
        .cfg
        .projects
        .iter()
        .filter(|(_, p)| project_file::path(&p.dir).is_file())
        .map(|(name, _)| name.as_str())
        .collect();
    Ok(json!({"threads": threads, "projects": projects}))
}

/// `wanted` = a thread id, or part of a thread name. One hit, else an error
/// that names the matches.
fn read_thread(app: &App, wanted: &str) -> Result<Value, String> {
    let thread = find_thread(app, wanted)?;
    let pins = app.store.pins(Some(&thread.id)).map_err(db)?;
    let mut result = json!({"thread": {"id": thread.id, "name": thread.name}, "pins": pins});
    if let Some((Some(memory), _)) = app.store.thread_memory(&thread.id).map_err(db)? {
        result["memory"] = json!(memory.text);
    }
    Ok(result)
}

/// A thread by id, else the one thread whose name contains `wanted`, case ignored.
fn find_thread(app: &App, wanted: &str) -> Result<Thread, String> {
    Ok(match app.store.thread_info(wanted).map_err(db)? {
        Some(thread) => thread,
        None => {
            let needle = wanted.to_lowercase();
            let mut hits: Vec<_> = app
                .store
                .threads()
                .map_err(db)?
                .into_iter()
                .filter(|t| t.name.to_lowercase().contains(&needle))
                .collect();
            if hits.len() != 1 {
                let names: Vec<&str> = hits.iter().map(|t| t.name.as_str()).collect();
                return Err(match names.len() {
                    0 => format!("no thread matches {wanted:?}"),
                    n => format!("{n} threads match {wanted:?}: {names:?}"),
                });
            }
            hits.remove(0)
        }
    })
}

/// The `switch_thread` tool. `caller` = the thread the call came from. Finds
/// `wanted`, or makes a new thread; the app opens what comes back.
pub fn switch_thread(
    app: &App,
    caller: Option<&str>,
    wanted: Option<&str>,
    name: Option<&str>,
    project: Option<&str>,
) -> Result<Value, String> {
    if let Some(wanted) = wanted {
        let thread = find_thread(app, wanted)?;
        if caller == Some(thread.id.as_str()) {
            return Err(format!("already in thread {:?}", thread.name));
        }
        return Ok(json!({"thread": thread, "created": false}));
    }
    // Checked before the create: a bad project leaves no orphan thread.
    if let Some(project) = project {
        if !app.cfg.projects.contains_key(project) {
            return Err(unknown_project(app, project));
        }
    }
    let name = match name {
        Some(raw) => state::thread_name(raw)?,
        None => state::THREAD_NAME_DEFAULT.to_string(),
    };
    let mut thread = app.store.create_thread(&name, now_ms()).map_err(db)?;
    if let Some(project) = project {
        thread = app
            .store
            .set_thread_project(&thread.id, Some(project))
            .map_err(db)?
            .ok_or_else(|| no_thread(&thread.id))?;
    }
    app.push("threads.changed", json!({ "thread": thread }));
    Ok(json!({"thread": thread, "created": true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_text_is_one_short_line() {
        assert_eq!(
            pin_text("  budget\r\nmax\n200\tEUR\u{0} ").unwrap(),
            "budget max 200EUR"
        );
        assert_eq!(pin_text("árvíztűrő").unwrap(), "árvíztűrő");
        let cut = pin_text(&"é".repeat(1000)).unwrap();
        assert_eq!(cut.chars().count(), PIN_MAX_CHARS);
        // The cut must not leave a trailing space behind.
        let cut = pin_text(&format!("{} tail", "x".repeat(299))).unwrap();
        assert_eq!(cut, "x".repeat(299));
        for empty in ["", "  ", "\n\r\n", "\u{0}"] {
            assert!(pin_text(empty).is_err(), "{empty:?}");
        }
    }

    #[test]
    fn size_and_warn_mark() {
        assert_eq!(size::<&str>(&[]), 0);
        assert_eq!(size(&["ab", "é"]), 5, "chars, one line end per pin");
        assert_eq!(warning(Scope::Always, 959), None);
        assert_eq!(
            warning(Scope::Always, 960).unwrap(),
            "always memory 80% full: tell the user once, offer to clean it up"
        );
        assert!(warning(Scope::Thread, 1199).is_none());
        assert!(warning(Scope::Thread, 1500)
            .unwrap()
            .starts_with("thread memory 100% full"));
        assert!(warning(Scope::Project, 3300)
            .unwrap()
            .starts_with("project memory 110% full"));
    }

    #[test]
    fn scope_names() {
        assert_eq!(Scope::parse(None), Ok(Scope::Thread));
        assert_eq!(Scope::parse(Some("")), Ok(Scope::Thread));
        assert_eq!(Scope::parse(Some(" always ")), Ok(Scope::Always));
        assert_eq!(Scope::parse(Some("project")), Ok(Scope::Project));
        assert!(Scope::parse(Some("global")).is_err());
    }

    #[test]
    fn find_is_a_case_blind_substring() {
        let texts = [
            "Budget max 200 EUR",
            "notes under beds/",
            "budget talk on Friday",
        ];
        assert_eq!(find(&texts, "BEDS"), [1]);
        assert_eq!(find(&texts, "budget"), [0, 2]);
        assert!(find(&texts, "pump").is_empty());
        assert_eq!(find(&["ÁRVÍZ"], "árvíz"), [0]);
    }

    #[test]
    fn plan_of_a_note() {
        let texts: Vec<String> = ["budget max 200 EUR", "notes under beds/", "drip, not spray"]
            .map(String::from)
            .to_vec();
        assert_eq!(plan(&texts, "water at dawn", None), Ok(Change::Add));
        assert_eq!(plan(&texts, "NOTES under beds/", None), Ok(Change::Keep));
        assert_eq!(
            plan(&texts, "budget max 300 EUR", Some("BUDGET")),
            Ok(Change::Swap(0))
        );
        // No hit: plain add. Unless the text is there already.
        assert_eq!(plan(&texts, "water at dawn", Some("pump")), Ok(Change::Add));
        assert_eq!(
            plan(&texts, "drip, not spray", Some("pump")),
            Ok(Change::Keep)
        );
        // Two hits: nothing happens, the caller names them.
        assert_eq!(plan(&texts, "x", Some("s")), Err(vec![1, 2]));
        // Swapping a pin into a copy of another one would leave it twice.
        assert_eq!(
            plan(&texts, "drip, not spray", Some("budget")),
            Ok(Change::Remove(0))
        );
        assert_eq!(
            plan(&texts, "budget max 200 EUR", Some("budget")),
            Ok(Change::Keep)
        );

        let mut list = texts.clone();
        apply(&mut list, Change::Swap(1), "notes under plots/");
        apply(&mut list, Change::Add, "new");
        apply(&mut list, Change::Remove(0), "");
        assert_eq!(list, ["notes under plots/", "drip, not spray", "new"]);
    }
}
