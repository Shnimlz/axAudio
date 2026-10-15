use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
};
use songbird::driver::Driver;
use songbird::input::HttpRequest;
use songbird::ConnectionInfo;
use std::num::NonZeroU64;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::models::{
    ApiResponse, ControlPayload, JoinPayload, LeavePayload, PlayPayload, ResolvePayload,
    StatsResponse, TrackResponse,
};
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
    match state.resolver.resolve(&payload.query).await {
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

    // Optimización de CPU en 2 vCPUs: use_softclip(false) ahorra ~3% de ciclos por tick
    let driver_entry = state
        .players
        .entry(payload.guild_id)
        .or_insert_with(|| {
            let mut config = songbird::Config::default();
            config.preallocated_tracks = 1;
            config.use_softclip = false;
            Arc::new(Mutex::new(Driver::new(config)))
        });

    let mut driver = driver_entry.lock().await;
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
) -> Result<Json<ApiResponse<TrackResponse>>, (StatusCode, Json<ApiResponse<TrackResponse>>)> {
    let driver_ref = match state.players.get(&payload.guild_id) {
        Some(d) => d.clone(),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiResponse::err("No active voice connection for this guild. Call /v1/voice/join first.")),
            ));
        }
    };

    info!(guild = payload.guild_id, query = %payload.url, "Resolviendo fuente de audio para streaming");

    // Extracción de metadatos y URL CDN directa (o stream nativo)
    let track = match state.resolver.resolve(&payload.url).await {
        Ok(t) => t,
        Err(e) => {
            error!(guild = payload.guild_id, error = %e, "Fallo al resolver fuente con yt-dlp");
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::err(format!("Fallo resolviendo pista: {}", e))),
            ));
        }
    };

    info!(
        guild = payload.guild_id,
        title = %track.title,
        stream_url = %track.stream_url,
        "Iniciando streaming nativo de audio vía Tokio HttpRequest"
    );

    let source = HttpRequest::new(state.http_client.clone(), track.stream_url.clone());
    let mut driver = driver_ref.lock().await;

    // Detener cualquier stream anterior y limpiar cola antes de reproducir la nueva pista
    driver.stop();
    driver.queue().stop();

    let handle = driver.enqueue_input(source.into()).await;

    if let Some(vol) = payload.volume {
        let clamped = vol.clamp(0.0, 2.0);
        let _ = handle.set_volume(clamped);
    }

    let response = TrackResponse {
        title: track.title,
        source_url: track.source_url,
        stream_url: track.stream_url,
        duration_secs: track.duration_secs,
        author: track.author,
        thumbnail: track.thumbnail,
    };

    Ok(Json(ApiResponse::ok(response)))
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
