use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Request, State,
    },
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use async_trait::async_trait;
use songbird::driver::Driver;
use songbird::events::{Event, EventContext, EventHandler, TrackEvent};
use songbird::input::HttpRequest;
use songbird::tracks::{Track, TrackHandle};
use songbird::ConnectionInfo;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tracing::{error, info, warn};

use crate::dsp::{apply_preset_to_params, inject_dsp, DspParams};
use crate::models::{
    ApiResponse, AutoplayPayload, ControlPayload, DspPayload, DspResponse, EqPayload, JoinPayload, LeavePayload,
    LoopMode, PlayPayload, PlayResponse, PlayerEvent, QueuePayload, QueueResponse, ResolvePayload,
    SearchPayload, SearchResponse, StatsResponse, TrackResponse,
};
use crate::sources::{FlatPlaylistItem, SourceResolver, TrackMetadata};
use crate::state::AppState;

struct TrackPlayNotifier {
    guild_id: u64,
    meta: Arc<TrackMetadata>,
    event_tx: broadcast::Sender<PlayerEvent>,
    state: Option<AppState>,
    has_played: Arc<AtomicBool>,
}

#[async_trait]
impl EventHandler for TrackPlayNotifier {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        self.has_played.store(true, Ordering::SeqCst);
        info!(guild = self.guild_id, title = %self.meta.title, "Evento TrackPlay emitido");
        if let Some(ref st) = self.state {
            st.set_last_track(self.guild_id, (*self.meta).clone());
            if let Some(vid) = SourceResolver::extract_video_id(&self.meta.source_url) {
                st.record_seen_track(self.guild_id, &vid);
            }
        }
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
    state: Option<AppState>,
    has_played: Arc<AtomicBool>,
    is_retry_attempt: bool,
}

#[async_trait]
impl EventHandler for TrackEndNotifier {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if !self.has_played.load(Ordering::SeqCst) {
            info!(
                guild = self.guild_id,
                title = %self.meta.title,
                "TrackEnd ignorado: la pista fue detenida/desalojada antes de sonar"
            );
            return None;
        }
        let reason = match ctx {
            EventContext::Track(slice) => slice
                .first()
                .map(|(state, _)| match state.playing {
                    songbird::tracks::PlayMode::Stop => "stopped",
                    songbird::tracks::PlayMode::End => "finished",
                    songbird::tracks::PlayMode::Errored(_) => "error",
                    _ => "finished",
                })
                .unwrap_or("finished"),
            _ => "finished",
        };

        if reason == "error" && self.state.is_some() && !self.is_retry_attempt {
            return None;
        }

        info!(guild = self.guild_id, title = %self.meta.title, reason = %reason, "Evento TrackEnd emitido");
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
            reason: reason.to_string(),
        });

        // Bucle de cola: Si la pista terminó con éxito y el modo es Queue, re-encolar al final
        if reason == "finished" {
            if let Some(ref st) = self.state {
                if st.get_loop_mode(self.guild_id) == LoopMode::Queue {
                    let st_clone = st.clone();
                    let guild_id = self.guild_id;
                    let meta = self.meta.clone();
                    tokio::spawn(async move {
                        if let Some(driver_ref) = st_clone.players.get(&guild_id) {
                            let dsp_params = st_clone.get_dsp_params(guild_id);
                            let target_volume = st_clone.get_volume(guild_id);
                            let effective_volume = (target_volume * 0.95).clamp(0.0, 2.0);
                            let trk = create_track_with_dsp(
                                st_clone.http_client.clone(),
                                meta.stream_url.clone(),
                                meta.clone(),
                                dsp_params,
                            )
                            .await;

                            let mut driver = driver_ref.lock().await;
                            let handle = driver.enqueue(trk).await;
                            let _ = handle.set_volume(effective_volume);
                            attach_track_events(
                                guild_id,
                                meta,
                                &handle,
                                &st_clone.event_tx,
                                Some(st_clone.clone()),
                            );
                        }
                    });
                }
            }
        }

        // Activación de prefetch de seguridad: Si la cola se vació (por ejemplo, tras un skip
        // o corte antes de la ventana de 15s), avanzar inmediatamente a la siguiente pista pendiente
        if let Some(ref st) = self.state {
            let st_clone = st.clone();
            let guild_id = self.guild_id;
            tokio::spawn(async move {
                tokio::task::yield_now().await;
                maybe_prefetch_next_track(&st_clone, guild_id).await;
            });
        }

        None
    }
}

struct TrackErrorNotifier {
    guild_id: u64,
    meta: Arc<TrackMetadata>,
    event_tx: broadcast::Sender<PlayerEvent>,
    state: Option<AppState>,
    retried: Arc<AtomicBool>,
}

#[async_trait]
impl EventHandler for TrackErrorNotifier {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        let err_msg = match ctx {
            EventContext::Track(slice) => slice
                .first()
                .map(|(state, _)| format!("{:?}", state.playing))
                .unwrap_or_else(|| "Error de reproducción".to_string()),
            _ => "Error de reproducción".to_string(),
        };

