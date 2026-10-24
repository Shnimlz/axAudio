use serde::{Deserialize, Serialize};
use crate::sources::FlatPlaylistItem;

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
    pub replace: Option<bool>,
    pub extractor_args: Option<String>,
    pub extra_args: Option<String>,
    pub cookies: Option<String>,
    pub format: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopMode {
    #[default]
    Off,
    Track,
    Queue,
}

impl LoopMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            LoopMode::Off => "off",
            LoopMode::Track => "track",
            LoopMode::Queue => "queue",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ControlPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    pub action: String, // "pause", "resume", "stop", "skip", "volume", "clear", "seek", "loop", "shuffle", "remove", "move"
    pub value: Option<f32>,           // volume (0.0 to 2.0) or seek seconds
    pub position_secs: Option<f64>,   // seek position in seconds
    pub mode: Option<String>,         // loop mode: "off" / "none", "track", "queue"
    pub index: Option<usize>,         // remove index (0-based in upcoming queue)
    pub from_index: Option<usize>,    // move source index
    pub to_index: Option<usize>,      // move destination index
    pub enabled: Option<bool>,        // autoplay enable/disable toggle
}

#[derive(Debug, Deserialize)]
pub struct AutoplayPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct LeavePayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
}

#[derive(Debug, Deserialize)]
pub struct QueuePayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
}

#[derive(Debug, Deserialize)]
pub struct ResolvePayload {
    pub query: String,
    pub extractor_args: Option<String>,
    pub extra_args: Option<String>,
    pub cookies: Option<String>,
    pub format: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SearchPayload {
    pub query: String,
    pub limit: Option<usize>,
    pub source: Option<String>, // "auto" (default), "youtube", "soundcloud"
    pub extractor_args: Option<String>,
    pub extra_args: Option<String>,
    pub cookies: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SearchResponse {
    pub query: String,
    pub source_used: String,
    pub tracks: Vec<FlatPlaylistItem>,
}

#[derive(Debug, Serialize, Clone)]
pub struct TrackResponse {
    pub title: String,
    pub source_url: String,
    pub stream_url: String,
    pub duration_secs: Option<u64>,
    pub author: Option<String>,
    pub thumbnail: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PlayResponse {
    pub title: String,
    pub source_url: String,
    pub stream_url: String,
    pub duration_secs: Option<u64>,
    pub author: Option<String>,
    pub thumbnail: Option<String>,
    pub is_playlist: bool,
    pub playlist_title: Option<String>,
    pub playlist_tracks_count: usize,
    pub queue_length: usize,
    pub was_empty: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct QueueResponse {
    pub guild_id: u64,
    pub is_playing: bool,
    pub current: Option<TrackResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_position_secs: Option<f64>,
    pub loop_mode: String,
    pub autoplay: bool,
    pub queue: Vec<TrackResponse>,
    pub total_tracks: usize,
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

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlayerEvent {
    TrackStart {
        guild_id: u64,
        track: TrackResponse,
    },
    TrackEnd {
        guild_id: u64,
        track: Option<TrackResponse>,
        reason: String,
    },
    TrackError {
        guild_id: u64,
        track: Option<TrackResponse>,
        error: String,
    },
}

use crate::dsp::{
    BandSetting, DspStatus, LoudnessSettings, ModulationSettings, SpatialSettings, TimescaleSettings,
};

#[derive(Debug, Deserialize)]
pub struct EqPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    pub bands: Option<Vec<BandSetting>>,
    pub preset: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DspPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    pub preset: Option<String>,
    pub spatial: Option<SpatialSettings>,
    pub modulation: Option<ModulationSettings>,
    pub loudness: Option<LoudnessSettings>,
    pub timescale: Option<TimescaleSettings>,
    pub bands: Option<Vec<BandSetting>>,
}

#[derive(Debug, Serialize)]
pub struct DspResponse {
    pub guild_id: u64,
    pub status: DspStatus,
    pub message: String,
}


