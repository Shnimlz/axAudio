use std::sync::Arc;
use std::time::Instant;
use dashmap::DashMap;
use reqwest::Client;
use songbird::driver::Driver;
use tokio::sync::Mutex;

use crate::sources::SourceResolver;

#[derive(Clone)]
pub struct AppState {
    pub players: Arc<DashMap<u64, Arc<Mutex<Driver>>>>,
    pub http_client: Client,
    pub resolver: Arc<SourceResolver>,
    pub start_time: Instant,
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
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