        if let Some(ref st) = self.state {
            if !self.retried.swap(true, Ordering::SeqCst) {
                st.resolver.invalidate_cache(&self.meta.source_url);
                st.resolver.invalidate_cache(&self.meta.stream_url);

                warn!(
                    guild = self.guild_id,
                    title = %self.meta.title,
                    url = %self.meta.source_url,
                    error = %err_msg,
                    "TrackError detectado (posible 403 o corte de CDN). Purgando caché y reintentando automáticamente..."
                );

                let st_clone = st.clone();
                let guild_id = self.guild_id;
                let source_url = self.meta.source_url.clone();
                let event_tx_clone = self.event_tx.clone();

                tokio::spawn(async move {
                    let play_opts = st_clone
                        .guild_play_options
                        .get(&guild_id)
                        .map(|p| p.clone())
                        .unwrap_or_default();

                    match st_clone
                        .resolver
                        .resolve_single(
                            &source_url,
                            play_opts.extractor_args.as_deref(),
                            play_opts.extra_args.as_deref(),
                            play_opts.cookies.as_deref(),
                            play_opts.format.as_deref(),
                        )
                        .await
                    {
                        Ok(fresh_track) => {
                            if let Some(driver_ref) = st_clone.players.get(&guild_id) {
                                let dsp_params = st_clone.get_dsp_params(guild_id);
                                let target_vol = st_clone.get_volume(guild_id);
                                let effective_vol = (target_vol * 0.95).clamp(0.0, 2.0);
                                let track_meta = Arc::new(fresh_track.clone());

                                let songbird_track = create_track_with_dsp(
                                    st_clone.http_client.clone(),
                                    fresh_track.stream_url.clone(),
                                    track_meta.clone(),
                                    dsp_params,
                                )
                                .await;

                                let handle = {
                                    let mut driver = driver_ref.lock().await;
                                    let h = driver.enqueue(songbird_track).await;
                                    let _ = h.set_volume(effective_vol);
                                    h
                                };

                                let retried_flag = Arc::new(AtomicBool::new(true));
                                attach_track_events_with_retried(
                                    guild_id,
                                    track_meta,
                                    &handle,
                                    &event_tx_clone,
                                    Some(st_clone.clone()),
                                    retried_flag,
                                );

                                if st_clone.get_loop_mode(guild_id) == LoopMode::Track {
                                    let _ = handle.enable_loop();
                                }

                                info!(
                                    guild = guild_id,
                                    title = %fresh_track.title,
                                    "Auto-curación exitosa: pista re-resuelta con stream fresco y reanudada"
                                );
                                return;
                            }
                        }
                        Err(e) => {
                            warn!(
                                guild = guild_id,
                                error = %e,
                                "Reintento de auto-curación falló al re-resolver la pista"
                            );
                        }
                    }

                    let _ = event_tx_clone.send(PlayerEvent::TrackError {
                        guild_id,
                        track: Some(TrackResponse {
                            title: source_url.clone(),
                            source_url,
                            stream_url: String::new(),
                            duration_secs: None,
                            author: None,
                            thumbnail: None,
                        }),
                        error: err_msg,
                    });
                    maybe_prefetch_next_track(&st_clone, guild_id).await;
                });

                return None;
            }
        }

        warn!(
            guild = self.guild_id,
            title = %self.meta.title,
            error = %err_msg,
            "Evento TrackError emitido (sin más reintentos)"
        );
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

fn shuffle_slice<T>(slice: &mut [T]) {
    if slice.len() <= 1 {
        return;
    }
    let mut rng_state = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    if rng_state == 0 {
        rng_state = 0x853c49e6748fea9b;
    }
    for i in (1..slice.len()).rev() {
        rng_state ^= rng_state >> 12;
        rng_state ^= rng_state << 25;
        rng_state ^= rng_state >> 27;
        let rand_val = rng_state.wrapping_mul(0x2545F4914F6CDD1D);
        let j = (rand_val as usize) % (i + 1);
        slice.swap(i, j);
    }
}

fn attach_track_events(
    guild_id: u64,
    meta: Arc<TrackMetadata>,
    handle: &TrackHandle,
    event_tx: &broadcast::Sender<PlayerEvent>,
    state: Option<AppState>,
) {
    let retried = Arc::new(AtomicBool::new(false));
    attach_track_events_with_retried(guild_id, meta, handle, event_tx, state, retried);
}

fn attach_track_events_with_retried(
    guild_id: u64,
    meta: Arc<TrackMetadata>,
    handle: &TrackHandle,
    event_tx: &broadcast::Sender<PlayerEvent>,
    state: Option<AppState>,
    retried: Arc<AtomicBool>,
) {
    let has_played = Arc::new(AtomicBool::new(false));
    let is_retry_attempt = retried.load(Ordering::SeqCst);

    let _ = handle.add_event(
        Event::Track(TrackEvent::Play),
        TrackPlayNotifier {
            guild_id,
            meta: meta.clone(),
            event_tx: event_tx.clone(),
            state: state.clone(),
            has_played: has_played.clone(),
        },
    );
    let _ = handle.add_event(
        Event::Track(TrackEvent::End),
        TrackEndNotifier {
            guild_id,
            meta: meta.clone(),
            event_tx: event_tx.clone(),
            state: state.clone(),
            has_played,
            is_retry_attempt,
        },
    );
    let _ = handle.add_event(
        Event::Track(TrackEvent::Error),
        TrackErrorNotifier {
            guild_id,
            meta,
            event_tx: event_tx.clone(),
            state,
            retried,
        },
    );
}


pub const AXAUDIO_VERSION: &str = match option_env!("AXAUDIO_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

pub async fn health() -> Json<ApiResponse<&'static str>> {
    Json(ApiResponse::ok("Motor axAudio operativo"))
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    if let Some(ref expected_pw) = state.auth_token {
        let header_authorized = req
            .headers()
            .get(header::AUTHORIZATION)
            .or_else(|| req.headers().get("x-axaudio-password"))
            .and_then(|h| h.to_str().ok())
            .map(|val| {
                let token = val.strip_prefix("Bearer ").unwrap_or(val).trim();
                token == expected_pw.as_str()
            })
            .unwrap_or(false);

        let query_authorized = if !header_authorized {
            req.uri().query().map(|q| {
                q.split('&').any(|pair| {
                    let mut parts = pair.splitn(2, '=');
                    let key = parts.next().unwrap_or("");
                    let val = parts.next().unwrap_or("");
                    (key == "auth" || key == "password") && val == expected_pw.as_str()
                })
            }).unwrap_or(false)
        } else {
            true
        };

        if !query_authorized {
            warn!(uri = %req.uri(), "Petición no autorizada rechazada por el middleware de autenticación");
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse::<()>::err("No autorizado: token de autenticación inválido o faltante")),
            ).into_response();
        }
    }

    next.run(req).await
}

