use std::sync::atomic::AtomicU64;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use dashmap::{DashMap, DashSet};
use reqwest::Client;
use songbird::driver::Driver;
use tokio::sync::broadcast;

use crate::dsp::DspParams;
use crate::models::{LoopMode, PlayerEvent};
use crate::rate_limiter::RateLimiter;
use crate::sources::{FlatPlaylistItem, SourceResolver, TrackMetadata};

#[derive(Clone, Debug, Default)]
pub struct GuildPlayOptions {
    pub extractor_args: Option<String>,
    pub extra_args: Option<String>,
    pub cookies: Option<String>,
    pub format: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone)]
pub struct AppState {
    pub players: Arc<DashMap<u64, Arc<tokio::sync::Mutex<Driver>>>>,
    pub dsp_params: Arc<DashMap<u64, Arc<RwLock<DspParams>>>>,
    pub http_client: Client,
    pub resolver: Arc<SourceResolver>,
    pub start_time: Instant,
    pub load_epochs: Arc<DashMap<u64, Arc<AtomicU64>>>,
    pub pending_playlists: Arc<DashMap<u64, Vec<FlatPlaylistItem>>>,
    pub guild_volumes: Arc<DashMap<u64, f32>>,
    pub loop_modes: Arc<DashMap<u64, LoopMode>>,
    pub autoplay_guilds: Arc<DashSet<u64>>,
    pub seen_tracks: Arc<DashMap<u64, Vec<String>>>,
    pub last_tracks: Arc<DashMap<u64, TrackMetadata>>,
    pub last_activity: Arc<DashMap<u64, Instant>>,
    pub prefetching_guilds: Arc<DashSet<u64>>,
    pub guild_play_options: Arc<DashMap<u64, GuildPlayOptions>>,
    pub event_tx: broadcast::Sender<PlayerEvent>,
    pub auth_token: Option<Arc<String>>,
    pub rate_limiter: Arc<RateLimiter>,
}

impl AppState {
    pub fn new() -> Self {
        // Buffer aumentado de 256 a 8192 para reducir eventos droppeados
        // cuando hay múltiples clientes WebSocket lentos.
        let (event_tx, _) = broadcast::channel(8192);
        
        // Rate limiter configurado por environment variables (defaults seguros para producción)
        let rate_limiter = Arc::new(RateLimiter::new(
            crate::rate_limiter::RateLimitConfig::from_env()
        ));
        
        let auth_token = match std::env::var("AXAUDIO_PASSWORD") {
            Ok(val) => {
                let trimmed = val.trim().to_string();
                if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
                    None
                } else {
                    Some(Arc::new(trimmed))
                }
            }
            Err(_) => Some(Arc::new("youshallnotpass".to_string())),
        };

        let default_ua = std::env::var("HTTP_USER_AGENT").unwrap_or_else(|_| {
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36".to_string()
        });

