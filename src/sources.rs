use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::Semaphore;
use tracing::{info, warn};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackMetadata {
    pub title: String,
    pub source_url: String,
    pub stream_url: String,
    pub duration_secs: Option<u64>,
    pub author: Option<String>,
    pub thumbnail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlatPlaylistItem {
    pub title: String,
    pub url: String,
    pub duration_secs: Option<u64>,
    pub author: Option<String>,
    pub thumbnail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlatPlaylist {
    pub title: String,
    pub entries: Vec<FlatPlaylistItem>,
}

#[derive(Clone, Debug)]
pub struct ResolvedPlaylistBatch {
    pub first_track: TrackMetadata,
    pub is_playlist: bool,
    pub playlist_title: Option<String>,
    pub total_tracks_count: usize,
    pub remaining_entries: Vec<FlatPlaylistItem>,
}

#[derive(Clone, Debug)]
pub struct ResolvedBatch {
    pub is_playlist: bool,
    pub playlist_title: Option<String>,
    pub tracks: Vec<TrackMetadata>,
}

#[derive(Clone)]
pub struct SourceResolver {
    cache: Arc<DashMap<String, (TrackMetadata, Instant)>>,
    ttl: Duration,
    cookie_rotation_index: Arc<AtomicUsize>,
    process_semaphore: Arc<Semaphore>,
}

impl SourceResolver {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(DashMap::new()),
            ttl: Duration::from_secs(3600 * 4), // 4 horas de TTL para streams de audio
            cookie_rotation_index: Arc::new(AtomicUsize::new(0)),
            process_semaphore: Arc::new(Semaphore::new(1)),
        }
    }

    /// Determina si una URL ya apunta a un flujo HTTP/S directo de audio sin requerir extractor
    pub fn is_direct_stream(url: &str) -> bool {
        let lower = url.to_lowercase();
        lower.ends_with(".mp3")
            || lower.ends_with(".ogg")
            || lower.ends_with(".flac")
            || lower.ends_with(".aac")
            || lower.ends_with(".wav")
            || lower.ends_with(".m4a")
            || lower.ends_with(".opus")
            || lower.contains("stream.zeno.fm")
            || lower.contains("icecast")
            || lower.contains("shoutcast")
    }

    /// Detecta si una consulta o URL corresponde a una lista de reproducción, Mix o Radio
    pub fn is_playlist_url(query: &str) -> bool {
        let q = query.trim();
        q.contains("playlist?list=") || q.contains("&list=") || q.contains("?list=")
    }

    /// Extrae la URL del primer video si está presente en el parámetro `v=` o formato youtu.be
    pub fn extract_video_url(url: &str) -> Option<String> {
        if let Some(pos) = url.find("v=") {
            let rest = &url[pos + 2..];
            let id: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if !id.is_empty() {
                return Some(format!("https://www.youtube.com/watch?v={}", id));
            }
        }
        if url.contains("youtu.be/") {
            if let Some(pos) = url.find("youtu.be/") {
                let rest = &url[pos + 9..];
                let id: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                    .collect();
                if !id.is_empty() {
                    return Some(format!("https://www.youtube.com/watch?v={}", id));
                }
            }
        }
        None
    }

    /// Escanea y retorna la lista de archivos de cookies disponibles (Pool multi-cuenta)
    pub async fn get_available_cookie_pool(&self) -> Vec<String> {
        let mut pool = Vec::new();

        // 1. Directorios de cookies (YTDLP_COOKIES_DIR, /app/cookies o ./cookies)
        let dir_candidates = [
            std::env::var("YTDLP_COOKIES_DIR").ok(),
            Some("/app/cookies".to_string()),
            Some("cookies".to_string()),
        ];

        for dir in dir_candidates.into_iter().flatten() {
            let path = PathBuf::from(&dir);
            if path.is_dir() {
                if let Ok(mut entries) = tokio::fs::read_dir(&path).await {
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        let entry_path = entry.path();
                        if entry_path.is_file() {
                            if let Some(ext) = entry_path.extension() {
                                if ext == "txt" {
                                    if let Some(s) = entry_path.to_str() {
                                        pool.push(s.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Lista separada por comas (YTDLP_COOKIES_POOL)
        if let Ok(env_pool) = std::env::var("YTDLP_COOKIES_POOL") {
            for item in env_pool.split(',') {
                let trimmed = item.trim();
                if !trimmed.is_empty() && PathBuf::from(trimmed).is_file() {
                    pool.push(trimmed.to_string());
                }
            }
        }

        // 3. Rutas individuales de fallback
        let single_candidates = [
            std::env::var("YTDLP_COOKIES_PATH").ok(),
            std::env::var("YOUTUBE_COOKIES_PATH").ok(),
            Some("/app/cookies.txt".to_string()),
            Some("cookies.txt".to_string()),
        ];

        for candidate in single_candidates.into_iter().flatten() {
            if PathBuf::from(&candidate).is_file() {
                pool.push(candidate);
            }
        }

        pool.sort();
        pool.dedup();
        pool
    }

    /// Ejecuta extracción de lista plana rápida mediante `--flat-playlist` (~1-4 segundos para 25 canciones)
    async fn run_flat_playlist_cmd(
        &self,
        query: &str,
        custom_extractor_args: Option<&str>,
        custom_extra_args: Option<&str>,
        cookie_path: Option<&str>,
    ) -> Result<FlatPlaylist, String> {
        let query_trimmed = query.trim();
        let _permit = self.process_semaphore.acquire().await.map_err(|e| format!("Fallo adquiriendo semáforo de proceso para flat-playlist: {}", e))?;
        let mut cmd = Command::new("yt-dlp");
        cmd.arg("--flat-playlist");
        cmd.arg("--dump-single-json");

        let playlist_max = std::env::var("YTDLP_PLAYLIST_MAX").unwrap_or_else(|_| "25".to_string());
        cmd.arg("--playlist-end").arg(playlist_max);

        let default_search = std::env::var("YTDLP_DEFAULT_SEARCH").unwrap_or_else(|_| "ytsearch:".to_string());
        cmd.arg("--default-search").arg(default_search);

        let remote_components = std::env::var("YTDLP_REMOTE_COMPONENTS").unwrap_or_else(|_| "ejs:github".to_string());
        if !remote_components.is_empty() && remote_components != "none" {
            cmd.arg("--remote-components").arg(remote_components);
        }

        if let Some(args) = custom_extractor_args {
            cmd.arg("--extractor-args").arg(args);
        } else if let Ok(env_args) = std::env::var("YTDLP_EXTRACTOR_ARGS") {
            cmd.arg("--extractor-args").arg(env_args);
        } else {
            let pot_provider_url = std::env::var("BGUTIL_POT_URL")
                .unwrap_or_else(|_| "http://bgutil-pot:4416".to_string());
            cmd.arg("--extractor-args")
                .arg(format!("youtubepot-bgutilhttp:base_url={}", pot_provider_url));
        }

        if let Some(path) = cookie_path {
            cmd.arg("--cookies").arg(path);
        }

        if let Ok(env_extra) = std::env::var("YTDLP_EXTRA_ARGS") {
            for arg in env_extra.split_whitespace() {
                if !arg.is_empty() {
                    cmd.arg(arg);
                }
            }
        }

        if let Some(extra) = custom_extra_args {
            for arg in extra.split_whitespace() {
                if !arg.is_empty() {
                    cmd.arg(arg);
                }
            }
        }

        cmd.arg(query_trimmed);

        let output = cmd.output().await.map_err(|e| {
            format!("Error ejecutando subproceso yt-dlp flat playlist: {:?}", e)
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("yt-dlp flat-playlist falló: {}", stderr.trim()));
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        let val: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| format!("Error parseando JSON de flat-playlist: {:?}", e))?;

        let title = val
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("Playlist de YouTube")
            .to_string();

        let mut entries = Vec::new();
        if let Some(items) = val.get("entries").and_then(|e| e.as_array()) {
            for item in items {
                let item_title = item
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("Pista Desconocida")
                    .to_string();

                let item_url = if let Some(u) = item.get("url").and_then(|u| u.as_str()) {
                    if u.starts_with("http") {
                        u.to_string()
                    } else {
                        format!("https://www.youtube.com/watch?v={}", u)
                    }
                } else if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    format!("https://www.youtube.com/watch?v={}", id)
                } else {
                    continue;
                };

                let duration_secs = item.get("duration").and_then(|d| d.as_u64());
                let author = item
                    .get("uploader")
                    .or_else(|| item.get("channel"))
                    .and_then(|u| u.as_str())
                    .map(|s| s.to_string());

                let thumbnail = item
                    .get("thumbnail")
                    .and_then(|t| t.as_str())
                    .or_else(|| {
                        item.get("thumbnails")
                            .and_then(|arr| arr.as_array())
                            .and_then(|arr| arr.last())
                            .and_then(|th| th.get("url"))
                            .and_then(|u| u.as_str())
                    })
                    .map(|s| s.to_string());

                entries.push(FlatPlaylistItem {
                    title: item_title,
                    url: item_url,
                    duration_secs,
                    author,
                    thumbnail,
                });
            }
        }

        Ok(FlatPlaylist { title, entries })
    }

    /// Resuelve el flujo de audio CDN de una única canción en ~3.5 segundos con formato 251 Opus nativo
    async fn run_single_track_cmd(
        &self,
        query: &str,
        custom_extractor_args: Option<&str>,
        custom_extra_args: Option<&str>,
        cookie_path: Option<&str>,
    ) -> Result<TrackMetadata, String> {
        let query_trimmed = query.trim();
        let _permit = self.process_semaphore.acquire().await.map_err(|e| format!("Fallo adquiriendo semáforo de proceso para pista individual: {}", e))?;
        let mut cmd = Command::new("yt-dlp");
        cmd.arg("--no-playlist");
        cmd.arg("--dump-single-json");

        let default_format = "ba[format_id=251]/ba[acodec=opus]/ba[ext=webm]/bestaudio[ext=m4a]/bestaudio/best";
        let format = std::env::var("YTDLP_FORMAT").unwrap_or_else(|_| default_format.to_string());
        cmd.arg("--format").arg(format);

        let default_search = std::env::var("YTDLP_DEFAULT_SEARCH").unwrap_or_else(|_| "ytsearch:".to_string());
        cmd.arg("--default-search").arg(default_search);

        let remote_components = std::env::var("YTDLP_REMOTE_COMPONENTS").unwrap_or_else(|_| "ejs:github".to_string());
        if !remote_components.is_empty() && remote_components != "none" {
            cmd.arg("--remote-components").arg(remote_components);
        }

        if let Some(args) = custom_extractor_args {
            cmd.arg("--extractor-args").arg(args);
        } else if let Ok(env_args) = std::env::var("YTDLP_EXTRACTOR_ARGS") {
            cmd.arg("--extractor-args").arg(env_args);
        } else {
            let pot_provider_url = std::env::var("BGUTIL_POT_URL")
                .unwrap_or_else(|_| "http://bgutil-pot:4416".to_string());
            cmd.arg("--extractor-args")
                .arg(format!("youtubepot-bgutilhttp:base_url={}", pot_provider_url));
        }

        if let Some(path) = cookie_path {
            cmd.arg("--cookies").arg(path);
        }

        if let Ok(env_extra) = std::env::var("YTDLP_EXTRA_ARGS") {
            for arg in env_extra.split_whitespace() {
                if !arg.is_empty() {
                    cmd.arg(arg);
                }
            }
        }

        if let Some(extra) = custom_extra_args {
            for arg in extra.split_whitespace() {
                if !arg.is_empty() {
                    cmd.arg(arg);
                }
            }
        }

        cmd.arg(query_trimmed);

        let output = cmd.output().await.map_err(|e| {
            format!("Error al ejecutar subproceso yt-dlp: {:?}", e)
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(query = %query_trimmed, stderr = %stderr, "yt-dlp falló al resolver la fuente");
            return Err(format!("yt-dlp failed: {}", stderr.trim()));
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        let val: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| format!("Error parseando salida JSON de yt-dlp: {:?}", e))?;

        let stream_url = val
            .get("url")
            .and_then(|u| u.as_str())
            .or_else(|| {
                val.get("entries")
                    .and_then(|arr| arr.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|item| item.get("url"))
                    .and_then(|u| u.as_str())
            })
            .ok_or_else(|| "No se encontró stream_url de audio para la pista".to_string())?;

        let title = val
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("Pista Desconocida")
            .to_string();

        let source_url = val
            .get("webpage_url")
            .and_then(|u| u.as_str())
            .unwrap_or(query_trimmed)
            .to_string();

        let duration_secs = val.get("duration").and_then(|d| d.as_u64());
        let author = val
            .get("uploader")
            .or_else(|| val.get("channel"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string());

        let thumbnail = val
            .get("thumbnail")
            .and_then(|t| t.as_str())
            .or_else(|| {
                val.get("thumbnails")
                    .and_then(|arr| arr.as_array())
                    .and_then(|arr| arr.last())
                    .and_then(|th| th.get("url"))
                    .and_then(|u| u.as_str())
            })
            .map(|s| s.to_string());

        Ok(TrackMetadata {
            title,
            source_url,
            stream_url: stream_url.to_string(),
            duration_secs,
            author,
            thumbnail,
        })
    }

    /// Resuelve una pista individual con soporte de caché y rotación de cookies
    pub async fn resolve_single(
        &self,
        query: &str,
        custom_extractor_args: Option<&str>,
        custom_extra_args: Option<&str>,
        custom_cookies: Option<&str>,
    ) -> Result<TrackMetadata, String> {
        let query_trimmed = query.trim();

        // 1. Revisar caché en memoria
        if custom_extractor_args.is_none() && custom_extra_args.is_none() && custom_cookies.is_none() {
            if let Some(entry) = self.cache.get(query_trimmed) {
                let (meta, cached_at) = entry.value();
                if cached_at.elapsed() < self.ttl {
                    info!(query = %query_trimmed, title = %meta.title, "Source resolver cache hit");
                    return Ok(meta.clone());
                }
            }
        }

        // 2. Stream directo
        if Self::is_direct_stream(query_trimmed) {
            return Ok(TrackMetadata {
                title: "Direct Stream".to_string(),
                source_url: query_trimmed.to_string(),
                stream_url: query_trimmed.to_string(),
                duration_secs: None,
                author: None,
                thumbnail: None,
            });
        }

        // 3. Cookie personalizada
        if let Some(c) = custom_cookies {
            let meta = self
                .run_single_track_cmd(query_trimmed, custom_extractor_args, custom_extra_args, Some(c))
                .await?;
            self.cache.insert(query_trimmed.to_string(), (meta.clone(), Instant::now()));
            return Ok(meta);
        }

        // 4. Pool de cookies
        let cookie_pool = self.get_available_cookie_pool().await;
        if cookie_pool.is_empty() {
            let meta = self
                .run_single_track_cmd(query_trimmed, custom_extractor_args, custom_extra_args, None)
                .await?;
            self.cache.insert(query_trimmed.to_string(), (meta.clone(), Instant::now()));
            return Ok(meta);
        }

        let total = cookie_pool.len();
        let start_idx = self.cookie_rotation_index.fetch_add(1, Ordering::Relaxed) % total;
        let mut last_err = String::new();

        for i in 0..total {
            let idx = (start_idx + i) % total;
            let cookie_file = &cookie_pool[idx];
            match self
                .run_single_track_cmd(query_trimmed, custom_extractor_args, custom_extra_args, Some(cookie_file))
                .await
            {
                Ok(meta) => {
                    self.cache.insert(query_trimmed.to_string(), (meta.clone(), Instant::now()));
                    return Ok(meta);
                }
                Err(e) => {
                    warn!(cookie_file = %cookie_file, error = %e, "Cookie falló para pista, reintentando...");
                    last_err = e;
                }
            }
        }

        Err(format!("Todas las cookies del pool fallaron resolviendo la pista. Último error: {}", last_err))
    }

    /// Resuelve la lista plana de una playlist / mix con rotación de cookies
    pub async fn resolve_flat_playlist(
        &self,
        query: &str,
        custom_extractor_args: Option<&str>,
        custom_extra_args: Option<&str>,
        custom_cookies: Option<&str>,
    ) -> Result<FlatPlaylist, String> {
        let query_trimmed = query.trim();

        if let Some(c) = custom_cookies {
            return self
                .run_flat_playlist_cmd(query_trimmed, custom_extractor_args, custom_extra_args, Some(c))
                .await;
        }

        let cookie_pool = self.get_available_cookie_pool().await;
        if cookie_pool.is_empty() {
            return self
                .run_flat_playlist_cmd(query_trimmed, custom_extractor_args, custom_extra_args, None)
                .await;
        }

        let total = cookie_pool.len();
        let start_idx = self.cookie_rotation_index.fetch_add(1, Ordering::Relaxed) % total;
        let mut last_err = String::new();

        for i in 0..total {
            let idx = (start_idx + i) % total;
            let cookie_file = &cookie_pool[idx];
            match self
                .run_flat_playlist_cmd(query_trimmed, custom_extractor_args, custom_extra_args, Some(cookie_file))
                .await
            {
                Ok(playlist) => return Ok(playlist),
                Err(e) => {
                    warn!(cookie_file = %cookie_file, error = %e, "Cookie falló para flat-playlist, reintentando...");
                    last_err = e;
                }
            }
        }

        Err(format!("Todas las cookies fallaron para flat-playlist. Último error: {}", last_err))
    }

    /// Resuelve de forma ultrarrápida (Fast Start): pista 1 inmediata + lista de pendientes
    pub async fn resolve_playlist_batch(
        &self,
        query: &str,
        custom_extractor_args: Option<&str>,
        custom_extra_args: Option<&str>,
        custom_cookies: Option<&str>,
    ) -> Result<ResolvedPlaylistBatch, String> {
        let query_trimmed = query.trim();

        // 1. Stream directo
        if Self::is_direct_stream(query_trimmed) {
            let meta = TrackMetadata {
                title: "Direct Stream".to_string(),
                source_url: query_trimmed.to_string(),
                stream_url: query_trimmed.to_string(),
                duration_secs: None,
                author: None,
                thumbnail: None,
            };
            return Ok(ResolvedPlaylistBatch {
                first_track: meta,
                is_playlist: false,
                playlist_title: None,
                total_tracks_count: 1,
                remaining_entries: vec![],
            });
        }

        let is_playlist = Self::is_playlist_url(query_trimmed);

        if !is_playlist {
            let first_track = self
                .resolve_single(query_trimmed, custom_extractor_args, custom_extra_args, custom_cookies)
                .await?;
            return Ok(ResolvedPlaylistBatch {
                first_track,
                is_playlist: false,
                playlist_title: None,
                total_tracks_count: 1,
                remaining_entries: vec![],
            });
        }

        // 2. Es un enlace de Mix o Playlist:
        // Si el enlace ya contiene un video ID específico (ej. watch?v=XYZ&list=RDXYZ),
        // resolvemos la primera canción Y la lista plana EN PARALELO mediante tokio::join!
        if let Some(first_video_url) = Self::extract_video_url(query_trimmed) {
            info!(v_url = %first_video_url, "Extrayendo primera pista y lista plana de forma secuencial y controlada...");
            let first_track = self.resolve_single(&first_video_url, custom_extractor_args, custom_extra_args, custom_cookies).await?;
            let flat_playlist = self.resolve_flat_playlist(query_trimmed, custom_extractor_args, custom_extra_args, custom_cookies).await?;

            let mut remaining = flat_playlist.entries;
            // Remover la primera canción de los pendientes para no duplicar
            remaining.retain(|e| e.url != first_track.source_url && !first_track.source_url.contains(&e.url));

            let total_count = remaining.len() + 1;
            info!(
                first = %first_track.title,
                playlist = %flat_playlist.title,
                total = total_count,
                "Extracción paralela completada en tiempo récord"
            );

            return Ok(ResolvedPlaylistBatch {
                first_track,
                is_playlist: true,
                playlist_title: Some(flat_playlist.title),
                total_tracks_count: total_count,
                remaining_entries: remaining,
            });
        }

        // 3. Playlist pura sin video específico (ej. playlist?list=PL...)
        info!(query = %query_trimmed, "Extrayendo flat-playlist de lista pura...");
        let mut flat_playlist = self
            .resolve_flat_playlist(query_trimmed, custom_extractor_args, custom_extra_args, custom_cookies)
            .await?;

        if flat_playlist.entries.is_empty() {
            return Err("La playlist no contiene pistas reproducibles".to_string());
        }

        let first_entry = flat_playlist.entries.remove(0);
        let first_track = self
            .resolve_single(&first_entry.url, custom_extractor_args, custom_extra_args, custom_cookies)
            .await?;

        let total_count = flat_playlist.entries.len() + 1;
        Ok(ResolvedPlaylistBatch {
            first_track,
            is_playlist: true,
            playlist_title: Some(flat_playlist.title),
            total_tracks_count: total_count,
            remaining_entries: flat_playlist.entries,
        })
    }

    /// Resuelve una única pista (compatible con /v1/tracks/resolve)
    pub async fn resolve(
        &self,
        query: &str,
        custom_extractor_args: Option<&str>,
        custom_extra_args: Option<&str>,
        custom_cookies: Option<&str>,
    ) -> Result<TrackMetadata, String> {
        self.resolve_single(query, custom_extractor_args, custom_extra_args, custom_cookies)
            .await
    }
}

impl Default for SourceResolver {
    fn default() -> Self {
        Self::new()
    }
}
