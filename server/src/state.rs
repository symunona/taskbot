//! Persistent state: devices, pair codes, project -> shim port index, threads.
//! Lives in the state dir, never inside the vault.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Mutex;

pub const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub const CODE_LEN: usize = 8;
pub const CODE_TTL_SECS: i64 = 300;
/// `last_seen_at` is only rewritten when it is older than this (keeps writes down).
const LAST_SEEN_GRANULARITY_SECS: i64 = 60;
pub const THREAD_NAME_DEFAULT: &str = "New thread";
pub const THREAD_NAME_MAX: usize = 80;
/// Most entries one `threads.append` may carry.
pub const APPEND_MAX: usize = 200;
pub const ENTRY_KINDS: [&str; 6] = ["user", "model", "agent", "tool", "marker", "system"];

type Res<T> = Result<T, rusqlite::Error>;

pub struct Store {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
}

pub struct Paired {
    pub device_id: String,
    pub device_token: String,
}

/// One conversation. Times are epoch milliseconds.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Thread {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub last_accessed_at: i64,
    /// Active project, a name from `[projects.*]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// A pinned note. `seq` = list order, never leaves the server.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Pin {
    #[serde(skip)]
    pub seq: i64,
    pub text: String,
    /// Epoch ms of the write or last replace.
    pub at: i64,
}

/// A thread as `memory_read` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Remembered {
    pub thread: Thread,
    /// How many pins the thread holds.
    pub pins: i64,
    /// The thread has a memory text.
    pub memory: bool,
}

/// Thread memory: where the thread stands, made by a compact.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Memory {
    pub text: String,
    /// `id` of the last entry folded in.
    pub upto_id: String,
    /// Epoch ms of the compact.
    pub at: i64,
}

/// Token counts of a thread, all time.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq)]
pub struct Usage {
    pub input: i64,
    pub output: i64,
}

/// What a compact starts from.
#[derive(Debug)]
pub struct FoldState {
    pub summary: Option<String>,
    /// `seq` of the last folded entry, 0 = none.
    pub upto: i64,
    /// `(seq, entry)` after `upto`, in `seq` order.
    pub entries: Vec<(i64, Value)>,
}

/// A thread entry ready to store. The server reads `id`, `ts` and `kind`;
/// the rest of the entry stays opaque inside `body`.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub ts: i64,
    pub kind: String,
    /// The whole entry, as JSON text.
    pub body: String,
}

