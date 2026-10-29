use serde::{Deserialize, Serialize};
use crate::sources::FlatPlaylistItem;
use crate::dsp::DspStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerKey {
    pub guild_id: u64,
    pub bot_id: u64,
}

impl PlayerKey {
    pub fn new(guild_id: u64, bot_id: u64) -> Self {
        Self { guild_id, bot_id }
    }
}

impl std::fmt::Display for PlayerKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.bot_id == 0 {
            write!(f, "guild:{}", self.guild_id)
        } else {
            write!(f, "guild:{}:bot:{}", self.guild_id, self.bot_id)
        }
    }
}

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

pub fn deserialize_optional_u64_flexible<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct OptionalStringOrNumVisitor;
    impl<'de> serde::de::Visitor<'de> for OptionalStringOrNumVisitor {
        type Value = Option<u64>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an optional u64 integer, numeric string, or null")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Some(v))
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if v >= 0 {
                Ok(Some(v as u64))
            } else {
                Err(E::custom("snowflake ID cannot be negative"))
            }
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            let trimmed = v.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
                Ok(None)
            } else {
                trimmed.parse::<u64>().map(Some).map_err(E::custom)
            }
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            deserializer.deserialize_any(self)
        }
    }
    deserializer.deserialize_option(OptionalStringOrNumVisitor)
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
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "bot_id")]
    pub bot_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct PlayPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
    pub url: String,
    pub volume: Option<f32>,
    pub replace: Option<bool>,
    pub extractor_args: Option<String>,
    pub extra_args: Option<String>,
    pub cookies: Option<String>,
    pub format: Option<String>,
    pub limit: Option<usize>,
    pub user_agent: Option<String>,
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
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
    pub action: String, // "pause", "resume", "stop", "skip", "volume", "clear", "seek", "loop", "shuffle", "remove", "move", "247"
    pub value: Option<f32>,           // volume (0.0 to 2.0) or seek seconds
    pub position_secs: Option<f64>,   // seek position in seconds
    pub mode: Option<String>,         // loop mode: "off" / "none", "track", "queue"
    pub index: Option<usize>,         // remove index (0-based in upcoming queue)
    pub from_index: Option<usize>,    // move source index
    pub to_index: Option<usize>,      // move destination index
    pub enabled: Option<bool>,        // autoplay or 24/7 enable/disable toggle
}

#[derive(Debug, Deserialize)]
pub struct AutoplayPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct TwentyFourSevenPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct LeavePayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct QueuePayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ResolvePayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
    pub query: String,
    pub extractor_args: Option<String>,
    pub extra_args: Option<String>,
    pub cookies: Option<String>,
    pub format: Option<String>,
    pub limit: Option<usize>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SearchPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_live: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_preset: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_id: Option<u64>,
    pub is_playing: bool,
    pub current: Option<TrackResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_position_secs: Option<f64>,
    pub loop_mode: String,
    pub autoplay: bool,
    pub twenty_four_seven: bool,
    pub queue: Vec<TrackResponse>,
    pub total_tracks: usize,
}

#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_seconds: u64,
    pub active_players: usize,
    pub active_247_players: usize,
    pub tracks_played_total: u64,
    pub resolves_total: u64,
    pub cached_tracks: usize,
    pub custom_presets_count: usize,
    pub memory_rss_bytes: u64,
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
        #[serde(skip_serializing_if = "Option::is_none")]
        bot_id: Option<u64>,
        track: TrackResponse,
    },
    TrackEnd {
        guild_id: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        bot_id: Option<u64>,
        track: Option<TrackResponse>,
        reason: String,
    },
    TrackError {
        guild_id: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        bot_id: Option<u64>,
        track: Option<TrackResponse>,
        error: String,
    },
    MetadataUpdate {
        guild_id: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        bot_id: Option<u64>,
        title: Option<String>,
        artist: Option<String>,
        raw: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct LiveRadioPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
    pub url: String,
    pub title: Option<String>,
    pub volume: Option<f32>,
    pub user_agent: Option<String>,
}

use crate::dsp::{
    BandSetting, LoudnessSettings, ModulationSettings, ParametricBand, ReverbSettings,
    SpatialSettings, TimescaleSettings,
};

