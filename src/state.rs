use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use dashmap::{DashMap, DashSet};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use songbird::driver::Driver;
use tokio::sync::broadcast;

use crate::dsp::DspParams;
use crate::models::{LoopMode, PlayerEvent, PlayerKey};
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

/// Configuración persistente en disco por cada reproductor (guild + bot)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedPlayerConfig {
    #[serde(default)]
    pub volume: Option<f32>,
    #[serde(default)]
    pub loop_mode: Option<LoopMode>,
    #[serde(default)]
    pub twenty_four_seven: Option<bool>,
    #[serde(default)]
    pub autoplay: Option<bool>,
    #[serde(default)]
    pub dsp_params: Option<DspParams>,
}

#[derive(Clone)]
pub struct AppState {
    pub players: Arc<DashMap<PlayerKey, Arc<tokio::sync::Mutex<Driver>>>>,
    pub dsp_params: Arc<DashMap<PlayerKey, Arc<RwLock<DspParams>>>>,
    pub http_client: Client,
    pub resolver: Arc<SourceResolver>,
    pub start_time: Instant,
    pub load_epochs: Arc<DashMap<PlayerKey, Arc<AtomicU64>>>,
    pub pending_playlists: Arc<DashMap<PlayerKey, Vec<FlatPlaylistItem>>>,
    pub guild_volumes: Arc<DashMap<PlayerKey, f32>>,
    pub loop_modes: Arc<DashMap<PlayerKey, LoopMode>>,
    pub autoplay_guilds: Arc<DashSet<PlayerKey>>,
    pub twenty_four_seven_players: Arc<DashSet<PlayerKey>>,
    pub seen_tracks: Arc<DashMap<PlayerKey, Vec<String>>>,
    pub last_tracks: Arc<DashMap<PlayerKey, TrackMetadata>>,
    pub last_activity: Arc<DashMap<PlayerKey, Instant>>,
    pub prefetching_guilds: Arc<DashSet<PlayerKey>>,
    pub guild_play_options: Arc<DashMap<PlayerKey, GuildPlayOptions>>,
    pub event_tx: broadcast::Sender<PlayerEvent>,
    pub auth_token: Option<Arc<String>>,
    pub rate_limiter: Arc<RateLimiter>,
    /// Presets personalizados definidos por el usuario (nombre -> DspParams)
    pub custom_presets: Arc<DashMap<String, DspParams>>,
    /// Contador atómico de pistas reproducidas acumuladas
    pub tracks_played_count: Arc<AtomicU64>,
    /// Contador atómico de resoluciones de pistas ejecutadas
    pub resolves_count: Arc<AtomicU64>,
    /// Contador y marca de tiempo de fallos consecutivos de reproducción para el Circuit Breaker anti-bucle
    pub consecutive_errors: Arc<DashMap<PlayerKey, (u32, Instant)>>,
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

        let loaded_settings = Self::load_guild_settings_from_disk();
        let guild_volumes = Arc::new(DashMap::new());
        let loop_modes = Arc::new(DashMap::new());
        let twenty_four_seven_players = Arc::new(DashSet::new());
        let autoplay_guilds = Arc::new(DashSet::new());
        let dsp_params = Arc::new(DashMap::new());

        for (k_str, conf) in loaded_settings {
            let parts: Vec<&str> = k_str.split(':').collect();
            if parts.len() == 2 {
                if let (Ok(g_id), Ok(b_id)) = (parts[0].parse::<u64>(), parts[1].parse::<u64>()) {
                    let key = PlayerKey::new(g_id, b_id);
                    if let Some(vol) = conf.volume {
                        guild_volumes.insert(key, vol);
                    }
                    if let Some(lm) = conf.loop_mode {
                        loop_modes.insert(key, lm);
                    }
                    if conf.twenty_four_seven == Some(true) {
                        twenty_four_seven_players.insert(key);
                    }
                    if conf.autoplay == Some(true) {
                        autoplay_guilds.insert(key);
                    }
                    if let Some(dsp) = conf.dsp_params {
                        dsp_params.insert(key, Arc::new(RwLock::new(dsp)));
                    }
                }
            }
        }