impl Entry {
    pub fn parse(v: &Value) -> Result<Self, String> {
        if !v.is_object() {
            return Err("entry must be an object".into());
        }
        let id = v["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("entry needs a non-empty string id")?;
        let ts = v["ts"]
            .as_i64()
            .ok_or_else(|| format!("entry {id:?}: ts must be an integer (epoch ms)"))?;
        let kind = v["kind"]
            .as_str()
            .filter(|k| ENTRY_KINDS.contains(k))
            .ok_or_else(|| format!("entry {id:?}: kind must be one of {ENTRY_KINDS:?}"))?;
        Ok(Self {
            id: id.to_string(),
            ts,
            kind: kind.to_string(),
            body: v.to_string(),
        })
    }
}

/// Clean a thread name: control chars dropped, trimmed, cut to
/// `THREAD_NAME_MAX` chars. Nothing left is an error.
pub fn thread_name(raw: &str) -> Result<String, String> {
    let clean: String = raw.chars().filter(|c| !c.is_control()).collect();
    let name: String = clean.trim().chars().take(THREAD_NAME_MAX).collect();
    let name = name.trim_end();
    if name.is_empty() {
        return Err("thread name is empty".into());
    }
    Ok(name.to_string())
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn new_code() -> String {
    // 32 symbols: the low 5 bits of a random byte are an unbiased index.
    crate::random_bytes::<CODE_LEN>()
        .iter()
        .map(|b| CODE_ALPHABET[(b & 31) as usize] as char)
        .collect()
}

impl Store {
    pub fn open(path: &Path) -> Res<Self> {
        if let Some(dir) = path.parent() {
            use std::os::unix::fs::DirBuilderExt;
            let _ = std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir);
        }
        let conn = Connection::open(path)?;
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS devices(
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                token_hash TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL,
                last_seen_at INTEGER);
             CREATE TABLE IF NOT EXISTS pair_codes(
                code TEXT PRIMARY KEY,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                used_at INTEGER);
             CREATE TABLE IF NOT EXISTS project_ports(
                project TEXT PRIMARY KEY,
                idx INTEGER NOT NULL UNIQUE);
             CREATE TABLE IF NOT EXISTS threads(
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_accessed_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS thread_entries(
                thread_id TEXT NOT NULL REFERENCES threads(id),
                seq INTEGER NOT NULL,
                id TEXT NOT NULL,
                ts INTEGER NOT NULL,
                kind TEXT NOT NULL,
                body TEXT NOT NULL,
                PRIMARY KEY(thread_id, seq),
                UNIQUE(thread_id, id));
             CREATE TABLE IF NOT EXISTS pins(
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                thread_id TEXT,
                text TEXT NOT NULL,
                at INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS pins_thread ON pins(thread_id);",
        )?;
        add_missing_columns(
            &conn,
            "threads",
            &[
                ("summary", "TEXT"),
                ("summary_upto", "INTEGER NOT NULL DEFAULT 0"),
                ("summary_at", "INTEGER NOT NULL DEFAULT 0"),
                ("tokens_in", "INTEGER NOT NULL DEFAULT 0"),
                ("tokens_out", "INTEGER NOT NULL DEFAULT 0"),
                ("project", "TEXT"),
            ],
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Mint a pair code. Returns `(code, expires_at)`.
    pub fn new_pair_code(&self, now: i64) -> Res<(String, i64)> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM pair_codes WHERE expires_at <= ?1 OR used_at IS NOT NULL",
            params![now],
        )?;
        let expires_at = now + CODE_TTL_SECS;
        loop {
            let code = new_code();
            let n = conn.execute(
                "INSERT OR IGNORE INTO pair_codes(code, created_at, expires_at) VALUES (?1, ?2, ?3)",
                params![code, now, expires_at],
            )?;
            if n == 1 {
                return Ok((code, expires_at));
            }
        }
    }

    /// Claim a code and create the device. `None` = unknown, used or expired code.
    pub fn claim(&self, code: &str, name: &str, now: i64) -> Res<Option<Paired>> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let claimed = tx.execute(
            "UPDATE pair_codes SET used_at = ?2 WHERE code = ?1 AND used_at IS NULL AND expires_at > ?2",
            params![code, now],
        )?;
        if claimed != 1 {
            return Ok(None);
        }
        let device_id = hex::encode(crate::random_bytes::<8>());
        let device_token = hex::encode(crate::random_bytes::<32>());
        tx.execute(
            "INSERT INTO devices(id, name, token_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![device_id, name, hash_token(&device_token), now],
        )?;
        tx.commit()?;
        Ok(Some(Paired {
            device_id,
            device_token,
        }))
    }

    /// Resolve a bearer token to its device, touching `last_seen_at`.
    pub fn authenticate(&self, token: &str, now: i64) -> Res<Option<Device>> {
        let conn = self.conn();
        let dev = conn
            .query_row(
                "SELECT id, name, created_at, last_seen_at FROM devices WHERE token_hash = ?1",
                params![hash_token(token)],
                row_device,
            )
            .optional()?;
        if let Some(d) = &dev {
            if d.last_seen_at
                .is_none_or(|t| now - t >= LAST_SEEN_GRANULARITY_SECS)
            {
                conn.execute(
                    "UPDATE devices SET last_seen_at = ?2 WHERE id = ?1",
                    params![d.id, now],
                )?;
            }
        }
        Ok(dev)
    }

    pub fn devices(&self) -> Res<Vec<Device>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT id, name, created_at, last_seen_at FROM devices ORDER BY created_at, id",
        )?;
        let rows = st.query_map([], row_device)?.collect();
        rows
    }

    pub fn revoke(&self, id: &str) -> Res<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM devices WHERE id = ?1", params![id])?
            == 1)
    }

    /// Stable index for a project; assigned on first use and never reused.
    pub fn port_index(&self, project: &str) -> Res<u32> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO project_ports(project, idx)
             VALUES (?1, (SELECT COALESCE(MAX(idx) + 1, 0) FROM project_ports))",
            params![project],
        )?;
        let idx = tx.query_row(
            "SELECT idx FROM project_ports WHERE project = ?1",
            params![project],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok(idx)
    }

    /// All threads, most recently accessed first.
    pub fn threads(&self) -> Res<Vec<Thread>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT id, name, created_at, last_accessed_at, project FROM threads
             ORDER BY last_accessed_at DESC, rowid DESC",
        )?;
        let rows = st.query_map([], row_thread)?.collect();
        rows
    }

    /// `name` must already have gone through [`thread_name`]. `now_ms` = epoch ms.
    pub fn create_thread(&self, name: &str, now_ms: i64) -> Res<Thread> {
        let thread = Thread {
            id: hex::encode(crate::random_bytes::<8>()),
            name: name.to_string(),
            created_at: now_ms,
            last_accessed_at: now_ms,
            project: None,
        };
        self.conn().execute(
            "INSERT INTO threads(id, name, created_at, last_accessed_at) VALUES (?1, ?2, ?3, ?3)",
            params![thread.id, thread.name, now_ms],
        )?;
        Ok(thread)
    }

    /// A thread and its entries in `seq` order, touching `last_accessed_at`.
    /// `None` = unknown id.
    pub fn thread(&self, id: &str, now_ms: i64) -> Res<Option<(Thread, Vec<Value>)>> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !touch_thread(&tx, id, now_ms)? {
            return Ok(None);
        }
        let thread = select_thread(&tx, id)?;
        let entries = {
            let mut st =
                tx.prepare("SELECT body FROM thread_entries WHERE thread_id = ?1 ORDER BY seq")?;
            let rows = st
                .query_map(params![id], |r| {
                    let body: String = r.get(0)?;
                    serde_json::from_str::<Value>(&body).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })
                })?
                .collect::<Res<Vec<Value>>>()?;
            rows
        };
        tx.commit()?;
        Ok(Some((thread, entries)))
    }

    /// `name` must already have gone through [`thread_name`]. `None` = unknown id.
    pub fn rename_thread(&self, id: &str, name: &str) -> Res<Option<Thread>> {
        let conn = self.conn();
        let n = conn.execute(
            "UPDATE threads SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        if n != 1 {
            return Ok(None);
        }
        select_thread(&conn, id).map(Some)
    }

    /// A thread, `last_accessed_at` left alone. `None` = unknown id.
    pub fn thread_info(&self, id: &str) -> Res<Option<Thread>> {
        select_thread(&self.conn(), id).optional()
    }

    /// Set (`Some`) or clear (`None`) the active project. `None` = unknown id.
    pub fn set_thread_project(&self, id: &str, project: Option<&str>) -> Res<Option<Thread>> {
        let conn = self.conn();
        let n = conn.execute(
            "UPDATE threads SET project = ?2 WHERE id = ?1",
            params![id, project],
        )?;
        if n != 1 {
            return Ok(None);
        }
        select_thread(&conn, id).map(Some)
    }

    /// Make `project` the active one if the thread has none. `None` = the
    /// thread has one already, or is unknown: nothing changed.
    pub fn adopt_thread_project(&self, id: &str, project: &str) -> Res<Option<Thread>> {
        let conn = self.conn();
        let n = conn.execute(
            "UPDATE threads SET project = ?2 WHERE id = ?1 AND project IS NULL",
            params![id, project],
        )?;
        if n != 1 {
            return Ok(None);
        }
        select_thread(&conn, id).map(Some)
    }

    /// Pins of a thread, or the `always` pins (`None`), in insert order.
    pub fn pins(&self, thread: Option<&str>) -> Res<Vec<Pin>> {
        let conn = self.conn();
        let mut st =
            conn.prepare("SELECT seq, text, at FROM pins WHERE thread_id IS ?1 ORDER BY seq")?;
        let rows = st
            .query_map(params![thread], |r| {
                Ok(Pin {
                    seq: r.get(0)?,
                    text: r.get(1)?,
                    at: r.get(2)?,
                })
            })?
            .collect();
        rows
    }

    /// `thread` = `None`: an `always` pin.
    pub fn add_pin(&self, thread: Option<&str>, text: &str, now_ms: i64) -> Res<()> {
        self.conn().execute(
            "INSERT INTO pins(thread_id, text, at) VALUES (?1, ?2, ?3)",
            params![thread, text, now_ms],
        )?;
        Ok(())
    }

    /// Swap the text of a pin; it keeps its place in the list.
    pub fn set_pin(&self, seq: i64, text: &str, now_ms: i64) -> Res<()> {
        self.conn().execute(
            "UPDATE pins SET text = ?2, at = ?3 WHERE seq = ?1",
            params![seq, text, now_ms],
        )?;
        Ok(())
    }

    pub fn delete_pin(&self, seq: i64) -> Res<()> {
        self.conn()
            .execute("DELETE FROM pins WHERE seq = ?1", params![seq])?;
        Ok(())
    }

    /// Threads that hold pins or a memory, most recently accessed first.
    /// `skip` = a thread id to leave out.
    pub fn remembered(&self, skip: Option<&str>, max: usize) -> Res<Vec<Remembered>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT t.id, t.name, t.created_at, t.last_accessed_at, t.project,
                    (SELECT COUNT(*) FROM pins p WHERE p.thread_id = t.id),
                    t.summary IS NOT NULL
             FROM threads t
             WHERE t.id IS NOT ?1
               AND (t.summary IS NOT NULL
                    OR EXISTS (SELECT 1 FROM pins p WHERE p.thread_id = t.id))
             ORDER BY t.last_accessed_at DESC, t.rowid DESC
             LIMIT ?2",
        )?;
        let rows = st
            .query_map(params![skip, max as i64], |r| {
                Ok(Remembered {
                    thread: row_thread(r)?,
                    pins: r.get(5)?,
                    memory: r.get(6)?,
                })
            })?
            .collect();
        rows
    }

    /// Upsert entries by their `id`, touching `last_accessed_at`. A new id takes
    /// the next `seq`; a known id is replaced in its slot. Returns how many
    /// entries were written, `None` = unknown thread.
    pub fn append_entries(&self, id: &str, entries: &[Entry], now_ms: i64) -> Res<Option<usize>> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !touch_thread(&tx, id, now_ms)? {
            return Ok(None);
        }
        {
            let mut st = tx.prepare(
                "INSERT INTO thread_entries(thread_id, seq, id, ts, kind, body)
                 VALUES (?1,
                         (SELECT COALESCE(MAX(seq), 0) + 1 FROM thread_entries WHERE thread_id = ?1),
                         ?2, ?3, ?4, ?5)
                 ON CONFLICT(thread_id, id) DO UPDATE
                 SET ts = excluded.ts, kind = excluded.kind, body = excluded.body",
            )?;
            for e in entries {
                st.execute(params![id, e.id, e.ts, e.kind, e.body])?;
            }
        }
        tx.commit()?;
        Ok(Some(entries.len()))
    }

    /// Memory and token use of a thread. `None` = unknown thread.
    pub fn thread_memory(&self, id: &str) -> Res<Option<(Option<Memory>, Usage)>> {
        self.conn()
            .query_row(
                "SELECT t.summary, e.id, t.summary_at, t.tokens_in, t.tokens_out
                 FROM threads t
                 LEFT JOIN thread_entries e ON e.thread_id = t.id AND e.seq = t.summary_upto
                 WHERE t.id = ?1",
                params![id],
                |r| {
                    let text: Option<String> = r.get(0)?;
                    let upto_id: Option<String> = r.get(1)?;
                    let memory = text.zip(upto_id).map(|(text, upto_id)| Memory {
                        text,
                        upto_id,
                        at: r.get(2).unwrap_or(0),
                    });
                    let usage = Usage {
                        input: r.get(3)?,
                        output: r.get(4)?,
                    };
                    Ok((memory, usage))
                },
            )
            .optional()
    }

    /// `user` entries not folded into the memory yet. Cheap trigger check.
    pub fn unfolded_turns(&self, id: &str) -> Res<i64> {
        self.conn().query_row(
            "SELECT COUNT(*) FROM thread_entries
             WHERE thread_id = ?1 AND kind = 'user'
               AND seq > COALESCE((SELECT summary_upto FROM threads WHERE id = ?1), 0)",
            params![id],
            |r| r.get(0),
        )
    }

    /// Memory text, fold point and the entries after it. `None` = unknown thread.
    pub fn fold_state(&self, id: &str) -> Res<Option<FoldState>> {
        let conn = self.conn();
        let head: Option<(Option<String>, i64)> = conn
            .query_row(
                "SELECT summary, summary_upto FROM threads WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((summary, upto)) = head else {
            return Ok(None);
        };
        let mut st = conn.prepare(
            "SELECT seq, body FROM thread_entries WHERE thread_id = ?1 AND seq > ?2 ORDER BY seq",
        )?;
        let entries = st
            .query_map(params![id, upto], |r| {
                let body: String = r.get(1)?;
                // A body that is not JSON cannot be folded; it counts as nothing.
                Ok((
                    r.get(0)?,
                    serde_json::from_str(&body).unwrap_or(Value::Null),
                ))
            })?
            .collect::<Res<Vec<_>>>()?;
        Ok(Some(FoldState {
            summary,
            upto,
            entries,
        }))
    }

    /// Store a compact's result and add its token use. False = the fold point
    /// moved meanwhile (or the thread is gone): nothing stored.
    pub fn store_memory(
        &self,
        id: &str,
        old_upto: i64,
        new_upto: i64,
        text: &str,
        now_ms: i64,
        used: Usage,
    ) -> Res<bool> {
        Ok(self.conn().execute(
            "UPDATE threads
             SET summary = ?4, summary_upto = ?3, summary_at = ?5,
                 tokens_in = tokens_in + ?6, tokens_out = tokens_out + ?7
             WHERE id = ?1 AND summary_upto = ?2",
            params![
                id,
                old_upto,
                new_upto,
                text,
                now_ms,
                used.input,
                used.output
            ],
        )? == 1)
    }

    /// Add token counts to a thread. `None` = unknown thread.
    pub fn add_usage(&self, id: &str, used: Usage) -> Res<Option<Usage>> {
        self.conn()
            .query_row(
                "UPDATE threads SET tokens_in = tokens_in + ?2, tokens_out = tokens_out + ?3
                 WHERE id = ?1 RETURNING tokens_in, tokens_out",
                params![id, used.input, used.output],
                |r| {
                    Ok(Usage {
                        input: r.get(0)?,
                        output: r.get(1)?,
                    })
                },
            )
            .optional()
    }
}

