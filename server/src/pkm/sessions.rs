use rusqlite::{Connection, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub client_id: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: u64,
    pub last_seen: u64,
}

pub struct SessionManager {
    db_path: PathBuf,
}

impl SessionManager {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    fn get_conn(&self) -> Result<Connection> {
        Connection::open(&self.db_path)
    }

    pub fn create_session(
        &self,
        client_id: &str,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<Session> {
        let conn = self.get_conn()?;
        let id = Uuid::new_v4().to_string();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        conn.execute(
            "INSERT INTO sessions (id, client_id, ip_address, user_agent, created_at, last_seen) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (&id, client_id, ip_address, user_agent, now, now),
        )?;

        Ok(Session {
            id,
            client_id: client_id.to_string(),
            ip_address: ip_address.map(|s| s.to_string()),
            user_agent: user_agent.map(|s| s.to_string()),
            created_at: now,
            last_seen: now,
        })
    }

    pub fn update_last_seen(&self, id: &str) -> Result<()> {
        let conn = self.get_conn()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        conn.execute(
            "UPDATE sessions SET last_seen = ?1 WHERE id = ?2",
            (now, id),
        )?;

        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, client_id, ip_address, user_agent, created_at, last_seen FROM sessions ORDER BY last_seen DESC"
        )?;

        let sessions = stmt.query_map([], |row| {
            Ok(Session {
                id: row.get(0)?,
                client_id: row.get(1)?,
                ip_address: row.get(2)?,
                user_agent: row.get(3)?,
                created_at: row.get(4)?,
                last_seen: row.get(5)?,
            })
        })?;

        let mut result = Vec::new();
        for session in sessions {
            result.push(session?);
        }

        Ok(result)
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        let conn = self.get_conn()?;
        conn.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        Ok(())
    }
}
