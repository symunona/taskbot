use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct HistoryManager {
    history_dir: PathBuf,
}

impl HistoryManager {
    pub fn new(vault_path: &Path) -> Self {
        let history_dir = vault_path.join("robot_memory/threads");
        fs::create_dir_all(&history_dir).unwrap_or_default();

        Self { history_dir }
    }

    pub fn save_transcript(
        &self,
        thread_id: &str,
        transcript: &str,
        summary: &str,
        is_closed: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use chrono::{DateTime, Utc};
        let dt = DateTime::<Utc>::from(SystemTime::now());
        let year = dt.format("%Y").to_string();
        let month = dt.format("%m").to_string();
        let date_str = dt.format("%Y-%m-%d").to_string();
        
        let slug = summary.to_lowercase().replace(" ", "-").chars().filter(|c| c.is_alphanumeric() || *c == '-').collect::<String>();
        let slug = if slug.is_empty() { format!("thread-{}", thread_id) } else { slug };
        
        let target_dir = self.history_dir.join(&year).join(&month);
        fs::create_dir_all(&target_dir).unwrap_or_default();
        
        let filename = format!("{}-{}.md", date_str, slug);
        let path = target_dir.join(&filename);
        
        let frontmatter = format!(
            "---\n\
            date: {}\n\
            thread_id: {}\n\
            persona: default\n\
            summary: \"{}\"\n\
            status: {}\n\
            ---\n",
            date_str, thread_id, summary, if is_closed { "closed" } else { "active" }
        );
        
        let content = format!("{}\n{}", frontmatter, transcript);
        fs::write(&path, content)?;
        
        // Update index file
        self.update_index(&year, &month, &date_str, &filename, summary)?;
        
        Ok(())
    }
    
    fn update_index(
        &self,
        year: &str,
        month: &str,
        date_str: &str,
        filename: &str,
        summary: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let index_path = self.history_dir.parent().unwrap().join("history-by-date.md");
        
        let mut content = if index_path.exists() {
            fs::read_to_string(&index_path)?
        } else {
            "# History by Date\n\n".to_string()
        };
        
        let year_heading = format!("## {}", year);
        if !content.contains(&year_heading) {
            content.push_str(&format!("\n{}\n", year_heading));
        }
        
        let month_heading = format!("### {}-{}", year, month);
        if !content.contains(&month_heading) {
            content.push_str(&format!("\n{}\n", month_heading));
        }
        
        let entry = format!("- [[{}|{} - {}]]", filename.replace(".md", ""), date_str, summary);
        if !content.contains(&entry) {
            content.push_str(&format!("{}\n", entry));
            fs::write(&index_path, content)?;
        }
        
        Ok(())
    }
}