pub async fn get_stats(State(state): State<AppState>) -> Json<ApiResponse<StatsResponse>> {
    let stats = StatsResponse {
        status: "healthy",
        version: AXAUDIO_VERSION,
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
            warn!(query = %payload.query, error = %e, "Fallo al resolver la pista");
            Err((
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::err(format!("Fallo al resolver la pista: {}", e))),
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
            warn!(query = %payload.query, error = %e, "Fallo en la búsqueda de pistas");
            Err((
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::err(format!("Fallo en la búsqueda de pistas: {}", e))),
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
        None => return Err((StatusCode::BAD_REQUEST, Json(ApiResponse::err("guild_id inválido: 0")))),
    };

    let channel_id = match NonZeroU64::new(payload.channel_id) {
        Some(id) => id,
        None => return Err((StatusCode::BAD_REQUEST, Json(ApiResponse::err("channel_id inválido: 0")))),
    };

    let user_id = match NonZeroU64::new(payload.user_id) {
        Some(id) => id,
        None => return Err((StatusCode::BAD_REQUEST, Json(ApiResponse::err("user_id inválido: 0")))),
    };

    info!(
        guild = payload.guild_id,
        channel = payload.channel_id,
        endpoint = %payload.endpoint,
        "Conectando driver de axAudio al canal de voz de Discord"
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
    let driver_ref = state
        .players
        .entry(payload.guild_id)
        .or_insert_with(|| {
            let mut config = songbird::Config::default();
            config.preallocated_tracks = 1;
            config.use_softclip = true;
            Arc::new(Mutex::new(Driver::new(config)))
        })
        .value()
        .clone();

    let mut driver = driver_ref.lock().await;

    // Configuración de Bitrate Opus de alta definición (por defecto 128 kbps, o variable AUDIO_BITRATE)
    let bitrate_bps = std::env::var("AUDIO_BITRATE")
        .ok()
        .and_then(|b| b.parse::<i32>().ok())
        .unwrap_or(128_000);
    driver.set_bitrate(songbird::driver::Bitrate::Bits(bitrate_bps));

    match driver.connect(connection_info).await {
        Ok(_) => {
            state.touch_activity(payload.guild_id);
            info!(guild = payload.guild_id, "Conexión exitosa al canal de voz");
            Ok(Json(ApiResponse::ok("Conectado al canal de voz de Discord")))
        }
        Err(e) => {
            error!(guild = payload.guild_id, error = ?e, "Fallo al conectar el driver al canal de voz");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::err(format!("Fallo en la conexión del driver: {:?}", e))),
            ))
        }
    }
}

async fn create_track_with_dsp(
    http_client: reqwest::Client,
    stream_url: String,
    meta: Arc<TrackMetadata>,
    dsp_params: Arc<std::sync::RwLock<DspParams>>,
) -> Track {
    let source = HttpRequest::new(http_client.clone(), stream_url.clone());
    let input: songbird::input::Input = source.into();

    match input
        .make_playable_async(
            songbird::input::codecs::get_codec_registry(),
            songbird::input::codecs::get_probe(),
        )
        .await
    {
        Ok(mut playable) => {
            if let Some(parsed) = playable.parsed_mut() {
                inject_dsp(parsed, dsp_params);
            }
            Track::new_with_data(playable, meta)
        }
        Err(e) => {
            warn!(error = ?e, "Pre-parsing falló para DSP, usando entrada nativa de Songbird");
            let fallback = HttpRequest::new(http_client, stream_url);
            Track::new_with_data(fallback.into(), meta)
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
                Json(ApiResponse::err("No hay conexión de voz activa para este servidor. Ejecute /v1/voice/join primero.")),
            ));
        }
    };

    state.touch_activity(payload.guild_id);

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
            payload.limit,
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

    // Si se solicita reemplazar explícitamente, se detiene la cola y el stream actual
    if payload.replace == Some(true) {
        state.next_epoch(payload.guild_id);
        state.pending_playlists.remove(&payload.guild_id);
        let mut d = driver_ref.lock().await;
        d.stop();
        d.queue().stop();
    }

    let target_volume = payload.volume.unwrap_or_else(|| state.get_volume(payload.guild_id));
    state.set_volume(payload.guild_id, target_volume);
    let effective_volume = (target_volume * 0.95).clamp(0.0, 2.0);

    let dsp_params = state.get_dsp_params(payload.guild_id);

    // Encolar inmediatamente la pista 1 con el pipeline DSP inyectado
    // IMPORTANTE: create_track_with_dsp conecta vía HTTP para resolver headers del stream.
    // Lo ejecutamos FUERA del lock del Driver para no bloquear /queue, /control ni /leave.
    let first_meta = Arc::new(batch.first_track.clone());
    let songbird_track = create_track_with_dsp(
        state.http_client.clone(),
        batch.first_track.stream_url.clone(),
        first_meta.clone(),
        dsp_params.clone(),
    )
    .await;

    let (handle, initial_queue_len, total_queue_len) = {
        let mut driver = driver_ref.lock().await;
        let init_len = driver.queue().len();
        let h = driver.enqueue(songbird_track).await;
        let _ = h.set_volume(effective_volume);
        let tot_len = driver.queue().len();
        (h, init_len, tot_len)
    };
    attach_track_events(
        payload.guild_id,
        first_meta,
        &handle,
        &state.event_tx,
        Some(state.clone()),
    );
    if state.get_loop_mode(payload.guild_id) == LoopMode::Track {
        let _ = handle.enable_loop();
    }

    // Si es una playlist/mix con pistas pendientes, registrar en cola pendiente y opciones de reproducción.
    // El motor global de prefetch anticipado preparará cada pista 10-15s antes del final de la pista activa,
    // garantizando streams HTTP frescos, cero saturación de sockets y transiciones 100% gapless.
    if !batch.remaining_entries.is_empty() {
        let guild_id = payload.guild_id;
        state.next_epoch(guild_id);
        state.pending_playlists.insert(guild_id, batch.remaining_entries.clone());
        state.guild_play_options.insert(
            guild_id,
            crate::state::GuildPlayOptions {
                extractor_args: payload.extractor_args.clone(),
                extra_args: payload.extra_args.clone(),
                cookies: payload.cookies.clone(),
                format: payload.format.clone(),
            },
        );

        info!(
            guild = guild_id,
            pending_count = batch.remaining_entries.len(),
            "Playlist registrada en cola pendiente: el motor de prefetch anticipado resolverá cada pista 10-15s antes de su corte"
        );

        // Si la pista inicial es corta (<= 20s), evaluar prefetch inmediatamente
        let state_trigger = state.clone();
        tokio::spawn(async move {
            maybe_prefetch_next_track(&state_trigger, guild_id).await;
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
                Json(ApiResponse::err("No hay reproductor de voz activo para este servidor")),
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
                title: format!("[Pendiente] {}", item.title),
                source_url: item.url.clone(),
                stream_url: String::new(),
                duration_secs: item.duration_secs,
                author: item.author.clone(),
                thumbnail: item.thumbnail.clone(),
            });
        }
    }

    let current_position_secs = if let Some(current_handle) = driver.queue().current() {
        current_handle
            .get_info()
            .await
            .ok()
            .map(|info| info.position.as_secs_f64())
    } else {
        None
    };

    let current = if !is_empty && !all_tracks.is_empty() {
        Some(all_tracks.remove(0))
    } else {
        None
    };

    let total = (if current.is_some() { 1 } else { 0 }) + all_tracks.len();
    let loop_mode = state.get_loop_mode(payload.guild_id).as_str().to_string();
    let autoplay = state.is_autoplay(payload.guild_id);

    let res = QueueResponse {
        guild_id: payload.guild_id,
        is_playing: current.is_some(),
        current,
        current_position_secs,
        loop_mode,
        autoplay,
        queue: all_tracks,
        total_tracks: total,
    };

    Ok(Json(ApiResponse::ok(res)))
}