        Self {
            players: Arc::new(DashMap::new()),
            dsp_params,
            http_client: Client::builder()
                .user_agent(default_ua)
                .build()
                .unwrap_or_else(|_| Client::new()),
            resolver: Arc::new(SourceResolver::new()),
            start_time: Instant::now(),
            load_epochs: Arc::new(DashMap::new()),
            pending_playlists: Arc::new(DashMap::new()),
            guild_volumes,
            loop_modes,
            autoplay_guilds,
            twenty_four_seven_players,
            seen_tracks: Arc::new(DashMap::new()),
            last_tracks: Arc::new(DashMap::new()),
            last_activity: Arc::new(DashMap::new()),
            prefetching_guilds: Arc::new(DashSet::new()),
            guild_play_options: Arc::new(DashMap::new()),
            event_tx,
            auth_token,
            rate_limiter,
            custom_presets: Arc::new(Self::load_presets_from_disk()),
            tracks_played_count: Arc::new(AtomicU64::new(0)),
            resolves_count: Arc::new(AtomicU64::new(0)),
            consecutive_errors: Arc::new(DashMap::new()),
        }
    }

    /// Resuelve la clave de reproductor a partir del guild_id y un posible bot_id.
    /// Si bot_id está presente y no es 0, se usa directamente.
    /// Si no se especifica y existe exactamente un reproductor activo para ese guild, se resuelve a ese reproductor.
    /// En caso contrario, resuelve a `(guild_id, 0)` para mantener retrocompatibilidad.
    pub fn resolve_player_key(&self, guild_id: u64, bot_id: Option<u64>) -> PlayerKey {
        if let Some(bid) = bot_id {
            if bid != 0 {
                return PlayerKey::new(guild_id, bid);
            }
        }

        let matching_bot_ids: Vec<u64> = self.players
            .iter()
            .filter(|e| e.key().guild_id == guild_id)
            .map(|e| e.key().bot_id)
            .collect();

        if matching_bot_ids.len() == 1 {
            PlayerKey::new(guild_id, matching_bot_ids[0])
        } else {
            PlayerKey::new(guild_id, 0)
        }
    }

    pub fn is_247(&self, key: PlayerKey) -> bool {
        self.twenty_four_seven_players.contains(&key)
    }

    pub fn set_247(&self, key: PlayerKey, enabled: bool) {
        if enabled {
            self.twenty_four_seven_players.insert(key);
        } else {
            self.twenty_four_seven_players.remove(&key);
        }
        self.persist_guild_settings();
    }

    pub fn get_volume(&self, key: PlayerKey) -> f32 {
        self.guild_volumes
            .get(&key)
            .map(|v| *v)
            .unwrap_or(1.0)
    }

    pub fn set_volume(&self, key: PlayerKey, vol: f32) {
        self.guild_volumes.insert(key, vol);
        self.persist_guild_settings();
    }

    pub fn get_loop_mode(&self, key: PlayerKey) -> LoopMode {
        self.loop_modes
            .get(&key)
            .map(|m| *m)
            .unwrap_or(LoopMode::Off)
    }

    pub fn set_loop_mode(&self, key: PlayerKey, mode: LoopMode) {
        self.loop_modes.insert(key, mode);
        self.persist_guild_settings();
    }

    pub fn is_autoplay(&self, key: PlayerKey) -> bool {
        self.autoplay_guilds.contains(&key)
    }

    pub fn set_autoplay(&self, key: PlayerKey, enabled: bool) {
        if enabled {
            self.autoplay_guilds.insert(key);
        } else {
            self.autoplay_guilds.remove(&key);
        }
        self.persist_guild_settings();
    }

    pub fn is_track_seen(&self, key: PlayerKey, video_id: &str) -> bool {
        if let Some(list) = self.seen_tracks.get(&key) {
            list.contains(&video_id.to_string())
        } else {
            false
        }
    }

    /// Registra un fallo de reproducción para el reproductor.
    /// Retorna `true` si se alcanzó el umbral de disparo del Circuit Breaker (3 fallos en menos de 20s).
    pub fn record_track_error(&self, key: PlayerKey) -> bool {
        let mut entry = self.consecutive_errors.entry(key).or_insert((0, Instant::now()));
        let (ref mut count, ref mut last_time) = *entry;
        if last_time.elapsed() < Duration::from_secs(20) {
            *count += 1;
        } else {
            *count = 1;
        }
        *last_time = Instant::now();
        *count >= 3
    }

    /// Retorna `true` si el reproductor está temporalmente bloqueado por dispararse el Circuit Breaker (3 fallos en menos de 20s).
    /// El bloqueo se mantiene durante un periodo de enfriamiento de 30s para evitar bucles infinitos.
    pub fn is_circuit_tripped(&self, key: PlayerKey) -> bool {
        if let Some(entry) = self.consecutive_errors.get(&key) {
            let (count, last_time) = *entry;
            count >= 3 && last_time.elapsed() < Duration::from_secs(30)
        } else {
            false
        }
    }

    /// Limpia el contador de errores consecutivos tras una reproducción exitosa.
    pub fn reset_consecutive_errors(&self, key: PlayerKey) {
        self.consecutive_errors.remove(&key);
    }

    pub fn record_seen_track(&self, key: PlayerKey, video_id: &str) {
        let mut entry = self.seen_tracks.entry(key).or_insert_with(Vec::new);
        if !entry.contains(&video_id.to_string()) {
            entry.push(video_id.to_string());
            if entry.len() > 60 {
                let overflow = entry.len() - 60;
                entry.drain(0..overflow);
            }
        }
    }

    pub fn clear_seen_tracks(&self, key: PlayerKey) {
        self.seen_tracks.remove(&key);
    }

    pub fn set_last_track(&self, key: PlayerKey, track: TrackMetadata) {
        self.last_tracks.insert(key, track);
    }

    pub fn get_last_track(&self, key: PlayerKey) -> Option<TrackMetadata> {
        self.last_tracks.get(&key).map(|t| t.clone())
    }

    pub fn is_prefetching(&self, key: PlayerKey) -> bool {
        self.prefetching_guilds.contains(&key)
    }

    pub fn start_prefetch(&self, key: PlayerKey) -> bool {
        self.prefetching_guilds.insert(key)
    }

    pub fn end_prefetch(&self, key: PlayerKey) {
        self.prefetching_guilds.remove(&key);
    }

    pub fn get_dsp_params(&self, key: PlayerKey) -> Arc<RwLock<DspParams>> {
        let entry = self
            .dsp_params
            .entry(key)
            .or_insert_with(|| Arc::new(RwLock::new(DspParams::default())));
        entry.value().clone()
    }

    pub fn next_epoch(&self, key: PlayerKey) -> u64 {
        let entry = self
            .load_epochs
            .entry(key)
            .or_insert_with(|| Arc::new(AtomicU64::new(0)));
        entry.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }

    #[allow(dead_code)]
    pub fn current_epoch(&self, key: PlayerKey) -> u64 {
        self.load_epochs
            .get(&key)
            .map(|e| e.load(std::sync::atomic::Ordering::SeqCst))
            .unwrap_or(0)
    }

    pub fn touch_activity(&self, key: PlayerKey) {
        self.last_activity.insert(key, Instant::now());
    }

    /// Retorna la última actividad del reproductor.
    /// Si no hay entrada, retorna `Instant::now() - Duration::MAX` (prácticamente el inicio del runtime),
    /// lo que hace que el GC detecte inmediatamente el reproductor como inactivo.
    pub fn get_last_activity(&self, key: PlayerKey) -> Instant {
        self.last_activity
            .get(&key)
            .map(|t| *t.value())
            .unwrap_or_else(|| Instant::now() - std::time::Duration::MAX)
    }

    /// Ruta al archivo de almacenamiento persistente de presets personalizados
    pub fn get_presets_file_path() -> std::path::PathBuf {
        std::env::var("AXAUDIO_PRESETS_FILE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("presets/custom_presets.json"))
    }

    /// Carga presets personalizados guardados previamente en disco
    pub fn load_presets_from_disk() -> DashMap<String, DspParams> {
        let path = Self::get_presets_file_path();
        if path.exists() {
            if let Ok(file_content) = std::fs::read_to_string(&path) {
                if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, DspParams>>(&file_content) {
                    let dash = DashMap::new();
                    for (k, v) in map {
                        dash.insert(k, v);
                    }
                    tracing::info!(count = dash.len(), path = ?path, "Presets personalizados cargados desde disco");
                    return dash;
                }
            }
        }
        DashMap::new()
    }

    /// Escribe los presets personalizados actuales a disco en formato JSON
    pub fn persist_presets_to_disk(&self) {
        let path = Self::get_presets_file_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let map: std::collections::HashMap<String, DspParams> = self
            .custom_presets
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();
        if let Ok(json) = serde_json::to_string_pretty(&map) {
            let _ = std::fs::write(&path, json);
        }
    }

    /// Guarda un preset personalizado global y lo sincroniza a disco
    pub fn save_custom_preset(&self, name: String, params: DspParams) {
        self.custom_presets.insert(name, params);
        self.persist_presets_to_disk();
    }

    /// Carga un preset personalizado por nombre
    pub fn get_custom_preset(&self, name: &str) -> Option<DspParams> {
        self.custom_presets.get(name).map(|e| e.value().clone())
    }

    /// Elimina un preset personalizado y sincroniza a disco
    pub fn delete_custom_preset(&self, name: &str) -> bool {
        let removed = self.custom_presets.remove(name).is_some();
        if removed {
            self.persist_presets_to_disk();
        }
        removed
    }

    /// Lista todos los presets personalizados
    pub fn list_custom_presets(&self) -> Vec<String> {
        self.custom_presets.iter().map(|e| e.key().clone()).collect()
    }

    /// Ruta al archivo de almacenamiento persistente de configuraciones por guild/bot
    pub fn get_guild_settings_file_path() -> std::path::PathBuf {
        std::env::var("AXAUDIO_GUILD_SETTINGS_FILE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("settings/guild_settings.json"))
    }

    /// Carga configuraciones de guilds guardadas previamente en disco
    pub fn load_guild_settings_from_disk() -> HashMap<String, PersistedPlayerConfig> {
        let path = Self::get_guild_settings_file_path();
        if path.exists() {
            if let Ok(file_content) = std::fs::read_to_string(&path) {
                if let Ok(map) = serde_json::from_str::<HashMap<String, PersistedPlayerConfig>>(&file_content) {
                    tracing::info!(count = map.len(), path = ?path, "Configuraciones de guilds cargadas desde disco");
                    return map;
                }
            }
        }
        HashMap::new()
    }

    /// Escribe las configuraciones activas de los reproductores a disco
    pub fn persist_guild_settings(&self) {
        let path = Self::get_guild_settings_file_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut map: HashMap<String, PersistedPlayerConfig> = HashMap::new();
        let mut all_keys = std::collections::HashSet::new();
        for e in self.guild_volumes.iter() {
            all_keys.insert(*e.key());
        }
        for e in self.loop_modes.iter() {
            all_keys.insert(*e.key());
        }
        for e in self.twenty_four_seven_players.iter() {
            all_keys.insert(*e);
        }
        for e in self.autoplay_guilds.iter() {
            all_keys.insert(*e);
        }
        for e in self.dsp_params.iter() {
            all_keys.insert(*e.key());
        }

        for key in all_keys {
            let volume = self.guild_volumes.get(&key).map(|v| *v);
            let loop_mode = self.loop_modes.get(&key).map(|m| *m);
            let twenty_four_seven = if self.twenty_four_seven_players.contains(&key) {
                Some(true)
            } else {
                None
            };
            let autoplay = if self.autoplay_guilds.contains(&key) {
                Some(true)
            } else {
                None
            };
            let dsp = self
                .dsp_params
                .get(&key)
                .and_then(|p| p.read().ok().map(|g| g.clone()));

            let config = PersistedPlayerConfig {
                volume,
                loop_mode,
                twenty_four_seven,
                autoplay,
                dsp_params: dsp,
            };
            map.insert(format!("{}:{}", key.guild_id, key.bot_id), config);
        }
        if let Ok(json) = serde_json::to_string_pretty(&map) {
            let _ = std::fs::write(&path, json);
        }
        self.resolver.persist_cache_to_disk(&crate::sources::SourceResolver::get_cache_file_path());
    }

    /// Retorna la memoria residente (RSS) en bytes del proceso axAudio
    pub fn get_memory_rss() -> u64 {
        #[cfg(target_os = "linux")]
        {
            if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
                let mut parts = statm.split_whitespace();
                let _total = parts.next();
                if let Some(rss_pages) = parts.next().and_then(|p| p.parse::<u64>().ok()) {
                    let page_size = 4096;
                    return rss_pages * page_size;
                }
            }
        }
        0
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_player_key_explicit_bot() {
        let state = AppState::new();
        let key = state.resolve_player_key(100, Some(555));
        assert_eq!(key, PlayerKey::new(100, 555));
    }

    #[tokio::test]
    async fn test_resolve_player_key_single_active() {
        let state = AppState::new();
        let expected = PlayerKey::new(100, 777);
        state.players.insert(expected, Arc::new(tokio::sync::Mutex::new(songbird::driver::Driver::new(Default::default()))));

        let resolved = state.resolve_player_key(100, None);
        assert_eq!(resolved, expected);
    }

    #[tokio::test]
    async fn test_multi_bot_isolation_same_guild() {
        let state = AppState::new();
        let key_bot_1 = PlayerKey::new(100, 111);
        let key_bot_2 = PlayerKey::new(100, 222);

        state.players.insert(key_bot_1, Arc::new(tokio::sync::Mutex::new(songbird::driver::Driver::new(Default::default()))));
        state.players.insert(key_bot_2, Arc::new(tokio::sync::Mutex::new(songbird::driver::Driver::new(Default::default()))));

        state.set_volume(key_bot_1, 0.5);
        state.set_volume(key_bot_2, 1.5);
        state.set_247(key_bot_1, true);

        assert_eq!(state.get_volume(key_bot_1), 0.5);
        assert_eq!(state.get_volume(key_bot_2), 1.5);
        assert!(state.is_247(key_bot_1));
        assert!(!state.is_247(key_bot_2));

        assert_eq!(state.resolve_player_key(100, Some(111)), key_bot_1);
        assert_eq!(state.resolve_player_key(100, Some(222)), key_bot_2);
    }

    #[test]
    fn test_resolve_player_key_fallback_zero() {
        let state = AppState::new();
        let resolved = state.resolve_player_key(100, None);
        assert_eq!(resolved, PlayerKey::new(100, 0));
    }

    #[test]
    fn test_twenty_four_seven_toggle() {
        let state = AppState::new();
        let key = PlayerKey::new(100, 200);

        assert!(!state.is_247(key));
        state.set_247(key, true);
        assert!(state.is_247(key));
        state.set_247(key, false);
        assert!(!state.is_247(key));
    }

    #[test]
    fn test_custom_presets_crud() {
        let state = AppState::new();
        let preset_name = "test_nightcore".to_string();
        let mut params = DspParams::default();
        params.timescale.speed = 1.3;
        params.timescale.pitch = 1.3;

        state.save_custom_preset(preset_name.clone(), params.clone());

        let retrieved = state.get_custom_preset(&preset_name);
        assert!(retrieved.is_some());
        let p = retrieved.unwrap();
        assert_eq!(p.timescale.speed, 1.3);

        let list = state.list_custom_presets();
        assert!(list.contains(&preset_name));

        assert!(state.delete_custom_preset(&preset_name));
        assert!(state.get_custom_preset(&preset_name).is_none());
        assert!(!state.delete_custom_preset(&preset_name));
    }

    #[test]
    fn test_memory_rss_reported() {
        let rss = AppState::get_memory_rss();
        #[cfg(target_os = "linux")]
        assert!(rss > 0, "RSS on linux should be non-zero");
        #[cfg(not(target_os = "linux"))]
        let _ = rss;
    }

    #[test]
    fn test_guild_settings_persistence() {
        let temp_path = std::env::temp_dir().join(format!("axaudio_test_settings_{}.json", std::process::id()));
        unsafe {
            std::env::set_var("AXAUDIO_GUILD_SETTINGS_FILE", &temp_path);
        }

        let state = AppState::new();
        let key = PlayerKey::new(999, 888);
        state.set_volume(key, 0.42);
        state.set_loop_mode(key, LoopMode::Queue);
        state.set_247(key, true);
        state.set_autoplay(key, true);

        let dsp_params = state.get_dsp_params(key);
        if let Ok(mut p) = dsp_params.write() {
            p.timescale.speed = 1.25;
        }
        state.persist_guild_settings();

        assert!(temp_path.exists());
        let content = std::fs::read_to_string(&temp_path).unwrap();
        assert!(content.contains("0.42"));
        assert!(content.contains("\"twenty_four_seven\": true"));

        let loaded = AppState::load_guild_settings_from_disk();
        let cfg = loaded.get("999:888").expect("Should have persisted 999:888");
        assert_eq!(cfg.volume, Some(0.42));
        assert_eq!(cfg.loop_mode, Some(LoopMode::Queue));
        assert_eq!(cfg.twenty_four_seven, Some(true));
        assert_eq!(cfg.autoplay, Some(true));
        assert_eq!(cfg.dsp_params.as_ref().map(|d| d.timescale.speed), Some(1.25));

        let _ = std::fs::remove_file(temp_path);
    }

    #[test]
    fn test_circuit_breaker() {
        let state = AppState::new();
        let key = PlayerKey::new(1234, 5678);

        // Sin errores previos, el circuit breaker no debe estar activo
        assert!(!state.is_circuit_tripped(key));

        // 1er error: no salta
        assert!(!state.record_track_error(key));
        assert!(!state.is_circuit_tripped(key));

        // 2do error: no salta
        assert!(!state.record_track_error(key));
        assert!(!state.is_circuit_tripped(key));

        // 3er error consecutivo: salta el circuit breaker
        assert!(state.record_track_error(key));
        assert!(state.is_circuit_tripped(key));

        // Reset: se limpia y vuelve a estar libre
        state.reset_consecutive_errors(key);
        assert!(!state.is_circuit_tripped(key));
    }
}
