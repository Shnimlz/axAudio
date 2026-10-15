use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
};
use songbird::driver::Driver;
use songbird::input::HttpRequest;
use songbird::tracks::Track;
use songbird::ConnectionInfo;
use std::num::NonZeroU64;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::models::{
    ApiResponse, ControlPayload, JoinPayload, LeavePayload, PlayPayload, PlayResponse,
    QueuePayload, QueueResponse, ResolvePayload, StatsResponse, TrackResponse,
};
use crate::sources::TrackMetadata;
use crate::state::AppState;

pub async fn health() -> Json<ApiResponse<&'static str>> {
    Json(ApiResponse::ok("axAudio engine operational"))
}

pub async fn get_stats(State(state): State<AppState>) -> Json<ApiResponse<StatsResponse>> {
    let stats = StatsResponse {
        status: "healthy",
        version: "0.1.0",
        uptime_seconds: state.start_time.elapsed().as_secs(),
        active_players: state.players.len(),
    };
    Json(ApiResponse::ok(stats))
}

pub async fn resolve_track(
    State(state): State<AppState>,
    Json(payload): Json<ResolvePayload>,
) -> Result<Json<ApiResponse<TrackResponse>>, (StatusCode, Json<ApiResponse<TrackResponse>>)> {
    match state
        .resolver
        .resolve(
            &payload.query,
            payload.extractor_args.as_deref(),
            payload.extra_args.as_deref(),
            payload.cookies.as_deref(),
        )
        .await
    {
        Ok(track) => {
            let res = TrackResponse {
                title: track.title,
                source_url: track.source_url,
                stream_url: track.stream_url,
                duration_secs: track.duration_secs,
                author: track.author,
                thumbnail: track.thumbnail,
            };
            Ok(Json(ApiResponse::ok(res)))
        }
        Err(err) => Err((
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::err(format!("Fallo resolviendo pista: {}", err))),
        )),
    }
}

pub async fn voice_join(
    State(state): State<AppState>,
    Json(payload): Json<JoinPayload>,
) -> Result<Json<ApiResponse<&'static str>>, (StatusCode, Json<ApiResponse<&'static str>>)> {
    let guild_id = match NonZeroU64::new(payload.guild_id) {
        Some(id) => id,
        None => return Err((StatusCode::BAD_REQUEST, Json(ApiResponse::err("Invalid guild_id: 0")))),
    };

    let channel_id = match NonZeroU64::new(payload.channel_id) {
        Some(id) => id,
        None => return Err((StatusCode::BAD_REQUEST, Json(ApiResponse::err("Invalid channel_id: 0")))),
    };

    let user_id = match NonZeroU64::new(payload.user_id) {
        Some(id) => id,
        None => return Err((StatusCode::BAD_REQUEST, Json(ApiResponse::err("Invalid user_id: 0")))),
    };

    info!(
        guild = payload.guild_id,
        channel = payload.channel_id,
        endpoint = %payload.endpoint,
        "Connecting axAudio driver to Discord Voice"
    );

    let connection_info = ConnectionInfo {
        channel_id: channel_id.into(),
        guild_id: guild_id.into(),
        endpoint: payload.endpoint,
        session_id: payload.session_id,
        token: payload.token,
        user_id: user_id.into(),
    };

    // Audio Fidelity Tuning: use_softclip(true) aplica limitador tanh suave a picos > 0 dBFS,
    // eliminando la distorsión armónica cuadrada (chillidos/estridencias) y reteniendo los graves intactos
    let driver_entry = state
        .players
        .entry(payload.guild_id)
        .or_insert_with(|| {
            let mut config = songbird::Config::default();
            config.preallocated_tracks = 1;
            config.use_softclip = true;
            Arc::new(Mutex::new(Driver::new(config)))
        });

    let mut driver = driver_entry.lock().await;

    // Configuración de Bitrate Opus de alta definición (por defecto 128 kbps, o variable AUDIO_BITRATE)
    let bitrate_bps = std::env::var("AUDIO_BITRATE")
        .ok()
        .and_then(|b| b.parse::<i32>().ok())
        .unwrap_or(128_000);
    driver.set_bitrate(songbird::driver::Bitrate::Bits(bitrate_bps));

    match driver.connect(connection_info).await {
        Ok(_) => {
            info!(guild = payload.guild_id, "Successfully connected to voice channel");
            Ok(Json(ApiResponse::ok("Connected to Discord voice channel")))
        }
        Err(e) => {
            error!(guild = payload.guild_id, error = ?e, "Failed to connect driver to voice channel");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::err(format!("Driver connection failed: {:?}", e))),
            ))
        }
    }
}

