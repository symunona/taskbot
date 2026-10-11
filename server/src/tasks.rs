//! Delegate tasks: one record per `delegate` call, every change pushed to the
//! app as `tasks.changed` (docs/protocol.md "Tasks"). Own model and store:
//! held in memory, mirrored to sqlite on state changes so a restart keeps them.

use crate::server::App;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the shim of every project with a delivered task is checked.
pub const WATCH_EVERY: Duration = Duration::from_secs(10);
/// Failed checks in a row before the session counts as gone.
const WATCH_MISSES: u8 = 2;
const WATCH_TIMEOUT: Duration = Duration::from_secs(2);
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Finished tasks older than this are dropped at start.
const KEEP_MS: i64 = 7 * DAY_MS;
/// Most tasks one `tasks.list` returns.
pub const LIST_MAX: usize = 100;
/// Finished tasks the `tasks` tool adds to the unfinished ones.
pub const RECENT_FINISHED: usize = 5;
pub const ACTIVITY_MAX: usize = 120;
const RESTARTED: &str = "server restarted before the prompt was delivered";
const SESSION_GONE: &str = "agent session is gone";

type Res<T> = Result<T, rusqlite::Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Queued,
    Spawning,
    Sent,
    Working,
    Waiting,
    Done,
    Failed,
}

impl State {
    pub fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }

    /// The shim holds the prompt: only the shim can finish the task now.
    fn delivered(self) -> bool {
        matches!(self, Self::Sent | Self::Working | Self::Waiting)
    }

    /// Delivery order. A task never moves to a lower rank.
    fn rank(self) -> u8 {
        match self {
            Self::Queued => 0,
            Self::Spawning => 1,
            Self::Sent => 2,
            Self::Working | Self::Waiting => 3,
            Self::Done | Self::Failed => 4,
        }
    }
}

/// One `delegate` call. Times are epoch milliseconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub project: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub prompt: String,
    pub state: State,
    /// Unfinished tasks of the project ahead of this one. Worked out on the
    /// way out, never stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_pos: Option<usize>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Task {
    fn set_state(&mut self, state: State, now: i64) {
        self.state = state;
        if state.rank() == State::Working.rank() && self.started_at.is_none() {
            self.started_at = Some(now);
        }
        if state != State::Working {
            self.last_activity = None;
        }
    }
}

struct Inner {
    /// Creation order.
    tasks: Vec<Task>,
    conn: Connection,
    /// Failed shim checks in a row, per project.
    misses: HashMap<String, u8>,
}

impl Inner {
    /// Task `i` as the app sees it: with `queue_pos`.
    fn view(&self, i: usize) -> Task {
        let mut task = self.tasks[i].clone();
        task.queue_pos = (task.state.rank() <= State::Sent.rank()).then(|| {
            self.tasks[..i]
                .iter()
                .filter(|t| t.project == task.project && !t.state.finished())
                .count()
        });
        task
    }

    /// Unfinished tasks of `project`, as views.
    fn open_views(&self, project: &str) -> Vec<Task> {
        (0..self.tasks.len())
            .filter(|&i| self.tasks[i].project == project && !self.tasks[i].state.finished())
            .map(|i| self.view(i))
            .collect()
    }

    fn save(&self, task: &Task) {
        let body = serde_json::to_string(task).unwrap_or_default();
        let saved = self.conn.execute(
            "INSERT INTO tasks(id, created_at, updated_at, finished, body) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET updated_at = ?3, finished = ?4, body = ?5",
            params![task.id, task.created_at, task.updated_at, task.state.finished(), body],
        );
        if let Err(e) = saved {
            tracing::error!(task = %task.id, "state db: {e}");
        }
    }
}

pub struct Tasks {
    inner: Mutex<Inner>,
}

