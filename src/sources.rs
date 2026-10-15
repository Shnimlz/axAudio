use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
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
}

impl SourceResolver {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(DashMap::new()),
            ttl: Duration::from_secs(3600 * 4), // 4 horas de TTL para streams de audio
            cookie_rotation_index: Arc::new(AtomicUsize::new(0)),
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

        // Deduplicar manteniendo orden
        pool.sort();
        pool.dedup();
        pool
    }

    /// Ejecuta subproceso de yt-dlp con soporte de playlists, mixes (RD...), radios y videos individuales
    async fn run_ytdlp(
        &self,
        query: &str,
        custom_extractor_args: Option<&str>,
        custom_extra_args: Option<&str>,
        cookie_path: Option<&str>,
    ) -> Result<ResolvedBatch, String> {
        let query_trimmed = query.trim();

        let mut cmd = Command::new("yt-dlp");
        cmd.arg("--dump-single-json");

        // Detección inteligente de Playlist, Mix de YouTube (list=RD...) o Radio
        let is_playlist_url = query_trimmed.contains("playlist?list=")
            || query_trimmed.contains("&list=")
            || query_trimmed.contains("?list=");

        if is_playlist_url {
            let playlist_max = std::env::var("YTDLP_PLAYLIST_MAX").unwrap_or_else(|_| "25".to_string());
            cmd.arg("--playlist-end").arg(playlist_max);
        } else {
            cmd.arg("--no-playlist");
        }

        // 1. Prioridad absoluta a Opus 48kHz (Formato 251 de YouTube) para audio nativo sin distorsión
        let default_format = "ba[format_id=251]/ba[acodec=opus]/ba[ext=webm]/bestaudio[ext=m4a]/bestaudio/best";
        let format = std::env::var("YTDLP_FORMAT").unwrap_or_else(|_| default_format.to_string());
        cmd.arg("--format").arg(format);

        // 2. Motor de búsqueda personalizable (default: ytsearch:)
        let default_search = std::env::var("YTDLP_DEFAULT_SEARCH").unwrap_or_else(|_| "ytsearch:".to_string());
        cmd.arg("--default-search").arg(default_search);

        // 3. EJS Challenge solver (GitHub releases)
        let remote_components = std::env::var("YTDLP_REMOTE_COMPONENTS").unwrap_or_else(|_| "ejs:github".to_string());
        if !remote_components.is_empty() && remote_components != "none" {
            cmd.arg("--remote-components").arg(remote_components);
        }

        // 4. Extractor Args personalizables (prioridad: payload > env YTDLP_EXTRACTOR_ARGS > bgutil-pot por defecto)
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

        // 5. Configurar cookies si existen
        if let Some(path) = cookie_path {
            info!(cookie_path = %path, "Utilizando cookie para autenticación yt-dlp");
            cmd.arg("--cookies").arg(path);
        }

        // 6. Argumentos adicionales libres (env YTDLP_EXTRA_ARGS y custom_extra_args)
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

        let is_playlist_type = val.get("_type").and_then(|t| t.as_str()) == Some("playlist");
        let playlist_title = val.get("title").and_then(|t| t.as_str()).map(|s| s.to_string());

        let mut tracks = Vec::new();

        if let Some(entries) = val.get("entries").and_then(|e| e.as_array()) {
            for item in entries {
                if let Some(stream_url) = item.get("url").and_then(|u| u.as_str()) {
                    let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("Unknown Track").to_string();
                    let source_url = item.get("webpage_url").and_then(|u| u.as_str()).unwrap_or(query_trimmed).to_string();
                    let duration_secs = item.get("duration").and_then(|d| d.as_u64());
                    let author = item.get("uploader").and_then(|u| u.as_str()).map(|s| s.to_string());
                    let thumbnail = item.get("thumbnail").and_then(|t| t.as_str()).map(|s| s.to_string());

                    tracks.push(TrackMetadata {
                        title,
                        source_url,
                        stream_url: stream_url.to_string(),
                        duration_secs,
                        author,
                        thumbnail,
                    });
                }
            }
        } else if let Some(stream_url) = val.get("url").and_then(|u| u.as_str()) {
            let title = val.get("title").and_then(|t| t.as_str()).unwrap_or("Unknown Track").to_string();
            let source_url = val.get("webpage_url").and_then(|u| u.as_str()).unwrap_or(query_trimmed).to_string();
            let duration_secs = val.get("duration").and_then(|d| d.as_u64());
            let author = val.get("uploader").and_then(|u| u.as_str()).map(|s| s.to_string());
            let thumbnail = val.get("thumbnail").and_then(|t| t.as_str()).map(|s| s.to_string());

            tracks.push(TrackMetadata {
                title,
                source_url,
                stream_url: stream_url.to_string(),
                duration_secs,
                author,
                thumbnail,
            });
        }

        if tracks.is_empty() {
            return Err("No se encontraron pistas reproducibles con URL directa de audio".to_string());
        }

        let is_playlist = is_playlist_type || (is_playlist_url && tracks.len() > 1);

        Ok(ResolvedBatch {
            is_playlist,
            playlist_title: if is_playlist { playlist_title } else { None },
            tracks,
        })
    }

    /// Resuelve un lote completo de pistas (útil para Playlists, Mixes y Radios de YouTube)
    pub async fn resolve_batch(
        &self,
        query: &str,
        custom_extractor_args: Option<&str>,
        custom_extra_args: Option<&str>,
        custom_cookies: Option<&str>,
    ) -> Result<ResolvedBatch, String> {
        let query_trimmed = query.trim();

        // 1. Revisar caché en memoria (si es pista individual conocida)
        if custom_extractor_args.is_none() && custom_extra_args.is_none() && custom_cookies.is_none() {
            if let Some(entry) = self.cache.get(query_trimmed) {
                let (meta, cached_at) = entry.value();
                if cached_at.elapsed() < self.ttl {
                    info!(query = %query_trimmed, title = %meta.title, "Source resolver cache hit");
                    return Ok(ResolvedBatch {
                        is_playlist: false,
                        playlist_title: None,
                        tracks: vec![meta.clone()],
                    });
                }
            }
        }

        // 2. Si es un stream directo a un endpoint de audio, no invocamos yt-dlp
        if Self::is_direct_stream(query_trimmed) {
            let meta = TrackMetadata {
                title: "Direct Stream".to_string(),
                source_url: query_trimmed.to_string(),
                stream_url: query_trimmed.to_string(),
                duration_secs: None,
                author: None,
                thumbnail: None,
            };
            return Ok(ResolvedBatch {
                is_playlist: false,
                playlist_title: None,
                tracks: vec![meta],
            });
        }

        info!(query = %query_trimmed, "Resolviendo stream CDN mediante yt-dlp (Soporte Playlist/Mix/Radio)...");

        // 3. Determinar cookies:
        // Caso A: Cookie personalizada en payload
        if let Some(c) = custom_cookies {
            let path = std::path::Path::new(c);
            let cookie_path = if path.exists() {
                c.to_string()
            } else if c.contains('\n') || c.contains("# Netscape") || c.contains(".google.com") {
                let temp_path = "/tmp/axaudio_custom_cookies.txt";
                let _ = tokio::fs::write(temp_path, c).await;
                temp_path.to_string()
            } else {
                c.to_string()
            };

            let batch = self
                .run_ytdlp(query_trimmed, custom_extractor_args, custom_extra_args, Some(&cookie_path))
                .await?;
            if !batch.is_playlist && batch.tracks.len() == 1 {
                self.cache.insert(query_trimmed.to_string(), (batch.tracks[0].clone(), Instant::now()));
            }
            return Ok(batch);
        }

        // Caso B: Rotación dentro del pool de cookies multi-cuenta
        let cookie_pool = self.get_available_cookie_pool().await;

        if cookie_pool.is_empty() {
            let batch = self
                .run_ytdlp(query_trimmed, custom_extractor_args, custom_extra_args, None)
                .await?;
            if !batch.is_playlist && batch.tracks.len() == 1 {
                self.cache.insert(query_trimmed.to_string(), (batch.tracks[0].clone(), Instant::now()));
            }
            return Ok(batch);
        }

        // Selección Round-Robin entre las cuentas del pool
        let total_cookies = cookie_pool.len();
        let start_index = self.cookie_rotation_index.fetch_add(1, Ordering::Relaxed) % total_cookies;

        let mut last_err = String::new();

        for i in 0..total_cookies {
            let current_idx = (start_index + i) % total_cookies;
            let current_cookie = &cookie_pool[current_idx];

            info!(
                cookie_index = current_idx,
                total_pool = total_cookies,
                cookie_file = %current_cookie,
                "Rotando conexión con cookie del pool para batch"
            );

            match self
                .run_ytdlp(query_trimmed, custom_extractor_args, custom_extra_args, Some(current_cookie))
                .await
            {
                Ok(batch) => {
                    if !batch.is_playlist && batch.tracks.len() == 1 {
                        self.cache.insert(query_trimmed.to_string(), (batch.tracks[0].clone(), Instant::now()));
                    }
                    info!(
                        tracks_count = batch.tracks.len(),
                        is_playlist = batch.is_playlist,
                        cookie_file = %current_cookie,
                        "Batch de audio resuelto exitosamente"
                    );
                    return Ok(batch);
                }
                Err(err) => {
                    warn!(
                        cookie_file = %current_cookie,
                        error = %err,
                        "Fallo con cookie del pool, intentando con siguiente cuenta..."
                    );
                    last_err = err;
                }
            }
        }

        Err(format!(
            "Todas las cookies del pool ({}) fallaron resolviendo la fuente. Último error: {}",
            total_cookies, last_err
        ))
    }

    /// Resuelve una única pista (compatible con /v1/tracks/resolve)
    pub async fn resolve(
        &self,
        query: &str,
        custom_extractor_args: Option<&str>,
        custom_extra_args: Option<&str>,
        custom_cookies: Option<&str>,
    ) -> Result<TrackMetadata, String> {
        let batch = self
            .resolve_batch(query, custom_extractor_args, custom_extra_args, custom_cookies)
            .await?;
        batch
            .tracks
            .into_iter()
            .next()
            .ok_or_else(|| "No se encontró ninguna pista reproducible".to_string())
    }
}

impl Default for SourceResolver {
    fn default() -> Self {
        Self::new()
    }
}
