use std::path::PathBuf;
use std::fs;
use walkdir::WalkDir;

pub struct FsManager {
    vault_path: PathBuf,
}

impl FsManager {
    pub fn new(vault_path: PathBuf) -> Self {
        Self { vault_path }
    }

    fn resolve_path(&self, rel_path: &str) -> Result<PathBuf, String> {
        let path = self.vault_path.join(rel_path);
        // Basic security check to prevent path traversal
        if !path.starts_with(&self.vault_path) {
            return Err("Invalid path".to_string());
        }
        Ok(path)
    }

    pub fn list_directory(&self, path: &str) -> Result<Vec<String>, String> {
        let full_path = self.resolve_path(path)?;
        let mut items = Vec::new();
        
        if full_path.is_dir() {
            if let Ok(entries) = fs::read_dir(full_path) {
                for entry in entries.filter_map(|e| e.ok()) {
                    if let Some(name) = entry.file_name().to_str() {
                        items.push(name.to_string());
                    }
                }
            }
        }
        Ok(items)
    }

    pub fn list_vault_files(&self, filter: Option<&str>) -> Result<Vec<String>, String> {
        let mut items = Vec::new();
        for entry in WalkDir::new(&self.vault_path).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                let rel_path = entry.path().strip_prefix(&self.vault_path)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .into_owned();
                
                if let Some(f) = filter {
                    if rel_path.contains(f) {
                        items.push(rel_path);
                    }
                } else {
                    items.push(rel_path);
                }
            }
        }
        Ok(items)
    }

    pub fn get_folder_tree(&self) -> Result<Vec<String>, String> {
        let mut items = Vec::new();
        for entry in WalkDir::new(&self.vault_path).into_iter().filter_map(|e| e.ok()) {
            let rel_path = entry.path().strip_prefix(&self.vault_path)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .into_owned();
            items.push(rel_path);
        }
        Ok(items)
    }

    pub fn create_directory(&self, path: &str) -> Result<(), String> {
        let full_path = self.resolve_path(path)?;
        fs::create_dir_all(full_path).map_err(|e| e.to_string())
    }

    pub fn read_file(&self, path: &str) -> Result<String, String> {
        let full_path = self.resolve_path(path)?;
        fs::read_to_string(full_path).map_err(|e| e.to_string())
    }

    pub fn create_file(&self, path: &str, content: &str) -> Result<(), String> {
        let full_path = self.resolve_path(path)?;
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(full_path, content).map_err(|e| e.to_string())
    }

    pub fn update_file(&self, path: &str, content: &str) -> Result<(), String> {
        self.create_file(path, content) // For now, overwrite is same as create
    }

    pub fn edit_file(&self, path: &str, start_line: usize, end_line: usize, new_content: &str) -> Result<(), String> {
        let content = self.read_file(path)?;
        let mut lines: Vec<&str> = content.lines().collect();
        
        let start_idx = if start_line > 0 { start_line - 1 } else { 0 };
        let end_idx = end_line;

        if start_idx > lines.len() || end_idx < start_idx {
            return Err("Invalid line range".to_string());
        }

        let end_idx = std::cmp::min(end_idx, lines.len());

        let new_lines: Vec<&str> = new_content.lines().collect();
        lines.splice(start_idx..end_idx, new_lines);

        let final_content = lines.join("\n");
        self.update_file(path, &final_content)
    }

    pub fn delete_file(&self, path: &str) -> Result<(), String> {
        let full_path = self.resolve_path(path)?;
        fs::remove_file(full_path).map_err(|e| e.to_string())
    }

    pub fn rename_file(&self, path: &str, new_path: &str) -> Result<(), String> {
        let full_path = self.resolve_path(path)?;
        let new_full_path = self.resolve_path(new_path)?;
        
        if let Some(parent) = new_full_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        
        fs::rename(full_path, new_full_path).map_err(|e| e.to_string())
    }

    pub fn move_file(&self, path: &str, dest_dir: &str) -> Result<(), String> {
        let full_path = self.resolve_path(path)?;
        let file_name = full_path.file_name().ok_or("Invalid file name")?;
        
        let dest_path = self.resolve_path(dest_dir)?.join(file_name);
        
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        
        fs::rename(full_path, dest_path).map_err(|e| e.to_string())
    }
}
