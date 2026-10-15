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

    /// Resuelve la URL CDN directa y los metadatos usando yt-dlp con caché en memoria
    pub async fn resolve(&self, query: &str) -> Result<TrackMetadata, String> {
        let query_trimmed = query.trim();

        // 1. Revisar caché en memoria
        if let Some(entry) = self.cache.get(query_trimmed) {
            let (meta, cached_at) = entry.value();
            if cached_at.elapsed() < self.ttl {
                info!(query = %query_trimmed, title = %meta.title, "Source resolver cache hit");
                return Ok(meta.clone());
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

        // 2. Extraer metadatos con yt-dlp en formato JSON (--dump-single-json)
        // Usamos clientes TV e iOS para saltar el bloqueo de BotGuard (poToken)
        let mut cmd = Command::new("yt-dlp");
        cmd.arg("--no-playlist")
            .arg("--format")
            .arg("bestaudio/best")
            .arg("--dump-single-json")
            .arg("--default-search")
            .arg("ytsearch:")
            .arg("--extractor-args")
            .arg("youtube:player_client=tv,tv_embedded,ios,web");

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
