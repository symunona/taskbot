use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

pub const EXAMPLE: &str = include_str!("../config.example.toml");
pub const PERMISSION_MODES: &[&str] = &["acceptEdits", "bypassPermissions", "default", "plan"];

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_addr")]
    pub listen_addr: String,
    #[serde(default = "default_port")]
    pub listen_port: u16,
    pub vault_path: PathBuf,
    #[serde(default = "default_model")]
    pub gemini_model: String,
    /// Model the app uses for text mode.
    #[serde(default = "default_text_model")]
    pub gemini_text_model: String,
    /// Model for thread memory compacts. None = `gemini_text_model`.
    #[serde(default)]
    pub gemini_summary_model: Option<String>,
    /// Gemini API root for the server's own calls. Tests point it at a fake.
    #[serde(default = "default_base_url")]
    pub gemini_base_url: String,
    #[serde(default)]
    pub channel_dir: Option<PathBuf>,
    #[serde(default = "default_port_base")]
    pub channel_port_base: u16,
    #[serde(default)]
    pub public_urls: Vec<String>,
    /// Where Claude Code keeps its session files. Default `~/.claude/sessions`.
    #[serde(default)]
    pub claude_sessions_dir: Option<PathBuf>,
    #[serde(default)]
    pub keys: Keys,
    #[serde(default)]
    pub projects: BTreeMap<String, Project>,
}

#[derive(Deserialize, Clone, Default)]
pub struct Keys {
    #[serde(default)]
    pub gemini_api_key: Option<String>,
}

// Never let a key reach a log line through `{:?}`.
impl fmt::Debug for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = if self.gemini_api_key.is_some() {
            "<set>"
        } else {
            "<unset>"
        };
        f.debug_struct("Keys")
            .field("gemini_api_key", &state)
            .finish()
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub dir: PathBuf,
    #[serde(default = "default_mode")]
    pub permission_mode: String,
    /// `claude --model` value. None = Claude Code default.
    #[serde(default)]
    pub model: Option<String>,
}

fn default_addr() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    55566
}
fn default_model() -> String {
    "gemini-2.5-flash-native-audio-preview-12-2025".into()
}
fn default_text_model() -> String {
    "gemini-flash-latest".into()
}
fn default_base_url() -> String {
    "https://generativelanguage.googleapis.com".into()
}
fn default_port_base() -> u16 {
    55600
}
fn default_mode() -> String {
    "bypassPermissions".into()
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into()))
}

fn expand(p: &Path) -> PathBuf {
    match p.strip_prefix("~") {
        Ok(rest) => home().join(rest),
        Err(_) => p.to_path_buf(),
    }
}

pub fn config_path() -> PathBuf {
    match std::env::var_os("TALTOSBOT_CONFIG") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => home().join(".config/taltosbot/config.toml"),
    }
}

pub fn state_dir() -> PathBuf {
    match std::env::var_os("TALTOSBOT_STATE_DIR") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => home().join(".local/state/taltosbot"),
    }
}