pub async fn player_play(
    State(state): State<AppState>,
    Json(payload): Json<PlayPayload>,
) -> Result<Json<ApiResponse<PlayResponse>>, (StatusCode, Json<ApiResponse<PlayResponse>>)> {
    let driver_ref = match state.players.get(&payload.guild_id) {
        Some(d) => d.clone(),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiResponse::err("No active voice connection for this guild. Call /v1/voice/join first.")),
            ));
        }
    };

    info!(guild = payload.guild_id, query = %payload.url, "Resolviendo fuente de audio (Pista/Playlist/Mix/Radio)...");

    // Extracción por lotes (Soporta canciones individuales, Playlists, Mixes RD... y Radios)
    let batch = match state
        .resolver
        .resolve_batch(
            &payload.url,
            payload.extractor_args.as_deref(),
            payload.extra_args.as_deref(),
            payload.cookies.as_deref(),
        )
        .await
    {
        Ok(b) => b,
        Err(e) => {
            error!(guild = payload.guild_id, error = %e, "Fallo al resolver lote de audio con yt-dlp");
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::err(format!("Fallo resolviendo pista(s): {}", e))),
            ));
        }
    };

    let mut driver = driver_ref.lock().await;

    // Si se solicita reemplazar explícitamente, se detiene la cola y el stream actual
    if payload.replace == Some(true) {
        driver.stop();
        driver.queue().stop();
    }

    let initial_queue_len = driver.queue().len();
    let target_volume = payload.volume.unwrap_or(1.0);
    let effective_volume = (target_volume * 0.95).clamp(0.0, 2.0);

    for track in &batch.tracks {
        let source = HttpRequest::new(state.http_client.clone(), track.stream_url.clone());
        let songbird_track = Track::new_with_data(source.into(), Arc::new(track.clone()));
        let handle = driver.enqueue(songbird_track).await;
        let _ = handle.set_volume(effective_volume);
    }

    let total_queue_len = driver.queue().len();
    let first_track = batch.tracks.first().cloned().unwrap();

    let response = PlayResponse {
        title: first_track.title,
        source_url: first_track.source_url,
        stream_url: first_track.stream_url,
        duration_secs: first_track.duration_secs,
        author: first_track.author,
        thumbnail: first_track.thumbnail,
        is_playlist: batch.is_playlist,
        playlist_title: batch.playlist_title,
        playlist_tracks_count: batch.tracks.len(),
        queue_length: total_queue_len,
        was_empty: initial_queue_len == 0,
    };

    info!(
        guild = payload.guild_id,
        tracks_enqueued = batch.tracks.len(),
        total_in_queue = total_queue_len,
        is_playlist = batch.is_playlist,
        "Pistas encoladas exitosamente en Songbird TrackQueue"
    );

    Ok(Json(ApiResponse::ok(response)))
}

pub async fn player_queue(
    State(state): State<AppState>,
    Json(payload): Json<QueuePayload>,
) -> Result<Json<ApiResponse<QueueResponse>>, (StatusCode, Json<ApiResponse<QueueResponse>>)> {
    let driver_ref = match state.players.get(&payload.guild_id) {
        Some(d) => d.clone(),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiResponse::err("No active voice player for this guild")),
            ));
        }
    };

    let driver = driver_ref.lock().await;
    let handles = driver.queue().current_queue();
    let is_empty = driver.queue().is_empty();

    let mut all_tracks = Vec::new();
    for handle in handles {
        let meta: Arc<TrackMetadata> = handle.data();
        all_tracks.push(TrackResponse {
            title: meta.title.clone(),
            source_url: meta.source_url.clone(),
            stream_url: meta.stream_url.clone(),
            duration_secs: meta.duration_secs,
            author: meta.author.clone(),
            thumbnail: meta.thumbnail.clone(),
        });
    }

    let current = if !is_empty && !all_tracks.is_empty() {
        Some(all_tracks.remove(0))
    } else {
        None
    };

    let total = (if current.is_some() { 1 } else { 0 }) + all_tracks.len();

    let res = QueueResponse {
        guild_id: payload.guild_id,
        is_playing: current.is_some(),
        current,
        queue: all_tracks,
        total_tracks: total,
    };

    Ok(Json(ApiResponse::ok(res)))
}

pub async fn player_control(
    State(state): State<AppState>,
    Json(payload): Json<ControlPayload>,
) -> Result<Json<ApiResponse<&'static str>>, (StatusCode, Json<ApiResponse<&'static str>>)> {
    let driver_ref = match state.players.get(&payload.guild_id) {
        Some(d) => d.clone(),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiResponse::err("No active voice player for this guild")),
            ));
        }
    };

    let mut driver = driver_ref.lock().await;

    match payload.action.as_str() {
        "pause" => {
            let _ = driver.queue().pause();
            Ok(Json(ApiResponse::ok("Player paused")))
        }
        "resume" => {
            let _ = driver.queue().resume();
            Ok(Json(ApiResponse::ok("Player resumed")))
        }
        "stop" => {
            driver.queue().stop();
            driver.stop();
            Ok(Json(ApiResponse::ok("Player stopped")))
        }
        "skip" => {
            let _ = driver.queue().skip();
            driver.stop();
            Ok(Json(ApiResponse::ok("Track skipped")))
        }
        "clear" => {
            driver.queue().modify_queue(|q| {
                if q.len() > 1 {
                    let first = q.pop_front().unwrap();
                    for track in q.drain(..) {
                        let _ = track.stop();
                    }
                    q.push_front(first);
                }
            });
            Ok(Json(ApiResponse::ok("Queue cleared")))
        }
        "volume" => {
            if let Some(val) = payload.value {
                let clamped = val.clamp(0.0, 2.0);
                if let Some(track) = driver.queue().current() {
                    let _ = track.set_volume(clamped);
                }
                Ok(Json(ApiResponse::ok("Volume updated")))
            } else {
                Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse::err("Missing 'value' for volume action")),
                ))
            }
        }
        other => Err((
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::err(format!("Unknown control action: '{}'", other))),
        )),
    }
}

pub async fn voice_leave(
    State(state): State<AppState>,
    Json(payload): Json<LeavePayload>,
) -> Result<Json<ApiResponse<&'static str>>, (StatusCode, Json<ApiResponse<&'static str>>)> {
    if let Some((_, driver_ref)) = state.players.remove(&payload.guild_id) {
        let mut driver = driver_ref.lock().await;
        driver.stop();
        driver.leave();
        info!(guild = payload.guild_id, "Driver disconnected and player removed");
        Ok(Json(ApiResponse::ok("Disconnected and cleaned up player")))
    } else {
        warn!(guild = payload.guild_id, "Attempted to leave non-existent voice session");
        Ok(Json(ApiResponse::ok("Session was not active")))
    }
}
