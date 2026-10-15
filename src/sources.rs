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

#[derive(Clone)]
pub struct SourceResolver {
    cache: Arc<DashMap<String, (TrackMetadata, Instant)>>,
    ttl: Duration,
}

impl SourceResolver {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(DashMap::new()),
            ttl: Duration::from_secs(3600 * 4), // 4 horas de TTL para streams de audio
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

    /// Resuelve la URL CDN directa y los metadatos usando yt-dlp con opciones y caché en memoria
    pub async fn resolve(
        &self,
        query: &str,
        custom_extractor_args: Option<&str>,
        custom_extra_args: Option<&str>,
        custom_cookies: Option<&str>,
    ) -> Result<TrackMetadata, String> {
        let query_trimmed = query.trim();

        // 1. Revisar caché en memoria (si no se pasan opciones personalizadas que alteren la extracción)
        if custom_extractor_args.is_none() && custom_extra_args.is_none() && custom_cookies.is_none() {
            if let Some(entry) = self.cache.get(query_trimmed) {
                let (meta, cached_at) = entry.value();
                if cached_at.elapsed() < self.ttl {
                    info!(query = %query_trimmed, title = %meta.title, "Source resolver cache hit");
                    return Ok(meta.clone());
                }
            }
        }

        // Si es un stream directo a un endpoint de audio, no invocamos yt-dlp
        if Self::is_direct_stream(query_trimmed) {
            let meta = TrackMetadata {
                title: "Direct Stream".to_string(),
                source_url: query_trimmed.to_string(),
                stream_url: query_trimmed.to_string(),
                duration_secs: None,
                author: None,
                thumbnail: None,
            };
            return Ok(meta);
        }

        info!(query = %query_trimmed, "Resolviendo stream CDN mediante yt-dlp...");

        let mut cmd = Command::new("yt-dlp");
        cmd.arg("--no-playlist")
            .arg("--dump-single-json");

        // 1. Formato personalizable (default: bestaudio/best)
        let format = std::env::var("YTDLP_FORMAT").unwrap_or_else(|_| "bestaudio/best".to_string());
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

        // 5. Soporte flexible de Cookies (prioridad: payload > YTDLP_COOKIES texto/env > archivos)
        let mut cookie_path_to_use: Option<String> = None;

        if let Some(c) = custom_cookies {
            let path = std::path::Path::new(c);
            if path.exists() {
                cookie_path_to_use = Some(c.to_string());
            } else if c.contains('\n') || c.contains("# Netscape") || c.contains(".google.com") {
                let temp_path = "/tmp/axaudio_custom_cookies.txt";
                if let Ok(()) = tokio::fs::write(temp_path, c).await {
                    cookie_path_to_use = Some(temp_path.to_string());
                }
            }
        }

        if cookie_path_to_use.is_none() {
            if let Ok(env_cookies) = std::env::var("YTDLP_COOKIES").or_else(|_| std::env::var("YOUTUBE_COOKIES")) {
                let trimmed = env_cookies.trim();
                let path = std::path::Path::new(trimmed);
                if path.exists() {
                    cookie_path_to_use = Some(trimmed.to_string());
                } else if trimmed.contains('\n') || trimmed.contains("# Netscape") || trimmed.contains(".google.com") {
                    let temp_path = "/tmp/axaudio_env_cookies.txt";
                    if let Ok(()) = tokio::fs::write(temp_path, trimmed).await {
                        cookie_path_to_use = Some(temp_path.to_string());
                    }
                }
            }
        }

        if cookie_path_to_use.is_none() {
            let cookie_candidates = [
                std::env::var("YTDLP_COOKIES_PATH").ok(),
                std::env::var("YOUTUBE_COOKIES_PATH").ok(),
                Some("/app/cookies.txt".to_string()),
                Some("cookies.txt".to_string()),
            ];
            for candidate in cookie_candidates.into_iter().flatten() {
                if std::path::Path::new(&candidate).exists() {
                    cookie_path_to_use = Some(candidate);
                    break;
                }
            }
        }

        if let Some(path) = cookie_path_to_use {
            info!(cookie_path = %path, "Utilizando cookies para autenticación yt-dlp");
            cmd.arg("--cookies").arg(path);
        }

        // 6. Argumentos adicionales libres (prioridad: env YTDLP_EXTRA_ARGS y custom_extra_args)
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
            warn!(query = %query_trimmed, stderr = %stderr, "yt-dlp no pudo resolver la fuente");
            return Err(format!("yt-dlp failed: {}", stderr.trim()));
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        let val: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| format!("Error parseando salida JSON de yt-dlp: {:?}", e))?;

        // En búsquedas (ytsearch:), yt-dlp encapsula el resultado dentro de entries[0]
        let item = if let Some(entries) = val.get("entries").and_then(|e| e.as_array()) {
            match entries.first() {
                Some(first) => first,
                None => return Err("Búsqueda sin resultados en YouTube".to_string()),
            }
        } else {
            &val
        };

        let stream_url = item
            .get("url")
            .and_then(|u| u.as_str())
            .ok_or_else(|| "No se encontró stream_url en los metadatos de yt-dlp".to_string())?
            .to_string();

        let title = item
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown Track")
            .to_string();

        let source_url = item
            .get("webpage_url")
            .and_then(|u| u.as_str())
            .unwrap_or(query_trimmed)
            .to_string();

        let duration_secs = item.get("duration").and_then(|d| d.as_u64());
        let author = item.get("uploader").and_then(|u| u.as_str()).map(|s| s.to_string());
        let thumbnail = item.get("thumbnail").and_then(|t| t.as_str()).map(|s| s.to_string());

        let meta = TrackMetadata {
            title,
            source_url,
            stream_url,
            duration_secs,
            author,
            thumbnail,
        };

        // Guardar en caché con el query original
        self.cache.insert(query_trimmed.to_string(), (meta.clone(), Instant::now()));

        info!(title = %meta.title, "Track resuelto exitosamente con URL CDN directa");
        Ok(meta)
    }
}

impl Default for SourceResolver {
    fn default() -> Self {
        Self::new()
    }
}
