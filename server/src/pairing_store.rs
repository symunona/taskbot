use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PairingToken {
    pub token: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub used: bool,
    pub used_by: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuthorizedClient {
    pub client_id: String,
    pub client_token: String,
    pub paired_at: u64,
    pub last_seen: Option<u64>,
    pub label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct PairingStoreData {
    pub pairing_tokens: HashMap<String, PairingToken>,
    pub authorized_clients: HashMap<String, AuthorizedClient>,
}

pub struct PairingStore {
    path: PathBuf,
    data: PairingStoreData,
}

impl PairingStore {
    pub fn new(path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let data = if path.exists() {
            let content = fs::read_to_string(&path)?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            PairingStoreData::default()
        };
        Ok(Self { path, data })
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let content = serde_json::to_string_pretty(&self.data)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, content)?;
        Ok(())
    }

    pub fn create_pairing_token(&mut self, ttl_secs: u64) -> Result<PairingToken, Box<dyn std::error::Error>> {
        let token = uuid::Uuid::new_v4().to_string();
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let expires_at = now + ttl_secs;

        let pt = PairingToken {
            token: token.clone(),
            created_at: now,
            expires_at,
            used: false,
            used_by: None,
        };

        self.data.pairing_tokens.insert(token.clone(), pt.clone());
        self.save()?;
        Ok(pt)
    }

    pub fn validate_and_consume_pairing_token(&mut self, token: &str) -> Result<AuthorizedClient, String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        
        if let Some(pt) = self.data.pairing_tokens.get_mut(token) {
            if pt.used {
                return Err("token_already_used".to_string());
            }
            if now > pt.expires_at {
                return Err("token_expired".to_string());
            }
            
            let client_id = uuid::Uuid::new_v4().to_string();
            let client_token = uuid::Uuid::new_v4().to_string();
            
            pt.used = true;
            pt.used_by = Some(client_id.clone());
            
            let ac = AuthorizedClient {
                client_id: client_id.clone(),
                client_token: client_token.clone(),
                paired_at: now,
                last_seen: Some(now),
                label: None,
            };
            
            self.data.authorized_clients.insert(client_token, ac.clone());
            self.save().map_err(|e| e.to_string())?;
            
            Ok(ac)
        } else {
            Err("invalid_token".to_string())
        }
    }

    pub fn validate_client_token(&mut self, client_token: &str) -> Result<(), String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        if let Some(ac) = self.data.authorized_clients.get_mut(client_token) {
            ac.last_seen = Some(now);
            self.save().map_err(|e| e.to_string())?;
            Ok(())
        } else {
            Err("invalid_token".to_string())
        }
    }

    pub fn list_clients(&self) -> Vec<AuthorizedClient> {
        self.data.authorized_clients.values().cloned().collect()
    }

    pub fn revoke_client(&mut self, client_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let token_to_remove = self.data.authorized_clients.iter()
            .find(|(_, ac)| ac.client_id == client_id)
            .map(|(token, _)| token.clone());
            
        if let Some(token) = token_to_remove {
            self.data.authorized_clients.remove(&token);
            self.save()?;
        }
        Ok(())
    }
}