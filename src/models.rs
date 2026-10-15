use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct JoinPayload {
    pub guild_id: u64,
    pub channel_id: u64,
    pub session_id: String,
    pub token: String,
    pub endpoint: String,
    pub user_id: u64,
}

#[derive(Debug, Deserialize)]
pub struct PlayPayload {
    pub guild_id: u64,
    pub url: String,
    pub volume: Option<f32>,
}

#[derive(Debug, Deserialize)]
pub struct ControlPayload {
    pub guild_id: u64,
    pub action: String, // "pause", "resume", "stop", "volume"
    pub value: Option<f32>, // used when action is "volume" (0.0 to 2.0)
}

#[derive(Debug, Deserialize)]
pub struct LeavePayload {
    pub guild_id: u64,
}

#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_seconds: u64,
    pub active_players: usize,
}

#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub message: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            message: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            message: Some(msg.into()),
        }
    }
}
