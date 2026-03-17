use notify::{RecursiveMode, Result, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::channel;

pub struct FileWatcher {
    vault_path: PathBuf,
}

impl FileWatcher {
    pub fn new(vault_path: PathBuf) -> Self {
        Self { vault_path }
    }

    pub fn watch(&self) -> Result<()> {
        let (tx, rx) = channel();

        let mut watcher = notify::recommended_watcher(tx)?;

        watcher.watch(&self.vault_path, RecursiveMode::Recursive)?;

        std::thread::spawn(move || {
            for res in rx {
                match res {
                    Ok(event) => println!("event: {:?}", event),
                    Err(e) => println!("watch error: {:?}", e),
                }
            }
        });

        Ok(())
    }
}
