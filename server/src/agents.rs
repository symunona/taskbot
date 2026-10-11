//! Agent sessions: spawn through `<channel_dir>/spawn-session.sh`, prompt
//! through the session's channel shim, status from Claude Code's session files.

use crate::config::Config;
use crate::server::App;
use crate::tasks::{self, State};
use serde::Serialize;
use serde_json::Value;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

pub const SPAWN_TIMEOUT: Duration = Duration::from_secs(90);
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(10);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
/// `agent.event` kind `prompt` carries at most this much of the prompt.
const EVENT_PROMPT_CHARS: usize = 500;

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Session {
    pub project: String,
    pub tmux: String,
    pub status: &'static str,
    pub claude_session_id: Option<String>,
    pub url: Option<String>,
}

pub fn tmux_name(project: &str) -> String {
    format!("tb-{project}")
}

fn pid_alive(pid: i64) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 only checks that the process exists; nothing is delivered.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// One `Session` per configured project. No live Claude process named
/// `tb-<project>` means `dead`.
pub fn sessions(cfg: &Config) -> Vec<Session> {
    // Internal Claude Code files; tolerate anything unexpected in them.
    let mut live: Vec<Value> = Vec::new();
    if let Ok(dir) = std::fs::read_dir(cfg.sessions_dir()) {
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if v["pid"].as_i64().is_some_and(pid_alive) {
                live.push(v);
            }
        }
    }
    cfg.projects
        .keys()
        .map(|project| {
            let tmux = tmux_name(project);
            let file = live
                .iter()
                .filter(|v| v["name"].as_str() == Some(tmux.as_str()))
                .max_by_key(|v| v["updatedAt"].as_i64().unwrap_or(0));
            let text = |key: &str| {
                file.and_then(|v| v[key].as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            };
            Session {
                project: project.clone(),
                status: match file {
                    None => "dead",
                    // Registry entry exists before `status` does: still starting.
                    Some(v) => match v["status"].as_str() {
                        // "shell": no turn running, only a background command.
                        Some("idle" | "shell") => "idle",
                        Some("waiting") => "waiting",
                        _ => "busy",
                    },
                },
                claude_session_id: text("sessionId"),
                url: text("bridgeSessionId").map(|id| format!("https://claude.ai/code/{id}")),
                tmux,
            }
        })
        .collect()
}

/// Port of the project's channel shim: `channel_port_base` + its stable index.
pub fn shim_port(app: &App, project: &str) -> Result<u16, String> {
    let idx = app
        .store
        .port_index(project)
        .map_err(|e| format!("state db: {e}"))?;
    u16::try_from(idx)
        .ok()
        .and_then(|i| app.cfg.channel_port_base.checked_add(i))
        .ok_or_else(|| "channel_port_base + project index does not fit in a port number".into())
}

/// Validate, record a `Task`, then hand the work to a background job and
/// return at once. What happens afterwards is pushed as `tasks.changed` and
/// `agent.event`. `thread_id` = the conversation the call came from.
pub fn delegate(
    app: &Arc<App>,
    project: &str,
    prompt: &str,
    thread_id: Option<&str>,
) -> Result<Value, String> {
    let Some(proj) = app.cfg.projects.get(project) else {
        let known: Vec<&str> = app.cfg.projects.keys().map(String::as_str).collect();
        return Err(format!(
            "unknown project {project:?}; known projects: {known:?}"
        ));
    };
    if prompt.trim().is_empty() {
        return Err("prompt is empty".into());
    }
    let Some(channel_dir) = app.cfg.channel_dir.clone() else {
        return Err(
            "channel_dir is not set in the server config; cannot reach agent sessions".into(),
        );
    };
    let port = shim_port(app, project)?;
    let task = app
        .tasks
        .create(project, thread_id, prompt, crate::now_ms());
    tasks::push(app, vec![task.clone()]);

    let job = Job {
        app: app.clone(),
        task_id: task.id.clone(),
        project: project.to_string(),
        prompt: prompt.to_string(),
        script: channel_dir.join("spawn-session.sh"),
        dir: proj.dir.clone(),
        mode: proj.permission_mode.clone(),
        model: proj.model.clone(),
        port,
    };
    tokio::spawn(job.run());
    Ok(serde_json::json!({
        "status": "sent",
        "project": project,
        "task_id": task.id,
        "state": task.state,
        "queue_pos": task.queue_pos,
    }))
}

struct Job {
    app: Arc<App>,
    task_id: String,
    project: String,
    prompt: String,
    script: std::path::PathBuf,
    dir: std::path::PathBuf,
    mode: String,
    model: Option<String>,
    port: u16,
}

impl Job {
    async fn run(self) {
        // One job per project at a time: no double spawn, prompts stay in order.
        let lock = self.app.project_lock(&self.project);
        let _guard = lock.lock().await;
        if let Err(e) = self.deliver().await {
            tracing::warn!(project = %self.project, "delegate failed: {e}");
            let failed = self.app.tasks.fail(&self.task_id, &e, crate::now_ms());
            tasks::push(&self.app, failed);
            self.app
                .push_event(&self.project, "error", Value::String(e.clone()));
            // `delegate` already answered "sent", so a failure has to be spoken too.
            // Spoken: the reason only (last line), not the pane dump before it.
            let why = e.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(&e);
            self.app.push(
                "agent.say",
                serde_json::json!({
                    "project": self.project,
                    "text": format!("Could not reach the agent for project {}: {}", self.project, why.trim()),
                }),
            );
        }
    }