#[derive(Debug, Deserialize)]
pub struct EqPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
    pub bands: Option<Vec<BandSetting>>,
    pub parametric: Option<Vec<ParametricBand>>,
    pub dolby_bands: Option<Vec<f32>>,
    pub iso31_bands: Option<Vec<f32>>,
    pub preset: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DspPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
    pub preset: Option<String>,
    pub spatial: Option<SpatialSettings>,
    pub modulation: Option<ModulationSettings>,
    pub loudness: Option<LoudnessSettings>,
    pub timescale: Option<TimescaleSettings>,
    pub reverb: Option<ReverbSettings>,
    pub bands: Option<Vec<BandSetting>>,
    pub parametric: Option<Vec<ParametricBand>>,
    pub dolby_bands: Option<Vec<f32>>,
    pub iso31_bands: Option<Vec<f32>>,
}

#[derive(Debug, Deserialize)]
pub struct ReverbPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
    #[serde(flatten)]
    pub settings: ReverbSettings,
}

#[derive(Debug, Deserialize)]
pub struct SpatialPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
    #[serde(flatten)]
    pub settings: SpatialSettings,
}

#[derive(Debug, Deserialize)]
pub struct TimescalePayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
    #[serde(flatten)]
    pub settings: TimescaleSettings,
}

#[derive(Debug, Deserialize)]
pub struct LoudnessPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
    #[serde(flatten)]
    pub settings: LoudnessSettings,
}

#[derive(Debug, Deserialize)]
pub struct ModulationPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
    #[serde(flatten)]
    pub settings: ModulationSettings,
}

#[derive(Debug, Deserialize)]
pub struct DspResetPayload {
    #[serde(deserialize_with = "deserialize_u64_flexible")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible", alias = "user_id", alias = "bot")]
    pub bot_id: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct DspResponse {
    pub guild_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_id: Option<u64>,
    pub status: DspStatus,
    pub message: String,
}

/// Payload para crear o actualizar un preset personalizado
#[derive(Debug, Deserialize)]
pub struct PresetSavePayload {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Si se proporciona, copia los parámetros DSP actuales del guild/bot al preset
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible")]
    pub guild_id: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_u64_flexible")]
    pub bot_id: Option<u64>,
}

/// Respuesta con info de un preset
#[derive(Debug, Serialize, Clone)]
pub struct PresetInfo {
    pub name: String,
    pub description: Option<String>,
    pub status: DspStatus,
}

/// Respuesta para listar presets
#[derive(Debug, Serialize)]
pub struct PresetListResponse {
    pub presets: Vec<PresetInfo>,
    pub count: usize,
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn test_player_key_display_and_equality() {
        let key1 = PlayerKey::new(12345, 67890);
        let key2 = PlayerKey::new(12345, 67890);
        let key3 = PlayerKey::new(12345, 0);

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
        assert_eq!(key1.to_string(), "guild:12345:bot:67890");
        assert_eq!(key3.to_string(), "guild:12345");
    }

    #[test]
    fn test_deserialize_optional_u64_flexible() {
        #[derive(Deserialize)]
        struct TestItem {
            #[serde(default, deserialize_with = "deserialize_optional_u64_flexible")]
            val: Option<u64>,
        }

        let from_num: TestItem = serde_json::from_str(r#"{"val": 998877}"#).unwrap();
        assert_eq!(from_num.val, Some(998877));

        let from_str: TestItem = serde_json::from_str(r#"{"val": "998877"}"#).unwrap();
        assert_eq!(from_str.val, Some(998877));

        let from_null: TestItem = serde_json::from_str(r#"{"val": null}"#).unwrap();
        assert_eq!(from_null.val, None);

        let from_empty_str: TestItem = serde_json::from_str(r#"{"val": ""}"#).unwrap();
        assert_eq!(from_empty_str.val, None);

        let from_missing: TestItem = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(from_missing.val, None);
    }

    #[test]
    fn test_play_payload_bot_id_aliases() {
        let json_bot_id = r#"{"guild_id": "111", "bot_id": "222", "url": "https://youtu.be/test"}"#;
        let p1: PlayPayload = serde_json::from_str(json_bot_id).unwrap();
        assert_eq!(p1.guild_id, 111);
        assert_eq!(p1.bot_id, Some(222));

        let json_user_id = r#"{"guild_id": 111, "user_id": 222, "url": "https://youtu.be/test"}"#;
        let p2: PlayPayload = serde_json::from_str(json_user_id).unwrap();
        assert_eq!(p2.guild_id, 111);
        assert_eq!(p2.bot_id, Some(222));

        let json_no_bot = r#"{"guild_id": 111, "url": "https://youtu.be/test"}"#;
        let p3: PlayPayload = serde_json::from_str(json_no_bot).unwrap();
        assert_eq!(p3.guild_id, 111);
        assert_eq!(p3.bot_id, None);
    }
}



