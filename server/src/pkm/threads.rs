use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ThreadFrontmatter {
    pub thread_id: String,
    pub keywords: Vec<String>,
    pub created: String,
    pub last_active: String,
    pub persona: String,
    pub status: String,
    pub summary: String,
    pub related_files: Vec<String>,
}

impl Default for ThreadFrontmatter {
    fn default() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        Self {
            thread_id: Uuid::new_v4().to_string(),
            keywords: vec![],
            created: now.to_string(),
            last_active: now.to_string(),
            persona: "default".to_string(),
            status: "active".to_string(),
            summary: "".to_string(),
            related_files: vec![],
        }
    }
}

pub struct ThreadManager {
    threads_dir: PathBuf,
}

impl ThreadManager {
    pub fn new(vault_path: &Path) -> Self {
        let threads_dir = vault_path.join("robot_memory/threads");

        fs::create_dir_all(&threads_dir).unwrap_or_default();

        Self {
            threads_dir,
        }
    }

    pub fn list_threads(&self) -> Result<Vec<ThreadFrontmatter>, Box<dyn std::error::Error>> {
        let mut threads = vec![];
        for entry in fs::read_dir(&self.threads_dir)?.flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "md") {
                let content = fs::read_to_string(entry.path())?;
                if let Some((fm, _)) = Self::parse_file(&content) {
                    if let Ok(frontmatter) = serde_yaml::from_str::<ThreadFrontmatter>(&fm) {
                        threads.push(frontmatter);
                    }
                }
            }
        }
        Ok(threads)
    }

    pub fn create_thread(&self) -> Result<ThreadFrontmatter, Box<dyn std::error::Error>> {
        let fm = ThreadFrontmatter::default();
        let filename = format!("{}-new-thread.md", fm.created);
        let path = self.threads_dir.join(filename);

        let content = format!("---\n{}\n---\n\n", serde_yaml::to_string(&fm)?);
        fs::write(path, content)?;

        Ok(fm)
    }

    pub fn get_thread(
        &self,
        thread_id: &str,
    ) -> Result<Option<(ThreadFrontmatter, String)>, Box<dyn std::error::Error>> {
        for entry in fs::read_dir(&self.threads_dir)?.flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "md") {
                let content = fs::read_to_string(entry.path())?;
                if let Some((fm_str, body)) = Self::parse_file(&content) {
                    if let Ok(fm) = serde_yaml::from_str::<ThreadFrontmatter>(&fm_str) {
                        if fm.thread_id == thread_id {
                            return Ok(Some((fm, body)));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    pub fn append_to_thread(
        &self,
        thread_id: &str,
        message: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for entry in fs::read_dir(&self.threads_dir)?.flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "md") {
                let content = fs::read_to_string(entry.path())?;
                if let Some((fm_str, body)) = Self::parse_file(&content) {
                    if let Ok(mut fm) = serde_yaml::from_str::<ThreadFrontmatter>(&fm_str) {
                        if fm.thread_id == thread_id {
                            fm.last_active = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_secs()
                                .to_string();
                            let new_content = format!(
                                "---\n{}\n---\n{}\n{}",
                                serde_yaml::to_string(&fm)?,
                                body,
                                message
                            );
                            fs::write(entry.path(), new_content)?;
                            return Ok(());
                        }
                    }
                }
            }
        }
        Err("Thread not found".into())
    }

    pub fn archive_segment(
        &self,
        thread_id: &str,
        summary: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for entry in fs::read_dir(&self.threads_dir)?.flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "md") {
                let content = fs::read_to_string(entry.path())?;
                if let Some((fm_str, body)) = Self::parse_file(&content) {
                    if let Ok(mut fm) = serde_yaml::from_str::<ThreadFrontmatter>(&fm_str) {
                        if fm.thread_id == thread_id {
                            let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                            
                            // Archive the current content
                            let mut archive_fm = fm.clone();
                            archive_fm.status = "archived".to_string();
                            if let Some(s) = summary {
                                archive_fm.summary = s.to_string();
                            }
                            let archive_content = format!("---\n{}\n---\n{}", serde_yaml::to_string(&archive_fm)?, body);
                            
                            use chrono::{DateTime, Utc};
                            let dt = DateTime::<Utc>::from(SystemTime::now());
                            let year = dt.format("%Y").to_string();
                            let month = dt.format("%m").to_string();
                            let date_str = dt.format("%Y-%m-%d").to_string();
                            
                            let slug = archive_fm.summary.to_lowercase().replace(" ", "-").chars().filter(|c| c.is_alphanumeric() || *c == '-').collect::<String>();
                            let slug = if slug.is_empty() { format!("segment-{}", timestamp) } else { slug };
                            
                            let archive_filename = format!("{}-{}.md", date_str, slug);
                            let target_dir = self.threads_dir.join(&year).join(&month);
                            fs::create_dir_all(&target_dir).unwrap_or_default();
                            
                            let archive_path = target_dir.join(archive_filename);
                            fs::write(&archive_path, archive_content)?;
                            
                            // Reset current thread body and update summary
                            if let Some(s) = summary {
                                fm.summary = s.to_string();
                            }
                            fm.last_active = timestamp.to_string();
                            let new_content = format!("---\n{}\n---\n", serde_yaml::to_string(&fm)?);
                            fs::write(entry.path(), new_content)?;
                            return Ok(());
                        }
                    }
                }
            }
        }
        Err("Thread not found".into())
    }

    pub fn close_thread(&self, thread_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        for entry in fs::read_dir(&self.threads_dir)?.flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "md") {
                let content = fs::read_to_string(entry.path())?;
                if let Some((fm_str, body)) = Self::parse_file(&content) {
                    if let Ok(mut fm) = serde_yaml::from_str::<ThreadFrontmatter>(&fm_str) {
                        if fm.thread_id == thread_id {
                            fm.status = "archived".to_string();
                            let new_content =
                                format!("---\n{}\n---\n{}", serde_yaml::to_string(&fm)?, body);
                                
                            use chrono::{DateTime, Utc};
                            let dt = DateTime::<Utc>::from(SystemTime::now());
                            let year = dt.format("%Y").to_string();
                            let month = dt.format("%m").to_string();
                            let date_str = dt.format("%Y-%m-%d").to_string();
                            
                            let slug = fm.summary.to_lowercase().replace(" ", "-").chars().filter(|c| c.is_alphanumeric() || *c == '-').collect::<String>();
                            let slug = if slug.is_empty() { format!("thread-{}", fm.created) } else { slug };
                            
                            let archive_filename = format!("{}-{}.md", date_str, slug);
                            let target_dir = self.threads_dir.join(&year).join(&month);
                            fs::create_dir_all(&target_dir).unwrap_or_default();
                            
                            let new_path = target_dir.join(archive_filename);
                            fs::write(&new_path, new_content)?;
                            fs::remove_file(entry.path())?;
                            return Ok(());
                        }
                    }
                }
            }
        }
        Err("Thread not found".into())
    }

    fn parse_file(content: &str) -> Option<(String, String)> {
        if let Some(stripped) = content.strip_prefix("---\n") {
            if let Some(end_idx) = stripped.find("---\n") {
                let frontmatter = stripped[..end_idx].to_string();
                let body = stripped[end_idx + 4..].to_string();
                return Some((frontmatter, body));
            }
        }
        None
    }
}