pub async fn player_control(
    State(state): State<AppState>,
    Json(payload): Json<ControlPayload>,
) -> Result<Json<ApiResponse<String>>, (StatusCode, Json<ApiResponse<String>>)> {
    let driver_ref = match state.players.get(&payload.guild_id) {
        Some(d) => d.clone(),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiResponse::err("No hay reproductor de voz activo para este servidor")),
            ));
        }
    };

    state.touch_activity(payload.guild_id);

    let mut driver = driver_ref.lock().await;

    match payload.action.as_str() {
        "pause" => {
            let _ = driver.queue().pause();
            Ok(Json(ApiResponse::ok("Reproductor pausado".to_string())))
        }
        "resume" => {
            let _ = driver.queue().resume();
            Ok(Json(ApiResponse::ok("Reproductor reanudado".to_string())))
        }
        "stop" => {
            state.next_epoch(payload.guild_id);
            state.pending_playlists.remove(&payload.guild_id);
            state.end_prefetch(payload.guild_id);
            driver.queue().stop();
            driver.stop();
            Ok(Json(ApiResponse::ok("Reproductor detenido".to_string())))
        }
        "skip" => {
            let _ = driver.queue().skip();
            Ok(Json(ApiResponse::ok("Pista saltada".to_string())))
        }
        "clear" => {
            state.next_epoch(payload.guild_id);
            state.pending_playlists.remove(&payload.guild_id);
            state.end_prefetch(payload.guild_id);
            driver.queue().modify_queue(|q| {
                if q.len() > 1 {
                    let first = q.pop_front().unwrap();
                    for handle in q.drain(..) {
                        let _ = handle.stop();
                    }
                    q.push_front(first);
                }
            });
            Ok(Json(ApiResponse::ok("Cola limpiada".to_string())))
        }
        "volume" => {
            if let Some(val) = payload.value {
                let clamped = val.clamp(0.0, 2.0);
                state.set_volume(payload.guild_id, clamped);
                if let Some(track) = driver.queue().current() {
                    let _ = track.set_volume(clamped * 0.95);
                }
                Ok(Json(ApiResponse::ok(format!("Volumen actualizado al {:.0}%", clamped * 100.0))))
            } else {
                Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse::err("Falta el parámetro 'value' para la acción de volumen")),
                ))
            }
        }
        "seek" => {
            let pos_secs = payload
                .position_secs
                .or_else(|| payload.value.map(|v| v as f64))
                .unwrap_or(0.0)
                .max(0.0);

            if let Some(track) = driver.queue().current() {
                let duration = std::time::Duration::from_secs_f64(pos_secs);
                match track.seek_async(duration).await {
                    Ok(actual) => Ok(Json(ApiResponse::ok(format!(
                        "Posición cambiada a {:.2}s",
                        actual.as_secs_f64()
                    )))),
                    Err(e) => Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse::err(format!("Fallo al cambiar posición: {:?}", e))),
                    )),
                }
            } else {
                Err((
                    StatusCode::NOT_FOUND,
                    Json(ApiResponse::err("No hay ninguna pista reproduciéndose actualmente para cambiar de posición")),
                ))
            }
        }
        "loop" => {
            let mode_str = payload.mode.as_deref().unwrap_or("track").to_lowercase();
            let mode = match mode_str.as_str() {
                "track" | "single" | "song" => LoopMode::Track,
                "queue" | "all" => LoopMode::Queue,
                "off" | "none" | "disabled" => LoopMode::Off,
                other => {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(ApiResponse::err(format!(
                            "Modo de bucle desconocido '{}'. Use 'track', 'queue', u 'off'",
                            other
                        ))),
                    ))
                }
            };

            state.set_loop_mode(payload.guild_id, mode);

            if let Some(track) = driver.queue().current() {
                if mode == LoopMode::Track {
                    let _ = track.enable_loop();
                } else {
                    let _ = track.disable_loop();
                }
            }

            Ok(Json(ApiResponse::ok(format!(
                "Modo de bucle configurado en '{}'",
                mode.as_str()
            ))))
        }
        "shuffle" => {
            let mut upcoming_count = 0;
            driver.queue().modify_queue(|q| {
                if q.len() > 2 {
                    let current = q.pop_front().unwrap();
                    let mut upcoming: Vec<_> = q.drain(..).collect();
                    shuffle_slice(&mut upcoming);
                    upcoming_count = upcoming.len();
                    q.push_front(current);
                    for track in upcoming {
                        q.push_back(track);
                    }
                } else if q.len() == 2 {
                    upcoming_count = 1;
                }
            });

            if let Some(mut pend) = state.pending_playlists.get_mut(&payload.guild_id) {
                shuffle_slice(&mut pend);
                upcoming_count += pend.len();
            }

            Ok(Json(ApiResponse::ok(format!(
                "Cola mezclada: {} pista(s) aleatorizada(s)",
                upcoming_count
            ))))
        }
        "remove" => {
            let target_idx = match payload.index {
                Some(i) => i,
                None => {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(ApiResponse::err("Falta el parámetro 'index' para la acción de eliminar")),
                    ))
                }
            };

            let mut removed_title = None;
            driver.queue().modify_queue(|q| {
                let queue_idx = target_idx + 1;
                if queue_idx < q.len() {
                    if let Some(handle) = q.remove(queue_idx) {
                        let meta: Arc<TrackMetadata> = handle.data();
                        removed_title = Some(meta.title.clone());
                        let _ = handle.stop();
                    }
                }
            });

            if removed_title.is_none() {
                let q_upcoming_len = driver.queue().len().saturating_sub(1);
                if target_idx >= q_upcoming_len {
                    let pending_idx = target_idx - q_upcoming_len;
                    if let Some(mut pend) = state.pending_playlists.get_mut(&payload.guild_id) {
                        if pending_idx < pend.len() {
                            let item = pend.remove(pending_idx);
                            removed_title = Some(item.title);
                        }
                    }
                }
            }

            match removed_title {
                Some(title) => Ok(Json(ApiResponse::ok(format!("Se eliminó '{}' de la cola", title)))),
                None => Err((
                    StatusCode::NOT_FOUND,
                    Json(ApiResponse::err(format!(
                        "Índice {} fuera de los límites de la cola",
                        target_idx
                    ))),
                )),
            }
        }
        "move" => {
            let from = match payload.from_index {
                Some(f) => f,
                None => {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(ApiResponse::err("Falta 'from_index' para la acción de mover")),
                    ))
                }
            };
            let to = match payload.to_index {
                Some(t) => t,
                None => {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(ApiResponse::err("Falta 'to_index' para la acción de mover")),
                    ))
                }
            };

            let q_upcoming_len = driver.queue().len().saturating_sub(1);
            let pending_len = state
                .pending_playlists
                .get(&payload.guild_id)
                .map(|p| p.len())
                .unwrap_or(0);
            let total_upcoming = q_upcoming_len + pending_len;

            if from >= total_upcoming || to >= total_upcoming {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse::err(format!(
                        "Parámetros 'from_index' ({}) o 'to_index' ({}) fuera de los límites de la cola (total: {})",
                        from, to, total_upcoming
                    ))),
                ));
            }

            if from == to {
                let current_title = if from < q_upcoming_len {
                    driver
                        .queue()
                        .modify_queue(|q| {
                            q.get(from + 1).map(|h| {
                                let meta: Arc<TrackMetadata> = h.data();
                                meta.title.clone()
                            })
                        })
                        .unwrap_or_else(|| "Pista".to_string())
                } else {
                    state
                        .pending_playlists
                        .get(&payload.guild_id)
                        .and_then(|p| p.get(from - q_upcoming_len).map(|it| it.title.clone()))
                        .unwrap_or_else(|| "Pista".to_string())
                };

                return Ok(Json(ApiResponse::ok(format!(
                    "Se movió '{}' de la posición {} a la {}",
                    current_title, from, to
                ))));
            }

            let mut moved_title = None;
            let mut trigger_prefetch = false;

            if from < q_upcoming_len && to < q_upcoming_len {
                // Caso 1: Ambas posiciones están en la cola interna precargada de Songbird
                driver.queue().modify_queue(|q| {
                    let from_q = from + 1;
                    let to_q = to + 1;
                    if let Some(handle) = q.remove(from_q) {
                        let meta: Arc<TrackMetadata> = handle.data();
                        moved_title = Some(meta.title.clone());
                        q.insert(to_q, handle);
                    }
                });
            } else if from >= q_upcoming_len && to >= q_upcoming_len {
                // Caso 2: Ambas posiciones están en state.pending_playlists
                let from_p = from - q_upcoming_len;
                let to_p = to - q_upcoming_len;
                if let Some(mut pend) = state.pending_playlists.get_mut(&payload.guild_id) {
                    if from_p < pend.len() && to_p < pend.len() {
                        let item = pend.remove(from_p);
                        moved_title = Some(item.title.clone());
                        pend.insert(to_p, item);
                    }
                }
            } else if from < q_upcoming_len && to >= q_upcoming_len {
                // Caso 3: Mover desde la cola interna de Songbird hacia la cola pendiente
                let mut moved_item = None;
                driver.queue().modify_queue(|q| {
                    let from_q = from + 1;
                    if let Some(handle) = q.remove(from_q) {
                        let meta: Arc<TrackMetadata> = handle.data();
                        moved_title = Some(meta.title.clone());
                        let _ = handle.stop();
                        moved_item = Some(FlatPlaylistItem {
                            title: meta.title.clone(),
                            url: meta.source_url.clone(),
                            duration_secs: meta.duration_secs,
                            author: meta.author.clone(),
                            thumbnail: meta.thumbnail.clone(),
                        });
                    }
                });

                if let Some(item) = moved_item {
                    let new_q_upcoming_len = driver.queue().len().saturating_sub(1);
                    let mut pend = state.pending_playlists.entry(payload.guild_id).or_default();
                    let to_p = (to - new_q_upcoming_len).min(pend.len());
                    pend.insert(to_p, item);
                    trigger_prefetch = true;
                }
            } else {
                // Caso 4: Mover desde la cola pendiente hacia la cola interna activa (to < q_upcoming_len)
                let from_p = from - q_upcoming_len;
                let item_opt = state
                    .pending_playlists
                    .get_mut(&payload.guild_id)
                    .and_then(|mut pend| {
                        if from_p < pend.len() {
                            Some(pend.remove(from_p))
                        } else {
                            None
                        }
                    });

                if let Some(item) = item_opt {
                    moved_title = Some(item.title.clone());
                    let mut displaced = Vec::new();
                    driver.queue().modify_queue(|q| {
                        while q.len() > to + 1 {
                            if let Some(handle) = q.pop_back() {
                                let meta: Arc<TrackMetadata> = handle.data();
                                let _ = handle.stop();
                                displaced.push(FlatPlaylistItem {
                                    title: meta.title.clone(),
                                    url: meta.source_url.clone(),
                                    duration_secs: meta.duration_secs,
                                    author: meta.author.clone(),
                                    thumbnail: meta.thumbnail.clone(),
                                });
                            }
                        }
                    });
                    displaced.reverse();

                    let mut pend = state.pending_playlists.entry(payload.guild_id).or_default();
                    pend.insert(0, item);
                    for (i, d) in displaced.into_iter().enumerate() {
                        pend.insert(1 + i, d);
                    }
                    state.end_prefetch(payload.guild_id);
                    trigger_prefetch = true;
                }
            }

            if trigger_prefetch {
                let st_clone = state.clone();
                let guild_id = payload.guild_id;
                tokio::spawn(async move {
                    maybe_prefetch_next_track(&st_clone, guild_id).await;
                });
            }

            match moved_title {
                Some(title) => Ok(Json(ApiResponse::ok(format!(
                    "Se movió '{}' de la posición {} a la {}",
                    title, from, to
                )))),
                None => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse::err("Error al reordenar la pista en la cola")),
                )),
            }
        }
        "autoplay" => {
            let new_val = if let Some(e) = payload.enabled {
                state.set_autoplay(payload.guild_id, e);
                e
            } else {
                let toggled = !state.is_autoplay(payload.guild_id);
                state.set_autoplay(payload.guild_id, toggled);
                toggled
            };

            if new_val {
                let st_clone = state.clone();
                let guild_id = payload.guild_id;
                tokio::spawn(async move {
                    maybe_prefetch_next_track(&st_clone, guild_id).await;
                });
            }

            Ok(Json(ApiResponse::ok(format!(
                "Autoplay {}",
                if new_val { "activado" } else { "desactivado" }
            ))))
        }
        other => Err((
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::err(format!("Acción de control desconocida: '{}'", other))),
        )),
    }
}

