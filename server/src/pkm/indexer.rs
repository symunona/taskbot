use rusqlite::Connection;
use serde_yaml::Value;
use std::fs;
use std::path::PathBuf;
use walkdir::WalkDir;

pub struct Indexer {
    vault_path: PathBuf,
}

impl Indexer {
    pub fn new(vault_path: PathBuf) -> Self {
        Self { vault_path }
    }

    pub fn index_vault(&self, conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
        for entry in WalkDir::new(&self.vault_path)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.path().extension().is_some_and(|ext| ext == "md") {
                let content = fs::read_to_string(entry.path())?;
                let filename = entry.file_name().to_string_lossy().to_string();
                let path_str = entry.path().to_string_lossy().to_string();

                let (frontmatter, body) = Self::parse_frontmatter(&content);
                let keywords = Self::extract_keywords(&frontmatter);

                let metadata = fs::metadata(entry.path())?;
                let created = metadata.created().unwrap_or(std::time::SystemTime::now());
                let modified = metadata.modified().unwrap_or(std::time::SystemTime::now());

                let created_str = format!("{:?}", created);
                let modified_ts = modified.duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64;

                conn.execute(
                    "INSERT OR REPLACE INTO files (path, filename, content, frontmatter, keywords, created, last_modified)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    (
                        &path_str,
                        &filename,
                        &body,
                        &frontmatter,
                        &keywords,
                        &created_str,
                        &modified_ts,
                    ),
                )?;
            }
        }
        Ok(())
    }

    fn parse_frontmatter(content: &str) -> (String, String) {
        if let Some(stripped) = content.strip_prefix("---\n") {
            if let Some(end_idx) = stripped.find("---\n") {
                let frontmatter = &stripped[..end_idx];
                let body = &stripped[end_idx + 4..];
                return (frontmatter.to_string(), body.to_string());
            }
        }
        (String::new(), content.to_string())
    }

    fn extract_keywords(frontmatter: &str) -> String {
        if let Ok(docs) = serde_yaml::from_str::<Value>(frontmatter) {
            if let Some(keywords) = docs.get("keywords") {
                if let Some(seq) = keywords.as_sequence() {
                    let kws: Vec<String> = seq
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                    return kws.join(", ");
                }
            }
        }
        String::new()
    }
}