        Self {
            players: Arc::new(DashMap::new()),
            dsp_params: Arc::new(DashMap::new()),
            http_client: Client::builder()
                .user_agent(default_ua)
                .build()
                .unwrap_or_else(|_| Client::new()),
            resolver: Arc::new(SourceResolver::new()),
            start_time: Instant::now(),
            load_epochs: Arc::new(DashMap::new()),
            pending_playlists: Arc::new(DashMap::new()),
            guild_volumes: Arc::new(DashMap::new()),
            loop_modes: Arc::new(DashMap::new()),
            autoplay_guilds: Arc::new(DashSet::new()),
            seen_tracks: Arc::new(DashMap::new()),
            last_tracks: Arc::new(DashMap::new()),
            last_activity: Arc::new(DashMap::new()),
            prefetching_guilds: Arc::new(DashSet::new()),
            guild_play_options: Arc::new(DashMap::new()),
            event_tx,
            auth_token,
            rate_limiter,
        }
    }

    pub fn get_volume(&self, guild_id: u64) -> f32 {
        self.guild_volumes
            .get(&guild_id)
            .map(|v| *v)
            .unwrap_or(1.0)
    }

    pub fn set_volume(&self, guild_id: u64, vol: f32) {
        self.guild_volumes.insert(guild_id, vol);
    }

    pub fn get_loop_mode(&self, guild_id: u64) -> LoopMode {
        self.loop_modes
            .get(&guild_id)
            .map(|m| *m)
            .unwrap_or(LoopMode::Off)
    }

    pub fn set_loop_mode(&self, guild_id: u64, mode: LoopMode) {
        self.loop_modes.insert(guild_id, mode);
    }

    pub fn is_autoplay(&self, guild_id: u64) -> bool {
        self.autoplay_guilds.contains(&guild_id)
    }

    pub fn set_autoplay(&self, guild_id: u64, enabled: bool) {
        if enabled {
            self.autoplay_guilds.insert(guild_id);
        } else {
            self.autoplay_guilds.remove(&guild_id);
        }
    }

    pub fn is_track_seen(&self, guild_id: u64, video_id: &str) -> bool {
        if let Some(list) = self.seen_tracks.get(&guild_id) {
            list.contains(&video_id.to_string())
        } else {
            false
        }
    }

    pub fn record_seen_track(&self, guild_id: u64, video_id: &str) {
        let mut entry = self.seen_tracks.entry(guild_id).or_insert_with(Vec::new);
        if !entry.contains(&video_id.to_string()) {
            entry.push(video_id.to_string());
            if entry.len() > 60 {
                let overflow = entry.len() - 60;
                entry.drain(0..overflow);
            }
        }
    }

    pub fn clear_seen_tracks(&self, guild_id: u64) {
        self.seen_tracks.remove(&guild_id);
    }

    pub fn set_last_track(&self, guild_id: u64, track: TrackMetadata) {
        self.last_tracks.insert(guild_id, track);
    }

    pub fn get_last_track(&self, guild_id: u64) -> Option<TrackMetadata> {
        self.last_tracks.get(&guild_id).map(|t| t.clone())
    }

    pub fn is_prefetching(&self, guild_id: u64) -> bool {
        self.prefetching_guilds.contains(&guild_id)
    }

    pub fn start_prefetch(&self, guild_id: u64) -> bool {
        self.prefetching_guilds.insert(guild_id)
    }

    pub fn end_prefetch(&self, guild_id: u64) {
        self.prefetching_guilds.remove(&guild_id);
    }

    pub fn get_dsp_params(&self, guild_id: u64) -> Arc<RwLock<DspParams>> {
        let entry = self
            .dsp_params
            .entry(guild_id)
            .or_insert_with(|| Arc::new(RwLock::new(DspParams::default())));
        entry.value().clone()
    }

    pub fn next_epoch(&self, guild_id: u64) -> u64 {
        let entry = self
            .load_epochs
            .entry(guild_id)
            .or_insert_with(|| Arc::new(AtomicU64::new(0)));
        entry.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }

    #[allow(dead_code)]
    pub fn current_epoch(&self, guild_id: u64) -> u64 {
        self.load_epochs
            .get(&guild_id)
            .map(|e| e.load(std::sync::atomic::Ordering::SeqCst))
            .unwrap_or(0)
    }

    pub fn touch_activity(&self, guild_id: u64) {
        self.last_activity.insert(guild_id, Instant::now());
    }

    /// Retorna la última actividad del guild.
    /// Si no hay entrada, retorna `Instant::now() - Duration::MAX` (prácticamente el inicio del runtime),
    /// lo que hace que el GC detecte inmediatamente el guild como inactivo.
    pub fn get_last_activity(&self, guild_id: u64) -> Instant {
        self.last_activity
            .get(&guild_id)
            .map(|t| *t.value())
            .unwrap_or_else(|| Instant::now() - std::time::Duration::MAX)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
