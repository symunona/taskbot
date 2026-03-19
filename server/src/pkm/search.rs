use std::path::PathBuf;
use walkdir::WalkDir;
use regex::Regex;

pub struct SearchManager {
    vault_path: PathBuf,
}

impl SearchManager {
    pub fn new(vault_path: PathBuf) -> Self {
        Self { vault_path }
    }

    pub fn search_regexp(&self, pattern: &str) -> Result<serde_json::Value, String> {
        let re = Regex::new(pattern).map_err(|e| e.to_string())?;
        let mut results = Vec::new();

        for entry in WalkDir::new(&self.vault_path).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if re.is_match(&content) {
                        let rel_path = entry.path().strip_prefix(&self.vault_path)
                            .unwrap_or(entry.path())
                            .to_string_lossy()
                            .into_owned();
                        
                        let first_match_line = content.lines().find(|l| re.is_match(l)).unwrap_or("");
                        
                        results.push(serde_json::json!({
                            "path": rel_path,
                            "match": first_match_line
                        }));
                    }
                }
            }
        }

        Ok(serde_json::Value::Array(results))
    }

    pub fn search_replace_file(&self, path: &str, search: &str, replace: &str) -> Result<(), String> {
        let full_path = self.vault_path.join(path);
        if !full_path.starts_with(&self.vault_path) {
            return Err("Invalid path".to_string());
        }

        let content = std::fs::read_to_string(&full_path).map_err(|e| e.to_string())?;
        let new_content = content.replace(search, replace);
        std::fs::write(&full_path, new_content).map_err(|e| e.to_string())
    }

    pub fn search_replace_global(&self, search: &str, replace: &str) -> Result<(), String> {
        for entry in WalkDir::new(&self.vault_path).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if content.contains(search) {
                        let new_content = content.replace(search, replace);
                        let _ = std::fs::write(entry.path(), new_content);
                    }
                }
            }
        }
        Ok(())
    }
}
