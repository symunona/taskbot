use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthPayload {
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthEvent {
    pub event: String,
    pub payload: AuthPayload,
}

pub fn is_valid_auth_message(msg: &str, expected_token: &str) -> bool {
    if let Ok(event) = serde_json::from_str::<AuthEvent>(msg) {
        if event.event == "auth.token" && event.payload.token == expected_token {
            return true;
        }
    }
    false
}