pub async fn player_seek(
    State(state): State<AppState>,
    Json(mut payload): Json<ControlPayload>,
) -> Result<Json<ApiResponse<String>>, (StatusCode, Json<ApiResponse<String>>)> {
    payload.action = "seek".to_string();
    player_control(State(state), Json(payload)).await
}

pub async fn player_shuffle(
    State(state): State<AppState>,
    Json(mut payload): Json<ControlPayload>,
) -> Result<Json<ApiResponse<String>>, (StatusCode, Json<ApiResponse<String>>)> {
    payload.action = "shuffle".to_string();
    player_control(State(state), Json(payload)).await
}

pub async fn player_loop(
    State(state): State<AppState>,
    Json(mut payload): Json<ControlPayload>,
) -> Result<Json<ApiResponse<String>>, (StatusCode, Json<ApiResponse<String>>)> {
    payload.action = "loop".to_string();
    player_control(State(state), Json(payload)).await
}

pub async fn player_autoplay(
    State(state): State<AppState>,
    Json(payload): Json<AutoplayPayload>,
) -> Result<Json<ApiResponse<bool>>, (StatusCode, Json<ApiResponse<bool>>)> {
    let new_val = if let Some(e) = payload.enabled {
        state.set_autoplay(payload.guild_id, e);
        e
    } else {
        let toggled = !state.is_autoplay(payload.guild_id);
        state.set_autoplay(payload.guild_id, toggled);
        toggled
    };

    if new_val {
        let st_clone = state.clone();
        let guild_id = payload.guild_id;
        tokio::spawn(async move {
            maybe_prefetch_next_track(&st_clone, guild_id).await;
        });
    }

    Ok(Json(ApiResponse::ok(new_val)))
}

