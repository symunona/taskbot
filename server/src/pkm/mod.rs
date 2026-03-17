pub mod archival;
pub mod db;
pub mod indexer;
pub mod sessions;
pub mod threads;
pub mod watcher;

use std::path::PathBuf;

pub struct Pkm {
    pub db: db::Database,
    pub indexer: indexer::Indexer,
    pub watcher: watcher::FileWatcher,
    pub threads: threads::ThreadManager,
    pub archival: archival::ArchivalManager,
    pub sessions: sessions::SessionManager,
}

impl Pkm {
    pub fn new(vault_path: PathBuf, db_path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let db = db::Database::new(&db_path)?;
        let indexer = indexer::Indexer::new(vault_path.clone());
        let watcher = watcher::FileWatcher::new(vault_path.clone());
        let threads = threads::ThreadManager::new(&vault_path);
        let archival = archival::ArchivalManager::new(threads::ThreadManager::new(&vault_path));
        let sessions = sessions::SessionManager::new(db_path);

        Ok(Self {
            db,
            indexer,
            watcher,
            threads,
            archival,
            sessions,
        })
    }

    pub fn index_vault(&self) -> Result<(), Box<dyn std::error::Error>> {
        if let Ok(conn) = self.db.conn.lock() {
            self.indexer.index_vault(&conn)
        } else {
            Err("Failed to lock database".into())
        }
    }
}
