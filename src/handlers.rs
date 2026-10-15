use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::StatusCode,
    response::{IntoResponse, Json},
};
use async_trait::async_trait;
use songbird::driver::Driver;
use songbird::events::{Event, EventContext, EventHandler, TrackEvent};
use songbird::input::HttpRequest;
use songbird::tracks::{Track, TrackHandle};
use songbird::ConnectionInfo;
use std::num::NonZeroU64;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tracing::{error, info, warn};

use crate::models::{
    ApiResponse, ControlPayload, JoinPayload, LeavePayload, PlayPayload, PlayResponse,
    PlayerEvent, QueuePayload, QueueResponse, ResolvePayload, SearchPayload, SearchResponse,
    StatsResponse, TrackResponse,
};
use crate::sources::TrackMetadata;
use crate::state::AppState;

struct TrackPlayNotifier {
    guild_id: u64,
    meta: Arc<TrackMetadata>,
    event_tx: broadcast::Sender<PlayerEvent>,
}

#[async_trait]
impl EventHandler for TrackPlayNotifier {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        info!(guild = self.guild_id, title = %self.meta.title, "TrackPlay event fired");
        let _ = self.event_tx.send(PlayerEvent::TrackStart {
            guild_id: self.guild_id,
            track: TrackResponse {
                title: self.meta.title.clone(),
                source_url: self.meta.source_url.clone(),
                stream_url: self.meta.stream_url.clone(),
                duration_secs: self.meta.duration_secs,
                author: self.meta.author.clone(),
                thumbnail: self.meta.thumbnail.clone(),
            },
        });
        None
    }
}

struct TrackEndNotifier {
    guild_id: u64,
    meta: Arc<TrackMetadata>,
    event_tx: broadcast::Sender<PlayerEvent>,
}

#[async_trait]
impl EventHandler for TrackEndNotifier {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        info!(guild = self.guild_id, title = %self.meta.title, "TrackEnd event fired");
        let _ = self.event_tx.send(PlayerEvent::TrackEnd {
            guild_id: self.guild_id,
            track: Some(TrackResponse {
                title: self.meta.title.clone(),
                source_url: self.meta.source_url.clone(),
                stream_url: self.meta.stream_url.clone(),
                duration_secs: self.meta.duration_secs,
                author: self.meta.author.clone(),
                thumbnail: self.meta.thumbnail.clone(),
            }),
            reason: "finished".to_string(),
        });
        None
    }
}

struct TrackErrorNotifier {
    guild_id: u64,
    meta: Arc<TrackMetadata>,
    event_tx: broadcast::Sender<PlayerEvent>,
}

#[async_trait]
impl EventHandler for TrackErrorNotifier {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        let err_msg = match ctx {
            EventContext::Track(slice) => slice
                .first()
                .map(|(state, _)| format!("{:?}", state.playing))
                .unwrap_or_else(|| "Playback error".to_string()),
            _ => "Playback error".to_string(),
        };
        warn!(guild = self.guild_id, title = %self.meta.title, error = %err_msg, "TrackError event fired");
        let _ = self.event_tx.send(PlayerEvent::TrackError {
            guild_id: self.guild_id,
            track: Some(TrackResponse {
                title: self.meta.title.clone(),
                source_url: self.meta.source_url.clone(),
                stream_url: self.meta.stream_url.clone(),
                duration_secs: self.meta.duration_secs,
                author: self.meta.author.clone(),
                thumbnail: self.meta.thumbnail.clone(),
            }),
            error: err_msg,
        });
        None
    }
}

fn attach_track_events(
    guild_id: u64,
    meta: Arc<TrackMetadata>,
    handle: &TrackHandle,
    event_tx: &broadcast::Sender<PlayerEvent>,
) {
    let _ = handle.add_event(
        Event::Track(TrackEvent::Play),
        TrackPlayNotifier {
            guild_id,
            meta: meta.clone(),
            event_tx: event_tx.clone(),
        },
    );
    let _ = handle.add_event(
        Event::Track(TrackEvent::End),
        TrackEndNotifier {
            guild_id,
            meta: meta.clone(),
            event_tx: event_tx.clone(),
        },
    );
    let _ = handle.add_event(
        Event::Track(TrackEvent::Error),
        TrackErrorNotifier {
            guild_id,
            meta,
            event_tx: event_tx.clone(),
        },
    );
}


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
            payload.format.as_deref(),
        )
        .await
    {
        Ok(meta) => {
            let res = TrackResponse {
                title: meta.title,
                source_url: meta.source_url,
                stream_url: meta.stream_url,
                duration_secs: meta.duration_secs,
                author: meta.author,
                thumbnail: meta.thumbnail,
            };
            Ok(Json(ApiResponse::ok(res)))
        }
        Err(e) => {
            warn!(query = %payload.query, error = %e, "Track resolution failed");
            Err((
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::err(format!("Track resolution failed: {}", e))),
            ))
        }
    }
}