/// Desconecta el driver de Songbird y purga todos los registros del servidor en AppState
pub async fn cleanup_guild_player(state: &AppState, guild_id: u64) -> bool {
    state.next_epoch(guild_id);
    state.pending_playlists.remove(&guild_id);
    state.end_prefetch(guild_id);
    state.guild_play_options.remove(&guild_id);
    state.dsp_params.remove(&guild_id);
    state.load_epochs.remove(&guild_id);
    state.guild_volumes.remove(&guild_id);
    state.loop_modes.remove(&guild_id);
    state.autoplay_guilds.remove(&guild_id);
    state.seen_tracks.remove(&guild_id);
    state.last_tracks.remove(&guild_id);
    state.last_activity.remove(&guild_id);

    if let Some((_, driver_ref)) = state.players.remove(&guild_id) {
        let mut driver = driver_ref.lock().await;
        driver.stop();
        driver.leave();
        true
    } else {
        false
    }
}

pub async fn voice_leave(
    State(state): State<AppState>,
    Json(payload): Json<LeavePayload>,
) -> Result<Json<ApiResponse<&'static str>>, (StatusCode, Json<ApiResponse<&'static str>>)> {
    if cleanup_guild_player(&state, payload.guild_id).await {
        info!(guild = payload.guild_id, "Driver desconectado y reproductor eliminado");
        Ok(Json(ApiResponse::ok("Desconectado y reproductor limpiado")))
    } else {
        warn!(guild = payload.guild_id, "Intento de abandonar una sesión de voz inexistente");
        Ok(Json(ApiResponse::ok("La sesión no estaba activa")))
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
    let mut ping_interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
    info!("Cliente conectado al WebSocket /v1/events");

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                if socket.send(Message::Ping(vec![].into())).await.is_err() {
                    break;
                }
            }
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
                        warn!("Cliente WebSocket atrasado, se omitieron {} eventos", missed);
                        let resync = serde_json::json!({
                            "type": "resync",
                            "missed_events": missed
                        });
                        if let Ok(json) = serde_json::to_string(&resync) {
                            let _ = socket.send(Message::Text(json.into())).await;
                        }
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

    info!("Cliente desconectado del WebSocket /v1/events");
}

pub async fn player_eq(
    State(state): State<AppState>,
    Json(payload): Json<EqPayload>,
) -> Result<Json<ApiResponse<DspResponse>>, (StatusCode, Json<ApiResponse<DspResponse>>)> {
    let dsp_params = state.get_dsp_params(payload.guild_id);
    let mut params = match dsp_params.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    let mut message = String::new();

    if let Some(ref preset) = payload.preset {
        match apply_preset_to_params(preset, &mut params) {
            Ok(desc) => message = format!("Preset '{}' aplicado: {}", preset, desc),
            Err(e) => return Err((StatusCode::BAD_REQUEST, Json(ApiResponse::err(e)))),
        }
    }

    if let Some(ref bands) = payload.bands {
        params.set_bands(bands);
        if message.is_empty() {
            message = format!("Se actualizaron {} bandas de ecualización", bands.len());
        }
    }

    let status = params.get_status();
    let res = DspResponse {
        guild_id: payload.guild_id,
        status,
        message,
    };

    Ok(Json(ApiResponse::ok(res)))
}