    fn advance(&self, state: State) {
        let changed = self
            .app
            .tasks
            .advance(&self.task_id, state, crate::now_ms());
        tasks::push(&self.app, changed);
    }

    async fn deliver(&self) -> Result<(), String> {
        let shim = format!("http://127.0.0.1:{}", self.port);
        let healthy = self
            .app
            .http
            .get(format!("{shim}/health"))
            .timeout(HEALTH_TIMEOUT)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
        if !healthy {
            self.advance(State::Spawning);
            self.spawn().await?;
            let detail = format!("{} on port {}", tmux_name(&self.project), self.port);
            self.app
                .push_event(&self.project, "spawned", Value::String(detail));
        }
        let resp = self
            .app
            .http
            .post(format!("{shim}/prompt"))
            .header("content-type", "text/plain; charset=utf-8")
            .header("x-task-id", &self.task_id)
            .body(self.prompt.clone())
            .timeout(PROMPT_TIMEOUT)
            .send()
            .await
            .map_err(|e| {
                format!(
                    "prompt not delivered to the shim on port {}: {e}",
                    self.port
                )
            })?;
        if !resp.status().is_success() {
            return Err(format!(
                "shim on port {} answered {} to the prompt",
                self.port,
                resp.status()
            ));
        }
        self.advance(State::Sent);
        // The phone only logs this: no need to echo a huge prompt to every socket.
        let mut shown: String = self.prompt.chars().take(EVENT_PROMPT_CHARS).collect();
        if shown.len() < self.prompt.len() {
            shown.push_str(" ...");
        }
        self.app
            .push_event(&self.project, "prompt", Value::String(shown));
        Ok(())
    }

    /// Exit 0 means the session exists and its shim answers `/health`.
    async fn spawn(&self) -> Result<(), String> {
        let mut cmd = tokio::process::Command::new(&self.script);
        if let Some(model) = &self.model {
            cmd.arg("--model").arg(model);
        }
        let child = cmd
            .arg("--name")
            .arg(tmux_name(&self.project))
            .arg("--project")
            .arg(&self.project)
            .arg("--dir")
            .arg(&self.dir)
            .arg("--port")
            .arg(self.port.to_string())
            .arg("--server")
            .arg(format!("http://127.0.0.1:{}", self.app.port))
            .arg("--permission-mode")
            .arg(&self.mode)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("cannot run {}: {e}", self.script.display()))?;
        let out = tokio::time::timeout(SPAWN_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| {
                format!(
                    "spawn-session.sh did not finish within {} s",
                    SPAWN_TIMEOUT.as_secs()
                )
            })?
            .map_err(|e| format!("spawn-session.sh: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: String = stderr
            .trim()
            .chars()
            .rev()
            .take(400)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        Err(format!("spawn-session.sh failed ({}): {tail}", out.status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_from_files() {
        let dir = tempfile::tempdir().unwrap();
        let me = std::process::id();
        let write = |file: &str, body: Value| {
            std::fs::write(dir.path().join(file), body.to_string()).unwrap()
        };
        write(
            "1.json",
            serde_json::json!({"pid": me, "name": "tb-live", "status": "busy", "sessionId": "s1", "bridgeSessionId": "session_abc"}),
        );
        write(
            "2.json",
            serde_json::json!({"pid": me, "name": "tb-nobridge", "status": "idle", "sessionId": "s2"}),
        );
        // pid 0x7ffffffe: far above any pid_max, so never alive.
        write(
            "3.json",
            serde_json::json!({"pid": 2147483646, "name": "tb-gone", "status": "idle", "sessionId": "s3"}),
        );
        write(
            "4.json",
            serde_json::json!({"pid": me, "name": "other", "status": "idle"}),
        );
        write(
            "6.json",
            serde_json::json!({"pid": me, "name": "tb-shell", "status": "shell"}),
        );
        std::fs::write(dir.path().join("5.json"), "not json").unwrap();
        std::fs::write(dir.path().join("1.abc.key"), "x").unwrap();

        let cfg = Config::parse(&format!(
            "vault_path='/v'\nclaude_sessions_dir='{}'\n[projects.live]\ndir='/a'\n[projects.nobridge]\ndir='/b'\n[projects.gone]\ndir='/c'\n[projects.never]\ndir='/d'\n[projects.shell]\ndir='/e'",
            dir.path().display()
        ))
        .unwrap();
        let by: std::collections::HashMap<_, _> = sessions(&cfg)
            .into_iter()
            .map(|s| (s.project.clone(), s))
            .collect();
        assert_eq!(by.len(), 5);
        assert_eq!(by["shell"].status, "idle");
        assert_eq!(by["live"].status, "busy");
        assert_eq!(by["live"].tmux, "tb-live");
        assert_eq!(by["live"].claude_session_id.as_deref(), Some("s1"));
        assert_eq!(
            by["live"].url.as_deref(),
            Some("https://claude.ai/code/session_abc")
        );
        assert_eq!(
            (by["nobridge"].status, &by["nobridge"].url),
            ("idle", &None)
        );
        assert_eq!(
            (by["gone"].status, &by["gone"].claude_session_id),
            ("dead", &None)
        );
        assert_eq!(by["never"].status, "dead");
    }
}