pub async fn search_tracks(
    State(state): State<AppState>,
    Json(payload): Json<SearchPayload>,
) -> Result<Json<ApiResponse<SearchResponse>>, (StatusCode, Json<ApiResponse<SearchResponse>>)> {
    let limit = payload.limit.unwrap_or(5);
    let source = payload.source.as_deref().unwrap_or("auto");

    match state
        .resolver
        .search(
            &payload.query,
            limit,
            source,
            payload.extractor_args.as_deref(),
            payload.extra_args.as_deref(),
            payload.cookies.as_deref(),
        )
        .await
    {
        Ok((source_used, tracks)) => {
            let res = SearchResponse {
                query: payload.query,
                source_used,
                tracks,
            };
            Ok(Json(ApiResponse::ok(res)))
        }
        Err(e) => {
            warn!(query = %payload.query, error = %e, "Track search failed");
            Err((
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::err(format!("Track search failed: {}", e))),
            ))
        }
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

    // Extracción optimizada (Fast Start): Pista 1 en paralelo con Flat-Playlist
    let batch = match state
        .resolver
        .resolve_playlist_batch(
            &payload.url,
            payload.extractor_args.as_deref(),
            payload.extra_args.as_deref(),
            payload.cookies.as_deref(),
            payload.format.as_deref(),
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
        state.next_epoch(payload.guild_id);
        state.pending_playlists.remove(&payload.guild_id);
        driver.stop();
        driver.queue().stop();
    }

    let initial_queue_len = driver.queue().len();
    let target_volume = payload.volume.unwrap_or(1.0);
    let effective_volume = (target_volume * 0.95).clamp(0.0, 2.0);

    // Encolar inmediatamente la pista 1 para inicio de reproducción instantáneo (~3-4s)
    let source = HttpRequest::new(state.http_client.clone(), batch.first_track.stream_url.clone());
    let first_meta = Arc::new(batch.first_track.clone());
    let songbird_track = Track::new_with_data(source.into(), first_meta.clone());
    let handle = driver.enqueue(songbird_track).await;
    let _ = handle.set_volume(effective_volume);
    attach_track_events(payload.guild_id, first_meta, &handle, &state.event_tx);

    let total_queue_len = driver.queue().len();
    drop(driver); // Liberar lock antes de despachar tareas asíncronas

    // Si es una playlist/mix con pistas pendientes, iniciar background worker
    if !batch.remaining_entries.is_empty() {
        let guild_id = payload.guild_id;
        let epoch = state.next_epoch(guild_id);
        state.pending_playlists.insert(guild_id, batch.remaining_entries.clone());

        let driver_clone = driver_ref.clone();
        let resolver_clone = state.resolver.clone();
        let http_client = state.http_client.clone();
        let pending_map = state.pending_playlists.clone();
        let epochs = state.load_epochs.clone();
        let event_tx_clone = state.event_tx.clone();
        let remaining = batch.remaining_entries;
        let extractor_args = payload.extractor_args.clone();
        let extra_args = payload.extra_args.clone();
        let cookies = payload.cookies.clone();
        let format_opt = payload.format.clone();

        tokio::spawn(async move {
            info!(
                guild = guild_id,
                count = remaining.len(),
                epoch = epoch,
                "Iniciando carga asíncrona de playlist en segundo plano..."
            );

            for item in remaining {
                // 1. Control de saturación de CPU y Buffer de cola:
                // Si la cola de Songbird ya tiene 3 o más pistas listas, pausar la resolución
                // hasta que la reproducción avance, evitando quemar CPU innecesariamente.
                loop {
                    let current_epoch = epochs
                        .get(&guild_id)
                        .map(|e| e.load(std::sync::atomic::Ordering::SeqCst))
                        .unwrap_or(0);
                    if current_epoch != epoch {
                        info!(guild = guild_id, "Carga de playlist en segundo plano cancelada");
                        return;
                    }

                    let in_queue = {
                        let d = driver_clone.lock().await;
                        d.queue().len()
                    };

                    if in_queue < 3 {
                        break;
                    }

                    tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
                }

                // 2. Cooldown para respirar la CPU entre llamadas a yt-dlp
                tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

                // Verificar si la sesión fue detenida o reemplazada
                let current_epoch = epochs
                    .get(&guild_id)
                    .map(|e| e.load(std::sync::atomic::Ordering::SeqCst))
                    .unwrap_or(0);
                if current_epoch != epoch {
                    info!(guild = guild_id, "Carga de playlist en segundo plano cancelada");
                    break;
                }

                match resolver_clone
                    .resolve_single(
                        &item.url,
                        extractor_args.as_deref(),
                        extra_args.as_deref(),
                        cookies.as_deref(),
                        format_opt.as_deref(),
                    )
                    .await
                {
                    Ok(track) => {
                        let cur_check = epochs
                            .get(&guild_id)
                            .map(|e| e.load(std::sync::atomic::Ordering::SeqCst))
                            .unwrap_or(0);
                        if cur_check != epoch {
                            break;
                        }

                        let mut d = driver_clone.lock().await;
                        let src = HttpRequest::new(http_client.clone(), track.stream_url.clone());
                        let track_meta = Arc::new(track.clone());
                        let trk = Track::new_with_data(src.into(), track_meta.clone());
                        let h = d.enqueue(trk).await;
                        let _ = h.set_volume(effective_volume);
                        attach_track_events(guild_id, track_meta, &h, &event_tx_clone);

                        if let Some(mut pend) = pending_map.get_mut(&guild_id) {
                            pend.retain(|p| p.url != item.url);
                        }
                        info!(
                            guild = guild_id,
                            title = %track.title,
                            in_queue = d.queue().len(),
                            "Pista de playlist resuelta y encolada en segundo plano"
                        );
                    }
                    Err(err) => {
                        warn!(
                            guild = guild_id,
                            url = %item.url,
                            error = %err,
                            "Fallo al resolver pista de playlist en background"
                        );
                        if let Some(mut pend) = pending_map.get_mut(&guild_id) {
                            pend.retain(|p| p.url != item.url);
                        }
                    }
                }
            }
            info!(guild = guild_id, "Carga en segundo plano de playlist completada");
        });
    }

    let response = PlayResponse {
        title: batch.first_track.title,
        source_url: batch.first_track.source_url,
        stream_url: batch.first_track.stream_url,
        duration_secs: batch.first_track.duration_secs,
        author: batch.first_track.author,
        thumbnail: batch.first_track.thumbnail,
        is_playlist: batch.is_playlist,
        playlist_title: batch.playlist_title,
        playlist_tracks_count: batch.total_tracks_count,
        queue_length: total_queue_len + if batch.is_playlist { batch.total_tracks_count.saturating_sub(1) } else { 0 },
        was_empty: initial_queue_len == 0,
    };

    info!(
        guild = payload.guild_id,
        is_playlist = batch.is_playlist,
        playlist_tracks_count = batch.total_tracks_count,
        "Pista inicial lista y encolada exitosamente"
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

    // Agregar pistas de playlist que aún se están cargando en segundo plano
    if let Some(pending) = state.pending_playlists.get(&payload.guild_id) {
        for item in pending.iter() {
            all_tracks.push(TrackResponse {
                title: format!("⏳ {}", item.title),
                source_url: item.url.clone(),
                stream_url: String::new(),
                duration_secs: item.duration_secs,
                author: item.author.clone(),
                thumbnail: item.thumbnail.clone(),
            });
        }
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
            state.next_epoch(payload.guild_id);
            state.pending_playlists.remove(&payload.guild_id);
            driver.queue().stop();
            driver.stop();
            Ok(Json(ApiResponse::ok("Player stopped")))
        }
        "skip" => {
            let _ = driver.queue().skip();
            Ok(Json(ApiResponse::ok("Track skipped")))
        }
        "clear" => {
            state.next_epoch(payload.guild_id);
            state.pending_playlists.remove(&payload.guild_id);
            driver.queue().modify_queue(|q| {
                if q.len() > 1 {
                    let first = q.pop_front().unwrap();
                    q.clear();
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
    state.next_epoch(payload.guild_id);
    state.pending_playlists.remove(&payload.guild_id);
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

pub async fn ws_events(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_ws_events(socket, state))
}

async fn handle_ws_events(mut socket: WebSocket, state: AppState) {
    let mut rx = state.event_tx.subscribe();
    info!("🔌 Client connected to axAudio WebSocket /v1/events");

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Ok(evt) => {
                        if let Ok(json) = serde_json::to_string(&evt) {
                            if socket.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        warn!("WebSocket client lagged behind, skipped {} events", missed);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    info!("🔌 Client disconnected from axAudio WebSocket /v1/events");
}