pub async fn player_dsp(
    State(state): State<AppState>,
    Json(payload): Json<DspPayload>,
) -> Result<Json<ApiResponse<DspResponse>>, (StatusCode, Json<ApiResponse<DspResponse>>)> {
    let dsp_params = state.get_dsp_params(payload.guild_id);
    let mut params = match dsp_params.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    let mut message = String::new();

    if let Some(ref preset) = payload.preset {
        match apply_preset_to_params(preset, &mut params) {
            Ok(desc) => message = format!("Preset '{}' aplicado: {}", preset, desc),
            Err(e) => return Err((StatusCode::BAD_REQUEST, Json(ApiResponse::err(e)))),
        }
    }

    if let Some(spatial) = payload.spatial {
        params.set_spatial(spatial);
        if message.is_empty() {
            message = "Configuración espacial y 3D actualizada".to_string();
        }
    }

    if let Some(modulation) = payload.modulation {
        params.set_modulation(modulation);
        if message.is_empty() {
            message = "Configuración de modulación (8D/Tremolo/Vibrato) actualizada".to_string();
        }
    }

    if let Some(loudness) = payload.loudness {
        params.set_loudness(loudness);
        if message.is_empty() {
            message = "Normalización de volumen EBU R128 actualizada".to_string();
        }
    }

    if let Some(timescale) = payload.timescale {
        params.set_timescale(timescale);
        if message.is_empty() {
            message = "Ajuste de velocidad/pitch (Timescale) actualizado".to_string();
        }
    }

    if let Some(ref bands) = payload.bands {
        params.set_bands(bands);
    }

    let status = params.get_status();
    let res = DspResponse {
        guild_id: payload.guild_id,
        status,
        message,
    };

    Ok(Json(ApiResponse::ok(res)))
}

/// Evalúa el estado del reproductor del guild y, si restan 10-15s para el final de la pista actual
/// (o si la cola está vacía tras un salto), pre-resuelve y encola la siguiente pista de la cola.
pub async fn maybe_prefetch_next_track(state: &AppState, guild_id: u64) {
    // 1. Si el modo de repetición es 'track', la pista actual se repite indefinidamente
    if state.get_loop_mode(guild_id) == LoopMode::Track {
        return;
    }

    // 2. Si ya hay una operación de prefetch en curso para este guild, evitar llamadas concurrentes
    if state.is_prefetching(guild_id) {
        return;
    }

    // 3. Verificar si hay pistas pendientes en la lista de reproducción o si autoplay está activo
    let has_pending = state
        .pending_playlists
        .get(&guild_id)
        .map(|p| !p.is_empty())
        .unwrap_or(false);

    if !has_pending && !state.is_autoplay(guild_id) {
        return;
    }

    // 4. Obtener el reproductor activo del guild
    let driver_ref = match state.players.get(&guild_id) {
        Some(d) => d.clone(),
        None => return,
    };

    // 5. Analizar el estado de reproducción y cola de Songbird
    // Minimizar el tiempo de retención del lock de driver: se extrae current_track y q_len
    let (current_track, q_len) = {
        let driver = driver_ref.lock().await;
        (driver.queue().current(), driver.queue().len())
    };

    let (should_prefetch, reason) = if q_len == 0 {
        // Cola vacía (por ejemplo tras skip o final abrupto): prefetch y arranque inmediato
        (true, "cola vacía, iniciando siguiente pista de inmediato".to_string())
    } else if q_len == 1 {
        // Solo hay 1 pista en Songbird (la que suena) y 0 pistas siguientes encoladas
        if let Some(ref current) = current_track {
            let meta: Arc<TrackMetadata> = current.data();
            if let Ok(info) = current.get_info().await {
                if info.playing == songbird::tracks::PlayMode::Play {
                    let pos_secs = info.position.as_secs();
                    if let Some(dur_secs) = meta.duration_secs {
                        let remaining = dur_secs.saturating_sub(pos_secs);
                        // Ventana óptima de prefetch: 10 a 15 segundos antes de terminar
                        // (o si la pista completa dura <= 20 segundos)
                        if remaining <= 15 || dur_secs <= 20 {
                            (
                                true,
                                format!("pista '{}' próxima a finalizar (restan {}s)", meta.title, remaining),
                            )
                        } else {
                            (false, String::new())
                        }
                    } else if pos_secs >= 5 {
                        // Duración desconocida (stream continuo o falta de header):
                        // Aseguramos 1 pista de buffer tras 5s de reproducción estable
                        (
                            true,
                            format!("pista '{}' con duración no reportada (pos: {}s)", meta.title, pos_secs),
                        )
                    } else {
                        (false, String::new())
                    }
                } else {
                    (false, String::new())
                }
            } else {
                (false, String::new())
            }
        } else {
            (false, String::new())
        }
    } else {
        // q_len >= 2: Ya existe al menos una pista pre-cargada lista en memoria
        (false, String::new())
    };

    if !should_prefetch {
        return;
    }

    // 6. Si no hay pistas pendientes pero autoplay está activo, generar recomendaciones vía InnerTube
    if !has_pending && state.is_autoplay(guild_id) {
        let seed_opt = if let Some(ref curr) = current_track {
            let meta: Arc<TrackMetadata> = curr.data();
            Some(meta.source_url.clone())
        } else {
            state.get_last_track(guild_id).map(|t| t.source_url)
        };

        if let Some(seed_url) = seed_opt {
            let clean_id = SourceResolver::extract_video_id(&seed_url).unwrap_or_else(|| seed_url.clone());
            info!(guild = guild_id, seed = %clean_id, "Autoplay activo: consultando recomendaciones en InnerTube (radio WEB_REMIX)...");
            if let Ok(items) = state.resolver.related_innertube(&clean_id, 25).await {
                let mut fresh: Vec<_> = items
                    .into_iter()
                    .filter(|item| {
                        if let Some(vid) = SourceResolver::extract_video_id(&item.url) {
                            !state.is_track_seen(guild_id, &vid) && vid != clean_id
                        } else {
                            true
                        }
                    })
                    .collect();

                if fresh.is_empty() {
                    state.clear_seen_tracks(guild_id);
                    fresh = state
                        .resolver
                        .related_innertube(&clean_id, 25)
                        .await
                        .unwrap_or_default();
                }

                if !fresh.is_empty() {
                    info!(
                        guild = guild_id,
                        count = fresh.len(),
                        next = %fresh[0].title,
                        "Autoplay: recomendaciones obtenidas y añadidas a la cola pendiente"
                    );
                    let mut list = state.pending_playlists.entry(guild_id).or_insert_with(Vec::new);
                    list.extend(fresh);
                }
            }
        }
    }

    // 7. Activar flag de prefetch y extraer la siguiente pista de pending_playlists
    if !state.start_prefetch(guild_id) {
        return;
    }

    let next_item = match state.pending_playlists.get_mut(&guild_id) {
        Some(mut list) if !list.is_empty() => list.remove(0),
        _ => {
            state.end_prefetch(guild_id);
            return;
        }
    };

    let play_opts = state
        .guild_play_options
        .get(&guild_id)
        .map(|o| o.clone())
        .unwrap_or_default();

    let state_clone = state.clone();
    let driver_clone = driver_ref.clone();

    tokio::spawn(async move {
        info!(
            guild = guild_id,
            next_title = %next_item.title,
            trigger = %reason,
            "Prefetch anticipado activado: resolviendo y precalentando flujo de audio (gapless)..."
        );

        let resolve_res = state_clone
            .resolver
            .resolve_single(
                &next_item.url,
                play_opts.extractor_args.as_deref(),
                play_opts.extra_args.as_deref(),
                play_opts.cookies.as_deref(),
                play_opts.format.as_deref(),
            )
            .await;

        match resolve_res {
            Ok(track) => {
                let dsp_params = state_clone.get_dsp_params(guild_id);
                let target_vol = state_clone.get_volume(guild_id);
                let effective_vol = (target_vol * 0.95).clamp(0.0, 2.0);
                let track_meta = Arc::new(track.clone());

                let songbird_track = create_track_with_dsp(
                    state_clone.http_client.clone(),
                    track.stream_url.clone(),
                    track_meta.clone(),
                    dsp_params,
                )
                .await;

                let handle = {
                    let mut driver = driver_clone.lock().await;
                    let h = driver.enqueue(songbird_track).await;
                    let _ = h.set_volume(effective_vol);
                    h
                };

                attach_track_events(
                    guild_id,
                    track_meta,
                    &handle,
                    &state_clone.event_tx,
                    Some(state_clone.clone()),
                );

                if state_clone.get_loop_mode(guild_id) == LoopMode::Track {
                    let _ = handle.enable_loop();
                }

                info!(
                    guild = guild_id,
                    title = %track.title,
                    "Prefetch exitoso: decodificador y stream listos en cola Songbird (transición gapless)"
                );
            }
            Err(err) => {
                warn!(
                    guild = guild_id,
                    url = %next_item.url,
                    title = %next_item.title,
                    error = %err,
                    "Fallo al resolver pista durante prefetch anticipado; continuará con la siguiente de la cola"
                );
            }
        }

        state_clone.end_prefetch(guild_id);
    });
}

