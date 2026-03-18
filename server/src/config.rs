use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct KeyConfig {
    #[serde(default)]
    pub gemini_api_key: Option<String>,
    #[serde(default)]
    pub google_api_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub listen_addr: String,
    pub vault_path: PathBuf,
    pub token: String,
    #[serde(default)]
    pub google_api_key: Option<String>,
    #[serde(default)]
    pub keys: KeyConfig,
    pub public_domain: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeminiApiKeySource {
    EnvGeminiApiKey,
    EnvGoogleApiKey,
    ConfigKeysGeminiApiKey,
    ConfigKeysGoogleApiKey,
    LegacyConfigGoogleApiKey,
}

impl GeminiApiKeySource {
    pub fn description(self) -> &'static str {
        match self {
            Self::EnvGeminiApiKey => "environment variable GEMINI_API_KEY",
            Self::EnvGoogleApiKey => "environment variable GOOGLE_API_KEY",
            Self::ConfigKeysGeminiApiKey => "config file [keys].gemini_api_key",
            Self::ConfigKeysGoogleApiKey => "config file [keys].google_api_key",
            Self::LegacyConfigGoogleApiKey => "legacy config field google_api_key",
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:55566".to_string(),
            vault_path: PathBuf::from("./vault"),
            token: uuid::Uuid::new_v4().to_string(),
            google_api_key: None,
            keys: KeyConfig::default(),
            public_domain: None,
        }
    }
}

impl Config {
    pub fn load(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        if path.exists() {
            let content = fs::read_to_string(path)?;
            Ok(toml::from_str(&content)?)
        } else {
            let config = Self::default();
            config.save(path)?;
            Ok(config)
        }
    }

    pub fn resolved_google_api_key(&self) -> Option<String> {
        self.resolved_google_api_key_with_source()
            .map(|(key, _)| key)
    }

    pub fn resolved_google_api_key_with_source(&self) -> Option<(String, GeminiApiKeySource)> {
        let env_gemini = std::env::var("GEMINI_API_KEY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(value) = env_gemini {
            return Some((value, GeminiApiKeySource::EnvGeminiApiKey));
        }

        let env_google = std::env::var("GOOGLE_API_KEY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(value) = env_google {
            return Some((value, GeminiApiKeySource::EnvGoogleApiKey));
        }

        let config_gemini = self
            .keys
            .gemini_api_key
            .clone()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(value) = config_gemini {
            return Some((value, GeminiApiKeySource::ConfigKeysGeminiApiKey));
        }

        let config_google = self
            .keys
            .google_api_key
            .clone()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(value) = config_google {
            return Some((value, GeminiApiKeySource::ConfigKeysGoogleApiKey));
        }

        self.google_api_key
            .clone()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| (value, GeminiApiKeySource::LegacyConfigGoogleApiKey))
    }

    pub fn save(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let content = toml::to_string(self)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        Ok(())
    }
}
