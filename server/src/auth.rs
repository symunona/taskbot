use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthPayload {
    pub token: Option<String>,
    pub client_token: Option<String>,
    pub pairing_token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthEvent {
    pub event: String,
    pub payload: AuthPayload,
}