/// Verifica si la pista en reproducción actual contiene segmentos de SponsorBlock
/// y adelanta la posición automáticamente si el cabezal entra en una sección no musical.
async fn check_sponsorblock_skip(state: &AppState, guild_id: u64) {
    let driver_ref = match state.players.get(&guild_id) {
        Some(d) => d.clone(),
        None => return,
    };

    let (current_handle, meta) = {
        let driver = driver_ref.lock().await;
        if let Some(current) = driver.queue().current() {
            let m: Arc<TrackMetadata> = current.data();
            (Some(current), Some(m))
        } else {
            (None, None)
        }
    };

    if let (Some(handle), Some(meta)) = (current_handle, meta) {
        if meta.skip_segments.is_empty() {
            return;
        }

        if let Ok(info) = handle.get_info().await {
            if info.playing == songbird::tracks::PlayMode::Play {
                let pos_secs = info.position.as_secs_f64();
                for seg in &meta.skip_segments {
                    if pos_secs >= seg.start && pos_secs < (seg.end - 0.25) {
                        info!(
                            guild = guild_id,
                            title = %meta.title,
                            from = pos_secs,
                            to = seg.end,
                            "SponsorBlock: saltando segmento no musical de videoclip"
                        );
                        let target = std::time::Duration::from_secs_f64(seg.end);
                        let _ = handle.seek_async(target).await;
                        break;
                    }
                }
            }
        }
    }
}

/// Garbage Collector de reproductores inactivos:
/// Si un servidor no tiene canciones en reproducción ni pendientes en cola por más de
/// AXAUDIO_IDLE_TIMEOUT_SECS (por defecto 300s / 5min), desconecta el driver y libera sockets/memoria.
async fn check_idle_cleanup(state: &AppState, guild_id: u64) {
    let timeout_secs: u64 = std::env::var("AXAUDIO_IDLE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);

    if timeout_secs == 0 {
        return;
    }

    let driver_ref = match state.players.get(&guild_id) {
        Some(d) => d.clone(),
        None => return,
    };

    let is_empty = {
        let driver = driver_ref.lock().await;
        driver.queue().is_empty()
    };

    let has_pending = state
        .pending_playlists
        .get(&guild_id)
        .map(|p| !p.is_empty())
        .unwrap_or(false);

    if !is_empty || has_pending {
        state.touch_activity(guild_id);
    } else {
        let last = state.get_last_activity(guild_id);
        if last.elapsed() >= std::time::Duration::from_secs(timeout_secs) {
            info!(
                guild = guild_id,
                idle_secs = last.elapsed().as_secs(),
                timeout = timeout_secs,
                "Garbage Collector: reproductor inactivo por más del límite configurado; liberando driver y recursos UDP"
            );
            cleanup_guild_player(state, guild_id).await;
        }
    }
}

/// Tarea global de monitoreo en segundo plano:
/// 1. Prefetch anticipado (10-15s antes del final de pista para audio gapless)
/// 2. SponsorBlock skipping en tiempo real
/// 3. Garbage Collection de reproductores inactivos
pub async fn run_prefetch_monitor(state: AppState) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(1000));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;

        let guild_ids: Vec<u64> = state.players.iter().map(|entry| *entry.key()).collect();
        for guild_id in guild_ids {
            maybe_prefetch_next_track(&state, guild_id).await;
            check_sponsorblock_skip(&state, guild_id).await;
            check_idle_cleanup(&state, guild_id).await;
        }
    }
}