/// `ALTER TABLE ... ADD COLUMN` for each column an older db lacks.
fn add_missing_columns(conn: &Connection, table: &str, columns: &[(&str, &str)]) -> Res<()> {
    let have: Vec<String> = conn
        .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?
        .query_map([], |r| r.get(0))?
        .collect::<Res<_>>()?;
    for (name, decl) in columns {
        if !have.iter().any(|h| h == name) {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {name} {decl}"))?;
        }
    }
    Ok(())
}

fn touch_thread(conn: &Connection, id: &str, now_ms: i64) -> Res<bool> {
    Ok(conn.execute(
        "UPDATE threads SET last_accessed_at = ?2 WHERE id = ?1",
        params![id, now_ms],
    )? == 1)
}

fn select_thread(conn: &Connection, id: &str) -> Res<Thread> {
    conn.query_row(
        "SELECT id, name, created_at, last_accessed_at, project FROM threads WHERE id = ?1",
        params![id],
        row_thread,
    )
}

fn row_thread(r: &rusqlite::Row<'_>) -> Res<Thread> {
    Ok(Thread {
        id: r.get(0)?,
        name: r.get(1)?,
        created_at: r.get(2)?,
        last_accessed_at: r.get(3)?,
        project: r.get(4)?,
    })
}

fn row_device(r: &rusqlite::Row<'_>) -> Res<Device> {
    Ok(Device {
        id: r.get(0)?,
        name: r.get(1)?,
        created_at: r.get(2)?,
        last_seen_at: r.get(3)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("t.db")).unwrap();
        (dir, s)
    }

    #[test]
    fn code_shape() {
        let (_d, s) = store();
        let (code, exp) = s.new_pair_code(1000).unwrap();
        assert_eq!(code.len(), 8);
        assert!(code.bytes().all(|b| CODE_ALPHABET.contains(&b)));
        assert_eq!(exp, 1000 + 300);
    }

    #[test]
    fn claim_is_single_use() {
        let (_d, s) = store();
        let (code, _) = s.new_pair_code(1000).unwrap();
        assert!(s.claim("WRONGXXX", "a", 1001).unwrap().is_none());
        let p = s
            .claim(&code, "phone", 1001)
            .unwrap()
            .expect("first claim wins");
        assert_eq!(p.device_token.len(), 64);
        assert!(s.claim(&code, "again", 1002).unwrap().is_none());
        assert_eq!(s.devices().unwrap().len(), 1);
    }

    #[test]
    fn claim_expires() {
        let (_d, s) = store();
        let (code, exp) = s.new_pair_code(1000).unwrap();
        assert!(
            s.claim(&code, "late", exp).unwrap().is_none(),
            "expires_at itself is too late"
        );
        assert!(s.claim(&code, "late", exp + 100).unwrap().is_none());
        assert!(s.devices().unwrap().is_empty());
        let (code2, exp2) = s.new_pair_code(2000).unwrap();
        assert!(s.claim(&code2, "ok", exp2 - 1).unwrap().is_some());
    }

    #[test]
    fn concurrent_claim_has_one_winner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let (code, _) = Store::open(&path).unwrap().new_pair_code(1000).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(12));
        let handles: Vec<_> = (0..12)
            .map(|i| {
                let (path, code, barrier) = (path.clone(), code.clone(), barrier.clone());
                // One connection per thread: the race is settled by sqlite, not by our mutex.
                std::thread::spawn(move || {
                    let s = Store::open(&path).unwrap();
                    barrier.wait();
                    s.claim(&code, &format!("d{i}"), 1001).unwrap().is_some()
                })
            })
            .collect();
        let wins = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|w| *w)
            .count();
        assert_eq!(wins, 1);
        assert_eq!(Store::open(&path).unwrap().devices().unwrap().len(), 1);
    }

    #[test]
    fn token_auth_and_revoke() {
        let (dir, s) = store();
        let (code, _) = s.new_pair_code(1000).unwrap();
        let p = s.claim(&code, "phone", 1001).unwrap().unwrap();

        let d = s
            .authenticate(&p.device_token, 2000)
            .unwrap()
            .expect("valid token");
        assert_eq!(
            (d.id.as_str(), d.name.as_str()),
            (p.device_id.as_str(), "phone")
        );
        assert_eq!(s.devices().unwrap()[0].last_seen_at, Some(2000));
        assert!(s.authenticate("", 2000).unwrap().is_none());
        assert!(s.authenticate(&"0".repeat(64), 2000).unwrap().is_none());
        // The hash is not itself a credential, and the token is not on disk.
        assert!(s
            .authenticate(&hash_token(&p.device_token), 2000)
            .unwrap()
            .is_none());
        drop(s);
        let s = Store::open(&dir.path().join("t.db")).unwrap();
        for f in std::fs::read_dir(dir.path()).unwrap() {
            let raw = std::fs::read(f.unwrap().path()).unwrap();
            assert!(!String::from_utf8_lossy(&raw).contains(&p.device_token));
        }

        assert!(s.revoke(&p.device_id).unwrap());
        assert!(!s.revoke(&p.device_id).unwrap());
        assert!(s.authenticate(&p.device_token, 3000).unwrap().is_none());
    }

    #[test]
    fn port_index_is_stable() {
        let (_d, s) = store();
        assert_eq!(s.port_index("a").unwrap(), 0);
        assert_eq!(s.port_index("b").unwrap(), 1);
        assert_eq!(s.port_index("a").unwrap(), 0);
        assert_eq!(s.port_index("c").unwrap(), 2);
    }

    fn entry(id: &str, ts: i64, text: &str) -> Entry {
        Entry::parse(&serde_json::json!({"id": id, "ts": ts, "kind": "user", "text": text}))
            .unwrap()
    }

    #[test]
    fn thread_name_rules() {
        assert_eq!(thread_name("  Tea\tnotes\n ").unwrap(), "Teanotes");
        assert_eq!(thread_name("árvíztűrő").unwrap(), "árvíztűrő");
        assert_eq!(thread_name(&"é".repeat(200)).unwrap().chars().count(), 80);
        // The cut must not leave a trailing space behind.
        let cut = thread_name(&format!("{} tail", "x".repeat(79))).unwrap();
        assert_eq!(cut, "x".repeat(79));
        for empty in ["", "   ", "\n\t", "\u{0}"] {
            assert!(thread_name(empty).is_err(), "{empty:?}");
        }
    }

    #[test]
    fn entry_parse_checks_id_ts_kind() {
        use serde_json::json;
        let full = json!({"id": "e1", "ts": 5, "kind": "tool", "text": "", "tool": {"name": "x", "args": {}}});
        let e = Entry::parse(&full).unwrap();
        assert_eq!((e.id.as_str(), e.ts, e.kind.as_str()), ("e1", 5, "tool"));
        assert_eq!(serde_json::from_str::<Value>(&e.body).unwrap(), full);
        for bad in [
            json!("e1"),
            json!({"ts": 1, "kind": "user"}),
            json!({"id": "", "ts": 1, "kind": "user"}),
            json!({"id": "a", "kind": "user"}),
            json!({"id": "a", "ts": "1", "kind": "user"}),
            json!({"id": "a", "ts": 1}),
            json!({"id": "a", "ts": 1, "kind": "robot"}),
        ] {
            assert!(Entry::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn threads_create_list_get_rename() {
        let (_d, s) = store();
        assert!(s.threads().unwrap().is_empty());
        let a = s.create_thread("A", 1000).unwrap();
        let b = s.create_thread("B", 1000).unwrap();
        assert_eq!(a.id.len(), 16);
        assert!(a.id.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_eq!((a.created_at, a.last_accessed_at), (1000, 1000));
        let ids =
            |s: &Store| -> Vec<String> { s.threads().unwrap().into_iter().map(|t| t.id).collect() };
        assert_eq!(ids(&s), [b.id.clone(), a.id.clone()], "tie: newest first");

        // get touches last_accessed_at, which reorders the list.
        let (got, entries) = s.thread(&a.id, 2000).unwrap().unwrap();
        assert_eq!(
            got,
            Thread {
                last_accessed_at: 2000,
                ..a.clone()
            }
        );
        assert!(entries.is_empty());
        assert_eq!(ids(&s), [a.id.clone(), b.id.clone()]);

        // rename changes the name only.
        let renamed = s.rename_thread(&b.id, "Bee").unwrap().unwrap();
        assert_eq!(
            renamed,
            Thread {
                name: "Bee".into(),
                ..b.clone()
            }
        );
        assert_eq!(s.threads().unwrap()[1], renamed);

        assert!(s.thread("nope", 3000).unwrap().is_none());
        assert!(s.rename_thread("nope", "x").unwrap().is_none());
        assert!(s.append_entries("nope", &[], 3000).unwrap().is_none());
    }

    #[test]
    fn old_db_gains_the_memory_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE threads(id TEXT PRIMARY KEY, name TEXT NOT NULL,
                    created_at INTEGER NOT NULL, last_accessed_at INTEGER NOT NULL);
                 INSERT INTO threads VALUES ('old', 'Old', 1, 1);",
            )
            .unwrap();
        for _ in 0..2 {
            let s = Store::open(&path).unwrap();
            assert_eq!(
                s.thread_memory("old").unwrap(),
                Some((None, Usage::default()))
            );
        }
    }

    #[test]
    fn old_db_gains_pins_and_project() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE threads(id TEXT PRIMARY KEY, name TEXT NOT NULL,
                    created_at INTEGER NOT NULL, last_accessed_at INTEGER NOT NULL);
                 INSERT INTO threads VALUES ('old', 'Old', 1, 1);",
            )
            .unwrap();
        for _ in 0..2 {
            let s = Store::open(&path).unwrap();
            assert!(s.pins(Some("old")).unwrap().is_empty());
            assert!(s.pins(None).unwrap().is_empty());
            assert_eq!(s.thread_info("old").unwrap().unwrap().project, None);
        }
    }

    #[test]
    fn pins_per_scope_keep_their_order() {
        let (_d, s) = store();
        let t = s.create_thread("T", 1000).unwrap();
        let other = s.create_thread("O", 1000).unwrap();
        s.add_pin(Some(&t.id), "a", 1).unwrap();
        s.add_pin(None, "everywhere", 2).unwrap();
        s.add_pin(Some(&t.id), "b", 3).unwrap();
        s.add_pin(Some(&t.id), "c", 4).unwrap();
        let texts = |pins: Vec<Pin>| pins.into_iter().map(|p| p.text).collect::<Vec<_>>();
        assert_eq!(texts(s.pins(Some(&t.id)).unwrap()), ["a", "b", "c"]);
        assert_eq!(texts(s.pins(None).unwrap()), ["everywhere"]);
        assert!(s.pins(Some(&other.id)).unwrap().is_empty());

        // A swap keeps the slot and takes the new time; a delete closes the gap.
        let pins = s.pins(Some(&t.id)).unwrap();
        s.set_pin(pins[1].seq, "B", 9).unwrap();
        s.delete_pin(pins[0].seq).unwrap();
        let pins = s.pins(Some(&t.id)).unwrap();
        assert_eq!((pins[0].text.as_str(), pins[0].at), ("B", 9));
        assert_eq!(texts(pins), ["B", "c"]);
        assert_eq!(
            serde_json::to_value(&s.pins(None).unwrap()[0]).unwrap(),
            serde_json::json!({"text": "everywhere", "at": 2}),
            "seq stays inside"
        );

        // Listed: threads with pins or a memory, the asking one left out.
        let empty = s.create_thread("E", 1000).unwrap();
        s.append_entries(&other.id, &[entry("e1", 1, "x")], 2000)
            .unwrap();
        s.store_memory(&other.id, 0, 1, "- m", 3000, Usage::default())
            .unwrap();
        let listed = s.remembered(None, 50).unwrap();
        let ids: Vec<&str> = listed.iter().map(|r| r.thread.id.as_str()).collect();
        assert_eq!(
            ids,
            [other.id.as_str(), t.id.as_str()],
            "newest access first"
        );
        assert!(!ids.contains(&empty.id.as_str()));
        assert_eq!((listed[0].pins, listed[0].memory), (0, true));
        assert_eq!((listed[1].pins, listed[1].memory), (2, false));
        assert_eq!(s.remembered(Some(&t.id), 50).unwrap().len(), 1);
        assert_eq!(s.remembered(None, 1).unwrap().len(), 1);
    }

    #[test]
    fn active_project_set_clear_adopt() {
        let (_d, s) = store();
        let t = s.create_thread("T", 1000).unwrap();
        assert_eq!(
            serde_json::to_value(&t).unwrap().as_object().unwrap().len(),
            4,
            "no project: field absent"
        );
        let adopted = s.adopt_thread_project(&t.id, "garden").unwrap().unwrap();
        assert_eq!(adopted.project.as_deref(), Some("garden"));
        assert!(
            s.adopt_thread_project(&t.id, "other").unwrap().is_none(),
            "has one: no change"
        );
        let set = s.set_thread_project(&t.id, Some("other")).unwrap().unwrap();
        assert_eq!(serde_json::to_value(&set).unwrap()["project"], "other");
        assert_eq!(s.threads().unwrap()[0].project.as_deref(), Some("other"));
        let cleared = s.set_thread_project(&t.id, None).unwrap().unwrap();
        assert_eq!(cleared.project, None);
        assert_eq!(cleared.last_accessed_at, 1000, "not an access");
        assert!(s.set_thread_project("nope", None).unwrap().is_none());
        assert!(s.adopt_thread_project("nope", "x").unwrap().is_none());
        assert!(s.thread_info("nope").unwrap().is_none());
    }

    #[test]
    fn memory_fold_point_and_usage() {
        let (_d, s) = store();
        let t = s.create_thread("T", 1000).unwrap();
        assert!(s.thread_memory("nope").unwrap().is_none());
        assert!(s.fold_state("nope").unwrap().is_none());
        let entries: Vec<Entry> = (1..=5).map(|i| entry(&format!("e{i}"), i, "x")).collect();
        s.append_entries(&t.id, &entries, 2000).unwrap();
        assert_eq!(s.unfolded_turns(&t.id).unwrap(), 5);
        let state = s.fold_state(&t.id).unwrap().unwrap();
        assert_eq!(
            (state.summary, state.upto, state.entries.len()),
            (None, 0, 5)
        );

        let used = Usage {
            input: 7,
            output: 3,
        };
        assert!(s.store_memory(&t.id, 0, 3, "- a", 5000, used).unwrap());
        assert!(
            !s.store_memory(&t.id, 0, 4, "- late", 6000, used).unwrap(),
            "fold point moved: loser stores nothing"
        );
        let (memory, usage) = s.thread_memory(&t.id).unwrap().unwrap();
        let memory = memory.unwrap();
        assert_eq!(
            (memory.text.as_str(), memory.upto_id.as_str(), memory.at),
            ("- a", "e3", 5000)
        );
        assert_eq!(usage, used);
        assert_eq!(s.unfolded_turns(&t.id).unwrap(), 2);
        let state = s.fold_state(&t.id).unwrap().unwrap();
        assert_eq!(
            state.entries.iter().map(|e| e.0).collect::<Vec<_>>(),
            [4, 5]
        );

        let total = s
            .add_usage(
                &t.id,
                Usage {
                    input: 1,
                    output: 1,
                },
            )
            .unwrap();
        assert_eq!(
            total,
            Some(Usage {
                input: 8,
                output: 4
            })
        );
        assert!(s.add_usage("nope", used).unwrap().is_none());
    }

    #[test]
    fn append_upserts_and_keeps_the_slot() {
        let (dir, s) = store();
        let t = s.create_thread("T", 1000).unwrap();
        let other = s.create_thread("O", 1000).unwrap();
        let n = s
            .append_entries(
                &t.id,
                &[
                    entry("a", 1, "one"),
                    entry("b", 2, "tw"),
                    entry("c", 3, "three"),
                ],
                2000,
            )
            .unwrap();
        assert_eq!(n, Some(3));
        // Same entry id in another thread is a different entry.
        s.append_entries(&other.id, &[entry("b", 9, "elsewhere")], 2000)
            .unwrap();
        // Replace `b`, add `d`; `b` twice in one call: the last one wins.
        let n = s
            .append_entries(
                &t.id,
                &[
                    entry("b", 20, "two"),
                    entry("d", 4, "four"),
                    entry("b", 21, "two!"),
                ],
                3000,
            )
            .unwrap();
        assert_eq!(n, Some(3));

        drop(s);
        let s = Store::open(&dir.path().join("t.db")).unwrap();
        let (thread, entries) = s.thread(&t.id, 3000).unwrap().unwrap();
        assert_eq!(thread.last_accessed_at, 3000);
        let seen: Vec<(&str, i64, &str)> = entries
            .iter()
            .map(|e| {
                (
                    e["id"].as_str().unwrap(),
                    e["ts"].as_i64().unwrap(),
                    e["text"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ("a", 1, "one"),
                ("b", 21, "two!"),
                ("c", 3, "three"),
                ("d", 4, "four")
            ]
        );
        let (_, entries) = s.thread(&other.id, 3000).unwrap().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["text"], "elsewhere");
        assert_eq!(s.threads().unwrap()[0].id, other.id, "append and get touch");
    }
}
