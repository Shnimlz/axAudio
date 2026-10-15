use serde::{Deserialize, Serialize};

pub fn deserialize_u64_flexible<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct StringOrNumVisitor;
    impl<'de> serde::de::Visitor<'de> for StringOrNumVisitor {
        type Value = u64;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a u64 integer or numeric string")
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
            Ok(v)
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if v >= 0 {
                Ok(v as u64)
            } else {
                Err(E::custom("snowflake ID cannot be negative"))
            }
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            v.parse::<u64>().map_err(E::custom)
        }
    }
    deserializer.deserialize_any(StringOrNumVisitor)
}

#[derive(Debug, Deserialize)]
pub struct JoinPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub channel_id: u64,
    pub session_id: String,
    pub token: String,
    pub endpoint: String,
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub user_id: u64,
}

#[derive(Debug, Deserialize)]
pub struct PlayPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    pub url: String,
    pub volume: Option<f32>,
}

#[derive(Debug, Deserialize)]
pub struct ControlPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    pub action: String, // "pause", "resume", "stop", "volume"
    pub value: Option<f32>, // used when action is "volume" (0.0 to 2.0)
}

#[derive(Debug, Deserialize)]
pub struct LeavePayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
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