pub enum LoadError {
    /// No file was there; a commented default has been written to this path.
    Created(PathBuf),
    Invalid(String),
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut cfg: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        cfg.vault_path = expand(&cfg.vault_path);
        cfg.channel_dir = cfg.channel_dir.as_deref().map(expand);
        cfg.claude_sessions_dir = cfg.claude_sessions_dir.as_deref().map(expand);
        for (name, p) in cfg.projects.iter_mut() {
            let ok = !name.is_empty()
                && name.len() <= 48
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !ok {
                return Err(format!(
                    "project name {name:?}: use only letters, digits, '-' and '_'"
                ));
            }
            if !PERMISSION_MODES.contains(&p.permission_mode.as_str()) {
                return Err(format!(
                    "project {name}: permission_mode {:?} is not one of {PERMISSION_MODES:?}",
                    p.permission_mode
                ));
            }
            if let Some(model) = &p.model {
                let ok = !model.is_empty()
                    && model.len() <= 80
                    && model
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_.[]:@/".contains(c));
                if !ok {
                    return Err(format!(
                        "project {name}: model {model:?}: use a Claude model alias or id"
                    ));
                }
            }
            p.dir = expand(&p.dir);
        }
        cfg.listen_addr.parse::<std::net::IpAddr>().map_err(|_| {
            format!(
                "listen_addr {:?} is not an IP address (the port goes in listen_port)",
                cfg.listen_addr
            )
        })?;
        Ok(cfg)
    }

    /// Load from `path`. A missing file is replaced by the commented default.
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text)
                .map_err(|e| LoadError::Invalid(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                write_private(path, EXAMPLE).map_err(|e| {
                    LoadError::Invalid(format!(
                        "cannot write default config {}: {e}",
                        path.display()
                    ))
                })?;
                Err(LoadError::Created(path.to_path_buf()))
            }
            Err(e) => Err(LoadError::Invalid(format!("{}: {e}", path.display()))),
        }
    }

    pub fn listen_ip(&self) -> std::net::IpAddr {
        self.listen_addr.parse().expect("validated in parse")
    }

    pub fn summary_model(&self) -> &str {
        self.gemini_summary_model
            .as_deref()
            .filter(|m| !m.is_empty())
            .unwrap_or(&self.gemini_text_model)
    }

    /// `$GEMINI_API_KEY` wins over `[keys].gemini_api_key`. Empty counts as unset.
    pub fn gemini_api_key(&self) -> Option<String> {
        std::env::var("GEMINI_API_KEY")
            .ok()
            .or_else(|| self.keys.gemini_api_key.clone())
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.claude_sessions_dir
            .clone()
            .unwrap_or_else(|| home().join(".claude/sessions"))
    }
}

/// Create `path` (and parents) readable by the owner only.
pub fn write_private(path: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(content.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_parses_and_has_no_secret() {
        let cfg = Config::parse(EXAMPLE).unwrap();
        assert_eq!(cfg.listen_port, 55566);
        assert_eq!(cfg.channel_port_base, 55600);
        assert!(cfg.keys.gemini_api_key.unwrap().is_empty());
        assert_eq!(cfg.gemini_text_model, "gemini-flash-latest");
    }

    #[test]
    fn text_model_default_and_override() {
        let cfg = Config::parse("vault_path='/v'").unwrap();
        assert_eq!(cfg.gemini_text_model, "gemini-flash-latest");
        let cfg = Config::parse("vault_path='/v'\ngemini_text_model='other'").unwrap();
        assert_eq!(cfg.gemini_text_model, "other");
    }

    #[test]
    fn permission_mode_defaults_to_bypass() {
        let cfg = Config::parse("vault_path='/v'\n[projects.a]\ndir='/x'\n[projects.b]\ndir='/y'\npermission_mode='plan'").unwrap();
        assert_eq!(cfg.projects["a"].permission_mode, "bypassPermissions");
        assert_eq!(cfg.projects["b"].permission_mode, "plan");
    }

    #[test]
    fn project_model_is_optional_and_checked() {
        let cfg = Config::parse(
            "vault_path='/v'\n[projects.a]\ndir='/x'\n[projects.b]\ndir='/y'\nmodel='haiku'",
        )
        .unwrap();
        assert_eq!(cfg.projects["a"].model, None);
        assert_eq!(cfg.projects["b"].model.as_deref(), Some("haiku"));
        for bad in ["", "a b", "x;rm"] {
            let text = format!("vault_path='/v'\n[projects.a]\ndir='/x'\nmodel='{bad}'");
            assert!(Config::parse(&text).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn missing_file_writes_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        assert!(matches!(Config::load(&path), Err(LoadError::Created(_))));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), EXAMPLE);
        assert!(Config::load(&path).is_ok());
    }

    #[test]
    fn rejects_old_keys_and_bad_projects() {
        assert!(Config::parse("vault_path='/v'\ntoken='x'").is_err());
        assert!(Config::parse("vault_path='/v'\n[projects.'a b']\ndir='/x'").is_err());
        assert!(
            Config::parse("vault_path='/v'\n[projects.a]\ndir='/x'\npermission_mode='yolo'")
                .is_err()
        );
        assert!(Config::parse("vault_path='/v'\nlisten_addr='0.0.0.0:55566'").is_err());
    }

    #[test]
    fn debug_does_not_print_key() {
        let cfg = Config::parse("vault_path='/v'\n[keys]\ngemini_api_key='sekrit'").unwrap();
        assert!(!format!("{cfg:?}").contains("sekrit"));
    }
}
