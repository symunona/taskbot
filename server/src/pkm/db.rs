use rusqlite::{Connection, Result};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Database {
    pub conn: Mutex<Connection>,
}

impl Database {
    pub fn new(path: &PathBuf) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn init(conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY,
                filename TEXT NOT NULL,
                content TEXT NOT NULL,
                frontmatter TEXT,
                keywords TEXT,
                created TEXT,
                last_modified INTEGER,
                revision INTEGER DEFAULT 1
            )",
            [],
        )?;

        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(
                content,
                keywords,
                content='files',
                content_rowid='rowid'
            )",
            [],
        )?;

        // Triggers to keep FTS index updated
        conn.execute(
            "CREATE TRIGGER IF NOT EXISTS files_ai AFTER INSERT ON files BEGIN
                INSERT INTO files_fts(rowid, content, keywords)
                VALUES (new.rowid, new.content, new.keywords);
            END;",
            [],
        )?;

        conn.execute(
            "CREATE TRIGGER IF NOT EXISTS files_ad AFTER DELETE ON files BEGIN
                INSERT INTO files_fts(files_fts, rowid, content, keywords)
                VALUES ('delete', old.rowid, old.content, old.keywords);
            END;",
            [],
        )?;

        conn.execute(
            "CREATE TRIGGER IF NOT EXISTS files_au AFTER UPDATE ON files BEGIN
                INSERT INTO files_fts(files_fts, rowid, content, keywords)
                VALUES ('delete', old.rowid, old.content, old.keywords);
                INSERT INTO files_fts(rowid, content, keywords)
                VALUES (new.rowid, new.content, new.keywords);
            END;",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                client_id TEXT NOT NULL,
                ip_address TEXT,
                user_agent TEXT,
                created_at INTEGER NOT NULL,
                last_seen INTEGER NOT NULL
            )",
            [],
        )?;

        Ok(())
    }
}