impl Tasks {
    /// Open the table in the state db and load it. Tasks the old server never
    /// delivered fail here: nothing is left that would deliver them.
    pub fn open(path: &Path, now: i64) -> Res<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks(
                id TEXT PRIMARY KEY,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                finished INTEGER NOT NULL,
                body TEXT NOT NULL);",
        )?;
        conn.execute(
            "DELETE FROM tasks WHERE finished = 1 AND updated_at < ?1",
            params![now - KEEP_MS],
        )?;
        let bodies: Vec<String> = conn
            .prepare("SELECT body FROM tasks ORDER BY created_at, rowid")?
            .query_map([], |r| r.get(0))?
            .collect::<Res<_>>()?;
        let mut inner = Inner {
            tasks: bodies
                .iter()
                .filter_map(|b| serde_json::from_str(b).ok())
                .collect(),
            conn,
            misses: HashMap::new(),
        };
        for i in 0..inner.tasks.len() {
            let task = &mut inner.tasks[i];
            task.queue_pos = None;
            if task.state.finished() || task.state.delivered() {
                continue;
            }
            task.set_state(State::Failed, now);
            task.error = Some(RESTARTED.into());
            task.updated_at = now;
            inner.save(&inner.tasks[i]);
        }
        Ok(Self {
            inner: Mutex::new(inner),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new `queued` task.
    pub fn create(&self, project: &str, thread_id: Option<&str>, prompt: &str, now: i64) -> Task {
        let mut inner = self.lock();
        inner.tasks.push(Task {
            id: hex::encode(crate::random_bytes::<8>()),
            project: project.to_string(),
            thread_id: thread_id.map(str::to_string),
            prompt: prompt.to_string(),
            state: State::Queued,
            queue_pos: None,
            created_at: now,
            started_at: None,
            updated_at: now,
            last_activity: None,
            answer: None,
            error: None,
        });
        let i = inner.tasks.len() - 1;
        inner.save(&inner.tasks[i]);
        inner.view(i)
    }

    /// Run `change` over every unfinished task of `project`. Returns the views
    /// that differ afterwards, `queue_pos` shifts of bystanders included:
    /// exactly what has to be pushed.
    fn mutate(&self, project: &str, now: i64, mut change: impl FnMut(&mut Task)) -> Vec<Task> {
        let mut inner = self.lock();
        let before = inner.open_views(project);
        let mut stored = Vec::new();
        for (i, task) in inner.tasks.iter_mut().enumerate() {
            if task.project != project || task.state.finished() {
                continue;
            }
            let old = task.clone();
            change(task);
            if *task == old {
                continue;
            }
            task.updated_at = now;
            // `last_activity` alone is not worth a disk write.
            if (task.state, &task.answer) != (old.state, &old.answer) {
                stored.push(i);
            }
        }
        for i in stored {
            inner.save(&inner.tasks[i]);
        }
        before
            .into_iter()
            .filter_map(|old| {
                let i = inner.tasks.iter().position(|t| t.id == old.id)?;
                let new = inner.view(i);
                (new != old).then_some(new)
            })
            .collect()
    }

    fn project_of(&self, id: &str) -> Option<String> {
        let inner = self.lock();
        let task = inner.tasks.iter().find(|t| t.id == id)?;
        Some(task.project.clone())
    }

    /// Delivery progress (`spawning`, `sent`). Never moves a task backwards:
    /// the shim's `start` can overtake the answer to the prompt POST.
    pub fn advance(&self, id: &str, state: State, now: i64) -> Vec<Task> {
        let Some(project) = self.project_of(id) else {
            return Vec::new();
        };
        self.mutate(&project, now, |t| {
            if t.id == id && state.rank() > t.state.rank() {
                t.set_state(state, now);
            }
        })
    }

    pub fn fail(&self, id: &str, error: &str, now: i64) -> Vec<Task> {
        let Some(project) = self.project_of(id) else {
            return Vec::new();
        };
        self.mutate(&project, now, |t| {
            if t.id == id {
                t.set_state(State::Failed, now);
                t.error = Some(error.to_string());
            }
        })
    }

    /// A shim event (`/internal/agent/event`). `ids` = the tasks of the turn;
    /// `None` (old shim) = every delivered task of the project.
    pub fn event(
        &self,
        project: &str,
        ids: Option<&[String]>,
        kind: &str,
        detail: &str,
        now: i64,
    ) -> Vec<Task> {
        let state = match kind {
            "start" | "resume" | "activity" => State::Working,
            "waiting" => State::Waiting,
            "stop" => State::Done,
            _ => return Vec::new(),
        };
        let activity: String = detail.trim().chars().take(ACTIVITY_MAX).collect();
        self.mutate(project, now, |t| {
            let hit = match ids {
                Some(ids) => ids.contains(&t.id),
                None => t.state.delivered(),
            };
            if !hit {
                return;
            }
            if t.state != state {
                t.set_state(state, now);
            }
            if kind == "activity" && !activity.is_empty() {
                t.last_activity = Some(activity.clone());
            }
        })
    }

    /// What the agent said for the tasks of its turn (`/internal/agent/say`).
    pub fn say(&self, project: &str, ids: &[String], text: &str, now: i64) -> Vec<Task> {
        self.mutate(project, now, |t| {
            if ids.contains(&t.id) {
                t.answer = Some(text.to_string());
            }
        })
    }

    fn pick(&self, keep: impl Fn(&Task) -> bool, max: usize) -> Vec<Task> {
        let inner = self.lock();
        (0..inner.tasks.len())
            .rev()
            .filter(|&i| keep(&inner.tasks[i]))
            .take(max)
            .map(|i| inner.view(i))
            .collect()
    }

    /// `tasks.list`: unfinished tasks and the last day's finished ones, newest first.
    pub fn list(&self, now: i64) -> Vec<Task> {
        self.pick(
            |t| !t.state.finished() || t.updated_at >= now - DAY_MS,
            LIST_MAX,
        )
    }

    /// The `tasks` tool: unfinished tasks and the few last finished ones, newest first.
    pub fn recent(&self, project: Option<&str>) -> Vec<Task> {
        let wanted = |t: &Task| project.is_none_or(|p| t.project == p);
        let mut finished = self.pick(|t| wanted(t) && t.state.finished(), RECENT_FINISHED);
        let mut tasks = self.pick(|t| wanted(t) && !t.state.finished(), usize::MAX);
        tasks.append(&mut finished);
        tasks.sort_by_key(|t| std::cmp::Reverse(t.created_at));
        tasks
    }

    /// Projects whose shim holds a task.
    fn watched(&self) -> Vec<String> {
        let mut inner = self.lock();
        let mut projects: Vec<String> = inner
            .tasks
            .iter()
            .filter(|t| t.state.delivered())
            .map(|t| t.project.clone())
            .collect();
        projects.sort();
        projects.dedup();
        inner.misses.retain(|p, _| projects.contains(p));
        projects
    }

    /// Count one shim check. `true` = the session counts as gone now.
    fn missed(&self, project: &str, alive: bool) -> bool {
        let mut inner = self.lock();
        if alive {
            inner.misses.remove(project);
            return false;
        }
        let n = inner.misses.entry(project.to_string()).or_default();
        *n += 1;
        *n >= WATCH_MISSES
    }

    fn fail_delivered(&self, project: &str, now: i64) -> Vec<Task> {
        self.lock().misses.remove(project);
        self.mutate(project, now, |t| {
            if t.state.delivered() {
                t.set_state(State::Failed, now);
                t.error = Some(SESSION_GONE.into());
            }
        })
    }
}

/// Tell every socket about changed tasks. A finished task in `changed` has
/// just finished (final states never change again): it gets its line in the
/// project file here.
pub fn push(app: &Arc<App>, changed: Vec<Task>) {
    for task in changed {
        if task.state.finished() {
            crate::memory::log_task(app, &task);
        }
        app.push("tasks.changed", json!({ "task": task }));
    }
}

/// One watchdog pass: a delivered task whose shim stopped answering would
/// stay unfinished forever, so it fails after `WATCH_MISSES` checks.
pub async fn check_sessions(app: &Arc<App>) {
    for project in app.tasks.watched() {
        let alive = match crate::agents::shim_port(app, &project) {
            Ok(port) => app
                .http
                .get(format!("http://127.0.0.1:{port}/health"))
                .timeout(WATCH_TIMEOUT)
                .send()
                .await
                .is_ok_and(|r| r.status().is_success()),
            Err(_) => false,
        };
        if !app.tasks.missed(&project, alive) {
            continue;
        }
        let failed = app.tasks.fail_delivered(&project, crate::now_ms());
        if failed.is_empty() {
            continue;
        }
        tracing::warn!(%project, "{SESSION_GONE}: {} task(s) failed", failed.len());
        push(app, failed);
        // The model only hears answers and failures, so this one is spoken.
        app.push(
            "agent.say",
            json!({
                "project": project,
                "text": format!("The agent session for project {project} is gone. Its task did not finish."),
            }),
        );
    }
}

/// Run the watchdog for as long as the server lives.
pub fn watch(app: &Arc<App>) {
    let app = Arc::downgrade(app);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(WATCH_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let Some(app) = app.upgrade() else { return };
            check_sessions(&app).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Tasks) {
        let dir = tempfile::tempdir().unwrap();
        let tasks = Tasks::open(&dir.path().join("t.db"), 0).unwrap();
        (dir, tasks)
    }

    fn states(changed: &[Task]) -> Vec<(&str, State, Option<usize>)> {
        changed
            .iter()
            .map(|t| (t.prompt.as_str(), t.state, t.queue_pos))
            .collect()
    }

    #[test]
    fn lifecycle_and_queue_positions() {
        let (_d, tasks) = store();
        let a = tasks.create("p", Some("th1"), "a", 10);
        let b = tasks.create("p", None, "b", 11);
        let other = tasks.create("q", None, "o", 12);
        assert_eq!((a.state, a.queue_pos), (State::Queued, Some(0)));
        assert_eq!((b.queue_pos, other.queue_pos), (Some(1), Some(0)));
        assert_eq!(a.thread_id.as_deref(), Some("th1"));
        assert_eq!(a.id.len(), 16);

        let changed = tasks.advance(&a.id, State::Spawning, 20);
        assert_eq!(states(&changed), [("a", State::Spawning, Some(0))]);
        assert_eq!(changed[0].updated_at, 20);
        tasks.advance(&a.id, State::Sent, 21);
        // The same state again, or a step back, changes nothing.
        assert!(tasks.advance(&a.id, State::Sent, 22).is_empty());
        assert!(tasks.advance(&a.id, State::Spawning, 22).is_empty());

        let ids = [a.id.clone()];
        let changed = tasks.event("p", Some(&ids), "start", "", 30);
        assert_eq!(states(&changed), [("a", State::Working, None)]);
        assert_eq!(changed[0].started_at, Some(30));
        // `sent` arriving after `start` must not undo it.
        assert!(tasks.advance(&a.id, State::Sent, 31).is_empty());

        let changed = tasks.event("p", Some(&ids), "activity", "  Edit notes/tea.md ", 32);
        assert_eq!(
            changed[0].last_activity.as_deref(),
            Some("Edit notes/tea.md")
        );
        assert!(tasks
            .event("p", Some(&ids), "activity", "Edit notes/tea.md", 33)
            .is_empty());
        let long = "x".repeat(500);
        let changed = tasks.event("p", Some(&ids), "activity", &long, 34);
        assert_eq!(
            changed[0].last_activity.as_ref().unwrap().len(),
            ACTIVITY_MAX
        );

        let changed = tasks.event("p", Some(&ids), "waiting", "permission", 40);
        assert_eq!(
            (changed[0].state, &changed[0].last_activity),
            (State::Waiting, &None)
        );
        let changed = tasks.event("p", Some(&ids), "resume", "", 41);
        assert_eq!(
            (changed[0].state, changed[0].started_at),
            (State::Working, Some(30))
        );

        let changed = tasks.say("p", &ids, "tea note added", 50);
        assert_eq!(changed[0].answer.as_deref(), Some("tea note added"));
        // Finishing `a` moves `b` up: both are pushed.
        let changed = tasks.event("p", Some(&ids), "stop", "answered", 51);
        assert_eq!(
            states(&changed),
            [("a", State::Done, None), ("b", State::Queued, Some(0))]
        );
        assert_eq!(
            (changed[0].updated_at, changed[1].updated_at),
            (51, 11),
            "a queue shift is not a change of the task itself"
        );

        // Final: nothing moves a finished task.
        assert!(tasks.event("p", Some(&ids), "start", "", 60).is_empty());
        assert!(tasks.say("p", &ids, "late", 60).is_empty());
        assert!(tasks.fail(&a.id, "late", 60).is_empty());
        assert!(tasks.event("p", Some(&ids), "nonsense", "", 60).is_empty());
        assert!(tasks.advance("no such id", State::Sent, 60).is_empty());

        let changed = tasks.fail(&b.id, "spawn failed", 70);
        assert_eq!(
            (changed[0].state, changed[0].error.as_deref()),
            (State::Failed, Some("spawn failed"))
        );
    }

    #[test]
    fn events_without_ids_hit_delivered_tasks_only() {
        let (_d, tasks) = store();
        let a = tasks.create("p", None, "a", 1);
        let b = tasks.create("p", None, "b", 2);
        let c = tasks.create("p", None, "c", 3);
        tasks.advance(&a.id, State::Sent, 4);
        tasks.advance(&b.id, State::Sent, 4);
        // `c` is still queued on the server: the shim cannot mean it.
        let changed = tasks.event("p", None, "stop", "answered", 5);
        assert_eq!(
            states(&changed),
            [
                ("a", State::Done, None),
                ("b", State::Done, None),
                ("c", State::Queued, Some(0))
            ]
        );
        // An id of another project is not touched.
        let other = tasks.create("q", None, "o", 6);
        assert!(tasks
            .event("p", Some(&[other.id]), "stop", "", 7)
            .is_empty());
        assert_eq!(c.queue_pos, Some(2));
    }

    #[test]
    fn lists() {
        let (_d, tasks) = store();
        let mut ids = Vec::new();
        for i in 0..8 {
            let t = tasks.create(if i % 2 == 0 { "p" } else { "q" }, None, &i.to_string(), i);
            ids.push(t.id);
        }
        for id in &ids[..7] {
            tasks.fail(id, "x", 100);
        }
        let prompts = |ts: Vec<Task>| ts.into_iter().map(|t| t.prompt).collect::<Vec<_>>();
        // The unfinished one plus the 5 last finished, newest first.
        assert_eq!(prompts(tasks.recent(None)), ["7", "6", "5", "4", "3", "2"]);
        assert_eq!(prompts(tasks.recent(Some("p"))), ["6", "4", "2", "0"]);
        assert_eq!(tasks.list(100 + DAY_MS).len(), 8);
        // A day later only the unfinished task is left.
        assert_eq!(prompts(tasks.list(101 + DAY_MS)), ["7"]);
    }

    #[test]
    fn survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let (queued, sent, done, old) = {
            let tasks = Tasks::open(&path, 0).unwrap();
            let old = tasks.create("p", None, "old", 1);
            tasks.fail(&old.id, "x", 2);
            let queued = tasks.create("p", None, "queued", KEEP_MS);
            let sent = tasks.create("p", None, "sent", KEEP_MS + 1);
            let done = tasks.create("p", None, "done", KEEP_MS + 2);
            tasks.advance(&sent.id, State::Sent, KEEP_MS + 3);
            tasks.advance(&done.id, State::Sent, KEEP_MS + 3);
            let ids = [sent.id.clone(), done.id.clone()];
            tasks.event("p", Some(&ids), "activity", "Bash ls", KEEP_MS + 4);
            let ids = [done.id.clone()];
            tasks.say("p", &ids, "all good", KEEP_MS + 5);
            tasks.event("p", Some(&ids), "stop", "", KEEP_MS + 6);
            (queued.id, sent.id, done.id, old.id)
        };
        let now = KEEP_MS + 10;
        let tasks = Tasks::open(&path, now).unwrap();
        let all = tasks.list(now);
        let by = |id: &str| all.iter().find(|t| t.id == id);
        assert!(by(&old).is_none(), "finished over a week ago: dropped");
        let q = by(&queued).unwrap();
        assert_eq!(
            (q.state, q.error.as_deref(), q.updated_at),
            (State::Failed, Some(RESTARTED), now)
        );
        // Delivered: the shim still holds it, so it stays open.
        assert_eq!(by(&sent).unwrap().state, State::Working);
        let d = by(&done).unwrap();
        assert_eq!(
            (d.state, d.answer.as_deref()),
            (State::Done, Some("all good"))
        );
        // The failure was written back.
        drop(tasks);
        let tasks = Tasks::open(&path, now + 1).unwrap();
        let again = tasks.list(now + 1);
        assert_eq!(
            again.iter().find(|t| t.id == queued).unwrap().updated_at,
            now
        );
    }

    #[test]
    fn two_misses_fail_the_delivered_tasks() {
        let (_d, tasks) = store();
        let a = tasks.create("p", None, "a", 1);
        tasks.create("p", None, "b", 2);
        assert!(tasks.watched().is_empty());
        tasks.advance(&a.id, State::Sent, 3);
        assert_eq!(tasks.watched(), ["p"]);
        assert!(!tasks.missed("p", false));
        // An answer in between starts the count again.
        assert!(!tasks.missed("p", true));
        assert!(!tasks.missed("p", false));
        assert!(tasks.missed("p", false));
        let failed = tasks.fail_delivered("p", 9);
        assert_eq!(
            states(&failed),
            [("a", State::Failed, None), ("b", State::Queued, Some(0))]
        );
        assert_eq!(failed[0].error.as_deref(), Some(SESSION_GONE));
        assert!(tasks.watched().is_empty());
    }
}
