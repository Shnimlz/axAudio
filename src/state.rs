use std::sync::atomic::AtomicU64;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use dashmap::DashMap;
use reqwest::Client;
use songbird::driver::Driver;
use tokio::sync::broadcast;

use crate::dsp::DspParams;
use crate::models::{LoopMode, PlayerEvent};
use crate::sources::{FlatPlaylistItem, SourceResolver};

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
    pub event_tx: broadcast::Sender<PlayerEvent>,
    pub auth_token: Option<Arc<String>>,
}

impl AppState {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(256);
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

        Self {
            players: Arc::new(DashMap::new()),
            dsp_params: Arc::new(DashMap::new()),
            http_client: Client::builder()
                .user_agent("Mozilla/5.0 (compatible; axAudio/0.1.0)")
                .build()
                .unwrap_or_else(|_| Client::new()),
            resolver: Arc::new(SourceResolver::new()),
            start_time: Instant::now(),
            load_epochs: Arc::new(DashMap::new()),
            pending_playlists: Arc::new(DashMap::new()),
            guild_volumes: Arc::new(DashMap::new()),
            loop_modes: Arc::new(DashMap::new()),
            event_tx,
            auth_token,
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
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
