use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Instant;
use dashmap::DashMap;
use reqwest::Client;
use songbird::driver::Driver;
use tokio::sync::Mutex;

use crate::sources::{FlatPlaylistItem, SourceResolver};

#[derive(Clone)]
pub struct AppState {
    pub players: Arc<DashMap<u64, Arc<Mutex<Driver>>>>,
    pub http_client: Client,
    pub resolver: Arc<SourceResolver>,
    pub start_time: Instant,
    pub load_epochs: Arc<DashMap<u64, Arc<AtomicU64>>>,
    pub pending_playlists: Arc<DashMap<u64, Vec<FlatPlaylistItem>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            players: Arc::new(DashMap::new()),
            http_client: Client::builder()
                .user_agent("Mozilla/5.0 (compatible; axAudio/0.1.0)")
                .build()
                .unwrap_or_else(|_| Client::new()),
            resolver: Arc::new(SourceResolver::new()),
            start_time: Instant::now(),
            load_epochs: Arc::new(DashMap::new()),
            pending_playlists: Arc::new(DashMap::new()),
        }
    }

    pub fn next_epoch(&self, guild_id: u64) -> u64 {
        let entry = self
            .load_epochs
            .entry(guild_id)
            .or_insert_with(|| Arc::new(AtomicU64::new(0)));
        entry.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }

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
