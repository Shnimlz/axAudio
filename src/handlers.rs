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
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};
use tracing::{error, info, warn};

use crate::dsp::{apply_preset_to_params, inject_dsp, DspParams};
use crate::models::{
    ApiResponse, AutoplayPayload, ControlPayload, DspPayload, DspResetPayload, DspResponse, EqPayload, JoinPayload, LeavePayload,
    LiveRadioPayload, LoudnessPayload, LoopMode, ModulationPayload, PlayPayload, PlayResponse, PlayerEvent, PlayerKey, PresetInfo, PresetListResponse, PresetSavePayload,
    QueuePayload, QueueResponse, ResolvePayload, ReverbPayload, SearchPayload, SearchResponse, SpatialPayload, StatsResponse,
    TimescalePayload, TrackResponse, TwentyFourSevenPayload,
};
use crate::sources::{FlatPlaylistItem, SourceResolver, TrackMetadata};
use crate::state::AppState;

struct TrackPlayNotifier {
    key: PlayerKey,
    meta: Arc<TrackMetadata>,
    event_tx: broadcast::Sender<PlayerEvent>,
    state: Option<AppState>,
    has_played: Arc<AtomicBool>,
}

#[async_trait]
impl EventHandler for TrackPlayNotifier {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        self.has_played.store(true, Ordering::SeqCst);
        info!(guild = self.key.guild_id, bot = self.key.bot_id, title = %self.meta.title, "Evento TrackPlay emitido");
        if let Some(ref st) = self.state {
            st.tracks_played_count.fetch_add(1, Ordering::Relaxed);
            st.set_last_track(self.key, (*self.meta).clone());
            if let Some(vid) = SourceResolver::extract_video_id(&self.meta.source_url) {
                st.record_seen_track(self.key, &vid);
            }
        }
        let _ = self.event_tx.send(PlayerEvent::TrackStart {
            guild_id: self.key.guild_id,
            bot_id: Some(self.key.bot_id),
            track: self.meta.to_response(),
        });
        None
    }
}

struct TrackEndNotifier {
    key: PlayerKey,
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
                guild = self.key.guild_id,
                bot = self.key.bot_id,
                title = %self.meta.title,
                "TrackEnd ignorado: la pista fue detenida/desalojada antes de sonar"
            );
            return None;
        }
        let (reason, played_secs) = match ctx {
            EventContext::Track(slice) => slice
                .first()
                .map(|(state, _)| {
                    let r = match state.playing {
                        songbird::tracks::PlayMode::Stop => "stopped",
                        songbird::tracks::PlayMode::End => "finished",
                        songbird::tracks::PlayMode::Errored(_) => "error",
                        _ => "finished",
                    };
                    (r, state.position.as_secs_f64())
                })
                .unwrap_or(("finished", 0.0)),
            _ => ("finished", 0.0),
        };

        // Detección de corte prematuro del stream por el CDN (menos de 4s reproducidos cuando la canción dura > 20s)
        let is_premature_eof = reason == "finished"
            && played_secs < 4.0
            && self.meta.duration_secs.map(|d| d > 20).unwrap_or(false)
            && !self.meta.is_live;

        if is_premature_eof {
            warn!(
                guild = self.key.guild_id,
                bot = self.key.bot_id,
                title = %self.meta.title,
                played_secs,
                expected_duration = ?self.meta.duration_secs,
                "TrackEnd anómalo: la pista finalizó tras solo {:.2}s (< 4s). Posible corte de CDN o stream truncado.",
                played_secs
            );

            if let Some(ref st) = self.state {
                let circuit_tripped = st.record_track_error(self.key);
                if circuit_tripped {
                    warn!(
                        guild = self.key.guild_id,
                        bot = self.key.bot_id,
                        "Circuit Breaker activado: 3 pistas consecutivas finalizaron prematuramente. Deteniendo reproducción para evitar bucle."
                    );
                    if let Some(driver_ref) = st.players.get(&self.key) {
                        let d = driver_ref.lock().await;
                        d.queue().stop();
                    }
                    st.pending_playlists.remove(&self.key);
                    let _ = self.event_tx.send(PlayerEvent::TrackError {
                        guild_id: self.key.guild_id,
                        bot_id: Some(self.key.bot_id),
                        track: Some(self.meta.to_response()),
                        error: "Reproducción detenida por protección anti-bucle: múltiples canciones fallaron o finalizaron prematuramente.".to_string(),
                    });
                    return None;
                }

                // Si aún no es intento de reintento, intentar auto-curación
                if !self.is_retry_attempt {
                    st.resolver.invalidate_cache(&self.meta.source_url);
                    st.resolver.invalidate_cache(&self.meta.stream_url);

                    let st_clone = st.clone();
                    let key = self.key;
                    let source_url = self.meta.source_url.clone();
                    let event_tx_clone = self.event_tx.clone();
                    let meta_clone = self.meta.clone();

                    tokio::spawn(async move {
                        let play_opts = st_clone
                            .guild_play_options
                            .get(&key)
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
                                if let Some(driver_ref) = st_clone.players.get(&key) {
                                    let dsp_params = st_clone.get_dsp_params(key);
                                    let target_vol = st_clone.get_volume(key);
                                    let effective_vol = (target_vol * 0.95).clamp(0.0, 2.0);
                                    let track_meta = Arc::new(fresh_track.clone());

                                    if let Ok(songbird_track) = create_track_with_dsp(
                                        st_clone.http_client.clone(),
                                        fresh_track.stream_url.clone(),
                                        track_meta.clone(),
                                        dsp_params,
                                        play_opts.user_agent.as_deref(),
                                    )
                                    .await
                                    {
                                        let handle = {
                                            let mut driver = driver_ref.lock().await;
                                            let h = driver.enqueue(songbird_track).await;
                                            let _ = h.set_volume(effective_vol);
                                            h
                                        };

                                        let retried_flag = Arc::new(AtomicBool::new(true));
                                        attach_track_events_with_retried(
                                            key,
                                            track_meta,
                                            &handle,
                                            &event_tx_clone,
                                            Some(st_clone.clone()),
                                            retried_flag,
                                        );

                                        info!(
                                            guild = key.guild_id,
                                            bot = key.bot_id,
                                            title = %fresh_track.title,
                                            "Auto-curación exitosa tras corte prematuro de CDN: pista reanudada"
                                        );
                                        return;
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(
                                    guild = key.guild_id,
                                    bot = key.bot_id,
                                    error = %e,
                                    "Auto-curación no pudo re-resolver la pista tras corte prematuro"
                                );
                            }
                        }

                        let _ = event_tx_clone.send(PlayerEvent::TrackError {
                            guild_id: key.guild_id,
                            bot_id: Some(key.bot_id),
                            track: Some(meta_clone.to_response()),
                            error: "El stream de audio se interrumpió de forma prematura (< 4s).".to_string(),
                        });
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        maybe_prefetch_next_track(&st_clone, key).await;
                    });

                    return None;
                }
            }
        }

        if reason == "error" && self.state.is_some() && !self.is_retry_attempt {
            return None;
        }

        info!(guild = self.key.guild_id, bot = self.key.bot_id, title = %self.meta.title, reason = %reason, played_secs, "Evento TrackEnd emitido");
        let _ = self.event_tx.send(PlayerEvent::TrackEnd {
            guild_id: self.key.guild_id,
            bot_id: Some(self.key.bot_id),
            track: Some(self.meta.to_response()),
            reason: reason.to_string(),
        });

        // Bucle de cola: Si la pista terminó con éxito y el modo es Queue, re-encolar al final
        if reason == "finished" && !is_premature_eof {
            if let Some(ref st) = self.state {
                // Éxito confirmado: limpiar contador de fallos consecutivos del Circuit Breaker
                st.reset_consecutive_errors(self.key);

                if st.get_loop_mode(self.key) == LoopMode::Queue {
                    let st_clone = st.clone();
                    let key = self.key;
                    let meta = self.meta.clone();
                    tokio::spawn(async move {
                        if let Some(driver_ref) = st_clone.players.get(&key) {
                            let dsp_params = st_clone.get_dsp_params(key);
                            let target_volume = st_clone.get_volume(key);
                            let effective_volume = (target_volume * 0.95).clamp(0.0, 2.0);
                            let play_opts = st_clone
                                .guild_play_options
                                .get(&key)
                                .map(|o| o.clone())
                                .unwrap_or_default();

                            // Si la URL del stream ha expirado o está próxima a expirar,
                            // re-resolverla con fresh URL antes de re-encolar
                            let meta_to_use = if meta.is_url_expired_or_expiring() {
                                st_clone.resolver.invalidate_cache(&meta.source_url);
                                st_clone.resolver.invalidate_cache(&meta.stream_url);
                                match st_clone.resolver.resolve_single(
                                    &meta.source_url,
                                    play_opts.extractor_args.as_deref(),
                                    play_opts.extra_args.as_deref(),
                                    play_opts.cookies.as_deref(),
                                    play_opts.format.as_deref(),
                                ).await {
                                    Ok(fresh) => Arc::new(fresh),
                                    Err(_) => meta.clone(),
                                }
                            } else {
                                meta.clone()
                            };

                            let trk = match create_track_with_dsp(
                                st_clone.http_client.clone(),
                                meta_to_use.stream_url.clone(),
                                meta_to_use.clone(),
                                dsp_params,
                                play_opts.user_agent.as_deref(),
                            )
                            .await {
                                Ok(t) => t,
                                Err(e) => {
                                    warn!(error = %e, "Fallo al inicializar stream para loop de cola");
                                    return;
                                }
                            };

                            let mut driver = driver_ref.lock().await;
                            let handle = driver.enqueue(trk).await;
                            let _ = handle.set_volume(effective_volume);
                            attach_track_events(
                                key,
                                meta_to_use,
                                &handle,
                                &st_clone.event_tx,
                                Some(st_clone.clone()),
                            );
                        }
                    });
                }
            }
        }

        // Activación de prefetch de seguridad
        if let Some(ref st) = self.state {
            let st_clone = st.clone();
            let key = self.key;
            tokio::spawn(async move {
                tokio::task::yield_now().await;
                maybe_prefetch_next_track(&st_clone, key).await;
            });
        }

        None
    }
}

struct TrackErrorNotifier {
    key: PlayerKey,
    meta: Arc<TrackMetadata>,
    event_tx: broadcast::Sender<PlayerEvent>,
    state: Option<AppState>,
    retried: Arc<AtomicBool>,
}

#[async_trait]
impl EventHandler for TrackErrorNotifier {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        let (err_msg, track_handle) = match ctx {
            EventContext::Track(slice) => {
                let msg = slice
                    .first()
                    .map(|(state, _)| format!("{:?}", state.playing))
                    .unwrap_or_else(|| "Error de reproducción".to_string());
                let h = slice.first().map(|(_, h)| (*h).clone());
                (msg, h)
            }
            _ => ("Error de reproducción".to_string(), None),
        };

        // Si la pista tenía repetición activa en su handle, desactivarla inmediatamente
        // para que Songbird no intente re-reproducir un stream roto en un bucle cerrado nativo.
        if let Some(ref h) = track_handle {
            let _ = h.disable_loop();
        }

        if let Some(ref st) = self.state {
            // Circuit Breaker: si 3 canciones consecutivas fallan en menos de 15 segundos,
            // detener la reproducción para evitar bucles infinitos y saturación de APIs.
            let circuit_tripped = st.record_track_error(self.key);
            if circuit_tripped {
                warn!(
                    guild = self.key.guild_id,
                    bot = self.key.bot_id,
                    title = %self.meta.title,
                    "Circuit Breaker activado: 3 canciones consecutivas fallaron al conectar con el stream. Deteniendo reproducción para evitar bucle."
                );
                if let Some(driver_ref) = st.players.get(&self.key) {
                    let d = driver_ref.lock().await;
                    d.queue().stop();
                }
                st.pending_playlists.remove(&self.key);
                // El Circuit Breaker se mantiene activo durante el tiempo de enfriamiento (30s)
                // para evitar que reintentos en segundo plano o autoplay vuelvan a generar bucles.

                let _ = self.event_tx.send(PlayerEvent::TrackError {
                    guild_id: self.key.guild_id,
                    bot_id: Some(self.key.bot_id),
                    track: Some(self.meta.to_response()),
                    error: "Reproducción detenida por protección anti-bucle: 3 canciones consecutivas fallaron al reproducirse.".to_string(),
                });
                return None;
            }

            if !self.retried.swap(true, Ordering::SeqCst) {
                st.resolver.invalidate_cache(&self.meta.source_url);
                st.resolver.invalidate_cache(&self.meta.stream_url);

                warn!(
                    guild = self.key.guild_id,
                    bot = self.key.bot_id,
                    title = %self.meta.title,
                    url = %self.meta.source_url,
                    error = %err_msg,
                    "TrackError detectado (posible 403 o corte de CDN). Purgando caché y reintentando automáticamente..."
                );

                let st_clone = st.clone();
                let key = self.key;
                let source_url = self.meta.source_url.clone();
                let event_tx_clone = self.event_tx.clone();

                tokio::spawn(async move {
                    let play_opts = st_clone
                        .guild_play_options
                        .get(&key)
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
                            if let Some(driver_ref) = st_clone.players.get(&key) {
                                let dsp_params = st_clone.get_dsp_params(key);
                                let target_vol = st_clone.get_volume(key);
                                let effective_vol = (target_vol * 0.95).clamp(0.0, 2.0);
                                let track_meta = Arc::new(fresh_track.clone());

                                match create_track_with_dsp(
                                    st_clone.http_client.clone(),
                                    fresh_track.stream_url.clone(),
                                    track_meta.clone(),
                                    dsp_params,
                                    play_opts.user_agent.as_deref(),
                                )
                                .await
                                {
                                    Ok(songbird_track) => {
                                        let handle = {
                                            let mut driver = driver_ref.lock().await;
                                            let h = driver.enqueue(songbird_track).await;
                                            let _ = h.set_volume(effective_vol);
                                            h
                                        };

                                        // El flag de reintento queda permanentemente en true para evitar reintentar en bucle
                                        let retried_flag = Arc::new(AtomicBool::new(true));
                                        attach_track_events_with_retried(
                                            key,
                                            track_meta,
                                            &handle,
                                            &event_tx_clone,
                                            Some(st_clone.clone()),
                                            retried_flag,
                                        );

                                        if st_clone.get_loop_mode(key) == LoopMode::Track {
                                            let _ = handle.enable_loop();
                                        }

                                        info!(
                                            guild = key.guild_id,
                                            bot = key.bot_id,
                                            title = %fresh_track.title,
                                            "Auto-curación exitosa: pista re-resuelta con stream fresco y reanudada"
                                        );
                                        return;
                                    }
                                    Err(e) => {
                                        warn!(
                                            guild = key.guild_id,
                                            bot = key.bot_id,
                                            title = %fresh_track.title,
                                            error = %e,
                                            "Auto-curación descartada: el stream re-resuelto tampoco pudo abrirse"
                                        );
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!(
                                guild = key.guild_id,
                                bot = key.bot_id,
                                error = %e,
                                "Reintento de auto-curación falló al re-resolver la pista"
                            );
                        }
                    }

                    let _ = event_tx_clone.send(PlayerEvent::TrackError {
                        guild_id: key.guild_id,
                        bot_id: Some(key.bot_id),
                        track: Some(TrackResponse {
                            title: source_url.clone(),
                            source_url,
                            stream_url: String::new(),
                            duration_secs: None,
                            author: None,
                            thumbnail: None,
                            is_live: None,
                            suggested_preset: None,
                        }),
                        error: err_msg,
                    });
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    maybe_prefetch_next_track(&st_clone, key).await;
                });

                return None;
            }
        }

        warn!(
            guild = self.key.guild_id,
            bot = self.key.bot_id,
            title = %self.meta.title,
            error = %err_msg,
            "Evento TrackError emitido (sin más reintentos)"
        );
        let _ = self.event_tx.send(PlayerEvent::TrackError {
            guild_id: self.key.guild_id,
            bot_id: Some(self.key.bot_id),
            track: Some(self.meta.to_response()),
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
    key: PlayerKey,
    meta: Arc<TrackMetadata>,
    handle: &TrackHandle,
    event_tx: &broadcast::Sender<PlayerEvent>,
    state: Option<AppState>,
) {
    let retried = Arc::new(AtomicBool::new(false));
    attach_track_events_with_retried(key, meta, handle, event_tx, state, retried);
}

fn attach_track_events_with_retried(
    key: PlayerKey,
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
            key,
            meta: meta.clone(),
            event_tx: event_tx.clone(),
            state: state.clone(),
            has_played: has_played.clone(),
        },
    );
    let _ = handle.add_event(
        Event::Track(TrackEvent::End),
        TrackEndNotifier {
            key,
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
            key,
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
        active_247_players: state.twenty_four_seven_players.len(),
        tracks_played_total: state.tracks_played_count.load(Ordering::Relaxed),
        resolves_total: state.resolves_count.load(Ordering::Relaxed),
        cached_tracks: state.resolver.cache_size(),
        custom_presets_count: state.custom_presets.len(),
        memory_rss_bytes: AppState::get_memory_rss(),
    };
    Json(ApiResponse::ok(stats))
}

pub async fn get_metrics(State(state): State<AppState>) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    let uptime = state.start_time.elapsed().as_secs();
    let active_players = state.players.len();
    let active_247 = state.twenty_four_seven_players.len();
    let tracks_played = state.tracks_played_count.load(Ordering::Relaxed);
    let resolves = state.resolves_count.load(Ordering::Relaxed);
    let cached = state.resolver.cache_size();
    let custom_presets = state.custom_presets.len();
    let memory_rss = AppState::get_memory_rss();

    let mut body = String::with_capacity(1024);
    body.push_str("# HELP axaudio_uptime_seconds Uptime of the axAudio engine in seconds\n");
    body.push_str("# TYPE axaudio_uptime_seconds counter\n");
    body.push_str(&format!("axaudio_uptime_seconds {}\n\n", uptime));

    body.push_str("# HELP axaudio_active_players Number of currently active Discord voice players\n");
    body.push_str("# TYPE axaudio_active_players gauge\n");
    body.push_str(&format!("axaudio_active_players {}\n\n", active_players));

    body.push_str("# HELP axaudio_247_players Number of players with 24/7 mode enabled\n");
    body.push_str("# TYPE axaudio_247_players gauge\n");
    body.push_str(&format!("axaudio_247_players {}\n\n", active_247));

    body.push_str("# HELP axaudio_tracks_played_total Total number of audio tracks played since startup\n");
    body.push_str("# TYPE axaudio_tracks_played_total counter\n");
    body.push_str(&format!("axaudio_tracks_played_total {}\n\n", tracks_played));

    body.push_str("# HELP axaudio_resolves_total Total number of track resolution queries executed\n");
    body.push_str("# TYPE axaudio_resolves_total counter\n");
    body.push_str(&format!("axaudio_resolves_total {}\n\n", resolves));

    body.push_str("# HELP axaudio_cached_tracks Number of tracks currently in memory cache\n");
    body.push_str("# TYPE axaudio_cached_tracks gauge\n");
    body.push_str(&format!("axaudio_cached_tracks {}\n\n", cached));

    body.push_str("# HELP axaudio_custom_presets Number of custom user-defined presets registered\n");
    body.push_str("# TYPE axaudio_custom_presets gauge\n");
    body.push_str(&format!("axaudio_custom_presets {}\n\n", custom_presets));

    body.push_str("# HELP axaudio_memory_rss_bytes Process resident set size (RSS) memory in bytes\n");
    body.push_str("# TYPE axaudio_memory_rss_bytes gauge\n");
    body.push_str(&format!("axaudio_memory_rss_bytes {}\n", memory_rss));

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

pub async fn resolve_track(
    State(state): State<AppState>,
    Json(payload): Json<ResolvePayload>,
) -> Result<Json<ApiResponse<TrackResponse>>, (StatusCode, Json<ApiResponse<TrackResponse>>)> {
    // Rate limit: verificar antes de ejecutar yt-dlp
    if let Err(e) = state.rate_limiter.check_resolve(payload.guild_id) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiResponse::err(format!("Rate limit excedido: {}", e))),
        ));
    }

    state.resolves_count.fetch_add(1, Ordering::Relaxed);

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
            let res = meta.to_response();
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
    // Rate limit: verificar antes de ejecutar yt-dlp
    if let Err(e) = state.rate_limiter.check_resolve(payload.guild_id) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiResponse::err(format!("Rate limit excedido: {}", e))),
        ));
    }

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

    // Rate limit: verificar antes de abrir sockets UDP
    if let Err(e) = state.rate_limiter.check_join(payload.guild_id) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiResponse::err(format!("Rate limit excedido: {}", e))),
        ));
    }

    let channel_id = match NonZeroU64::new(payload.channel_id) {
        Some(id) => id,
        None => return Err((StatusCode::BAD_REQUEST, Json(ApiResponse::err("channel_id inválido: 0")))),
    };

    let user_id = match NonZeroU64::new(payload.user_id) {
        Some(id) => id,
        None => return Err((StatusCode::BAD_REQUEST, Json(ApiResponse::err("user_id inválido: 0")))),
    };

    let bot_id = payload.bot_id.unwrap_or(payload.user_id);
    let player_key = PlayerKey::new(payload.guild_id, bot_id);

    info!(
        guild = payload.guild_id,
        bot = bot_id,
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
        .entry(player_key)
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
            state.touch_activity(player_key);
    state.resolves_count.fetch_add(1, Ordering::Relaxed);
            info!(guild = payload.guild_id, bot = bot_id, "Conexión exitosa al canal de voz");
            Ok(Json(ApiResponse::ok("Conectado al canal de voz de Discord")))
        }
        Err(e) => {
            error!(guild = payload.guild_id, bot = bot_id, error = ?e, "Fallo al conectar el driver al canal de voz");
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
    custom_user_agent: Option<&str>,
) -> Result<Track, String> {
    let is_google_cdn = stream_url.contains("googlevideo.com")
        || stream_url.contains("googleapis.com/videoplayback");

    // Prioridad de User-Agent: la devuelta por yt-dlp (meta.stream_headers) > custom
    // del payload > default del AppState. La primera es la con la que se firmó la
    // URL y es la única que googlevideo.com acepta.
    let meta_ua: Option<String> = meta
        .stream_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("User-Agent"))
        .map(|(_, v)| v.clone());

    let client = if is_google_cdn {
        let ua = meta_ua
            .or_else(|| custom_user_agent.map(|s| s.to_string()))
            .unwrap_or_else(|| {
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36".to_string()
            });
        reqwest::Client::builder()
            .user_agent(ua)
            .connect_timeout(Duration::from_secs(4))
            .build()
            .unwrap_or_else(|_| http_client.clone())
    } else if let Some(ua) = custom_user_agent {
        reqwest::Client::builder()
            .user_agent(ua)
            .connect_timeout(Duration::from_secs(4))
            .build()
            .unwrap_or_else(|_| http_client.clone())
    } else {
        http_client.clone()
    };

    // Inyectar los headers que yt-dlp devolvió (`http_headers`). Es crítico para
    // streams firmados de YouTube (googlevideo.com / videoplayback): la URL está
    // firmada contra el `User-Agent` (y a veces `Client-Context`) que el extractor
    // usó para obtenerla. Enviar una UA genérica (la de state.http_client) provoca
    // 403 Forbidden al reproducir, reintentar o hacer Range requests sobre el stream.
    let mut headers: reqwest::header::HeaderMap = reqwest::header::HeaderMap::new();
    for (k, v) in &meta.stream_headers {
        if k.eq_ignore_ascii_case("Content-Length") {
            // content_length se maneja vía HttpRequest::content_length para que
            // songbird construya `Range: bytes=0-{max}` correctamente.
            continue;
        }
        if let (Ok(name), Ok(val)) = (
            k.parse::<reqwest::header::HeaderName>(),
            v.parse::<reqwest::header::HeaderValue>(),
        ) {
            headers.insert(name, val);
        }
    }

    let effective_content_length = meta
        .stream_content_length
        .or_else(|| SourceResolver::extract_clen_from_url(&stream_url));

    let mut source = HttpRequest::new_with_headers(client.clone(), stream_url.clone(), headers.clone());
    if let Some(len) = effective_content_length {
        source.content_length = Some(len);
    }
    let input: songbird::input::Input = source.into();

    let make_playable_fut = input.make_playable_async(
        songbird::input::codecs::get_codec_registry(),
        songbird::input::codecs::get_probe(),
    );

    match tokio::time::timeout(Duration::from_secs(5), make_playable_fut).await {
        Ok(Ok(mut playable)) => {
            if let Some(parsed) = playable.parsed_mut() {
                inject_dsp(parsed, dsp_params);
            }
            Ok(Track::new_with_data(playable, meta))
        }
        Ok(Err(e)) => {
            let err_str = e.to_string();
            if err_str.contains("403")
                || err_str.contains("Forbidden")
                || err_str.contains("404")
                || err_str.contains("410")
            {
                return Err(format!("Error HTTP ({}) al abrir stream", err_str));
            }
            warn!(error = ?e, "Pre-parsing falló para DSP, intentando entrada nativa de Songbird con headers");
            let mut fallback = HttpRequest::new_with_headers(client, stream_url, headers);
            if let Some(len) = effective_content_length {
                fallback.content_length = Some(len);
            }
            Ok(Track::new_with_data(fallback.into(), meta))
        }
        Err(_) => {
            warn!("Pre-parsing excedió límite de 5s, usando entrada nativa en streaming de Songbird");
            let mut fallback = HttpRequest::new_with_headers(client, stream_url, headers);
            if let Some(len) = effective_content_length {
                fallback.content_length = Some(len);
            }
            Ok(Track::new_with_data(fallback.into(), meta))
        }
    }
}

pub async fn player_play(
    State(state): State<AppState>,
    Json(payload): Json<PlayPayload>,
) -> Result<Json<ApiResponse<PlayResponse>>, (StatusCode, Json<ApiResponse<PlayResponse>>)> {
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let driver_ref = match state.players.get(&player_key) {
        Some(d) => d.clone(),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiResponse::err("No hay conexión de voz activa para este servidor/bot. Ejecute /v1/voice/join primero.")),
            ));
        }
    };

    state.touch_activity(player_key);
    state.resolves_count.fetch_add(1, Ordering::Relaxed);

    info!(guild = payload.guild_id, bot = player_key.bot_id, query = %payload.url, "Resolviendo fuente de audio (Pista/Playlist/Mix/Radio)...");

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
            error!(guild = payload.guild_id, bot = player_key.bot_id, error = %e, "Fallo al resolver lote de audio con yt-dlp");
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::err(format!("Fallo resolviendo pista(s): {}", e))),
            ));
        }
    };

    // Si se solicita reemplazar explícitamente, se detiene la cola y el stream actual
    if payload.replace == Some(true) {
        state.next_epoch(player_key);
        state.pending_playlists.remove(&player_key);
        let mut d = driver_ref.lock().await;
        d.stop();
        d.queue().stop();
    }

    let target_volume = payload.volume.unwrap_or_else(|| state.get_volume(player_key));
    state.set_volume(player_key, target_volume);
    let effective_volume = (target_volume * 0.95).clamp(0.0, 2.0);

    let dsp_params = state.get_dsp_params(player_key);

    // Encolar inmediatamente la pista 1 con el pipeline DSP inyectado
    // IMPORTANTE: create_track_with_dsp conecta vía HTTP para resolver headers del stream.
    // Lo ejecutamos FUERA del lock del Driver para no bloquear /queue, /control ni /leave.
    let first_meta = Arc::new(batch.first_track.clone());
    let songbird_track = match create_track_with_dsp(
        state.http_client.clone(),
        batch.first_track.stream_url.clone(),
        first_meta.clone(),
        dsp_params.clone(),
        payload.user_agent.as_deref(),
    )
    .await {
        Ok(t) => t,
        Err(e) => {
            error!(guild = payload.guild_id, bot = player_key.bot_id, error = %e, "Fallo al inicializar stream de audio");
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse::err(format!("Fallo al conectar con el stream de audio: {}", e))),
            ));
        }
    };

    // Petición explícita de usuario: resetear contador de fallos del Circuit Breaker
    state.reset_consecutive_errors(player_key);

    let (handle, initial_queue_len, total_queue_len) = {
        let mut driver = driver_ref.lock().await;
        let init_len = driver.queue().len();
        let h = driver.enqueue(songbird_track).await;
        let _ = h.set_volume(effective_volume);
        let tot_len = driver.queue().len();
        (h, init_len, tot_len)
    };
    attach_track_events(
        player_key,
        first_meta,
        &handle,
        &state.event_tx,
        Some(state.clone()),
    );
    if state.get_loop_mode(player_key) == LoopMode::Track {
        let _ = handle.enable_loop();
    }

    // Si es una playlist/mix con pistas pendientes, registrar en cola pendiente y opciones de reproducción.
    // El motor global de prefetch anticipado preparará cada pista 10-15s antes del final de la pista activa,
    // garantizando streams HTTP frescos, cero saturación de sockets y transiciones 100% gapless.
    if !batch.remaining_entries.is_empty() {
        state.next_epoch(player_key);
        state.pending_playlists.insert(player_key, batch.remaining_entries.clone());
        state.guild_play_options.insert(
            player_key,
            crate::state::GuildPlayOptions {
                extractor_args: payload.extractor_args.clone(),
                extra_args: payload.extra_args.clone(),
                cookies: payload.cookies.clone(),
                format: payload.format.clone(),
                user_agent: payload.user_agent.clone(),
            },
        );

        info!(
            guild = player_key.guild_id,
            bot = player_key.bot_id,
            pending_count = batch.remaining_entries.len(),
            "Playlist registrada en cola pendiente: el motor de prefetch anticipado resolverá cada pista 10-15s antes de su corte"
        );

        // Si la pista inicial es corta (<= 20s), evaluar prefetch inmediatamente
        let state_trigger = state.clone();
        tokio::spawn(async move {
            maybe_prefetch_next_track(&state_trigger, player_key).await;
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
        bot = player_key.bot_id,
        is_playlist = batch.is_playlist,
        playlist_tracks_count = batch.total_tracks_count,
        "Pista inicial lista y encolada exitosamente"
    );

    Ok(Json(ApiResponse::ok(response)))
}

/// Endpoint dedicado para streaming de radio en vivo (Icecast / SHOUTcast / HLS / HTTP stream)
/// con demuxer de metadatos ICY y emisión de eventos WebSocket en tiempo real.
pub async fn player_live(
    State(state): State<AppState>,
    Json(payload): Json<LiveRadioPayload>,
) -> Result<Json<ApiResponse<PlayResponse>>, (StatusCode, Json<ApiResponse<PlayResponse>>)> {
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let driver_ref = match state.players.get(&player_key) {
        Some(d) => d.clone(),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiResponse::err("No hay conexión de voz activa para este servidor/bot. Ejecute /v1/voice/join primero.")),
            ));
        }
    };

    let target_volume = payload.volume.unwrap_or_else(|| state.get_volume(player_key));
    state.set_volume(player_key, target_volume);
    let effective_volume = (target_volume * 0.95).clamp(0.0, 2.0);

    let dsp_params = state.get_dsp_params(player_key);

    let (songbird_track, track_meta) = match crate::icy::create_live_radio_track(
        state.http_client.clone(),
        payload.url.clone(),
        payload.title.clone(),
        player_key.guild_id,
        Some(player_key.bot_id),
        dsp_params,
        state.event_tx.clone(),
        payload.user_agent.as_deref(),
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            warn!(url = %payload.url, error = %e, "Error al conectar a radio streaming en vivo");
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse::err(format!("No se pudo conectar a la radio en vivo: {}", e))),
            ));
        }
    };

    // Limpiar cola pendiente de playlists anteriores si existían
    state.pending_playlists.remove(&player_key);

    let handle = {
        let mut driver = driver_ref.lock().await;
        // Para radio en vivo, reemplazamos cualquier pista actual
        driver.queue().stop();
        let h = driver.play(songbird_track);
        let _ = h.set_volume(effective_volume);
        h
    };

    attach_track_events(
        player_key,
        track_meta.clone(),
        &handle,
        &state.event_tx,
        Some(state.clone()),
    );

    state.touch_activity(player_key);

    let response = PlayResponse {
        title: track_meta.title.clone(),
        source_url: track_meta.source_url.clone(),
        stream_url: track_meta.stream_url.clone(),
        duration_secs: None,
        author: track_meta.author.clone(),
        thumbnail: None,
        is_playlist: false,
        playlist_title: None,
        playlist_tracks_count: 0,
        queue_length: 1,
        was_empty: true,
    };

    info!(
        guild = payload.guild_id,
        bot = player_key.bot_id,
        title = %track_meta.title,
        url = %payload.url,
        "Radio streaming en vivo iniciada con éxito"
    );

    Ok(Json(ApiResponse::ok(response)))
}

pub async fn player_queue(
    State(state): State<AppState>,
    Json(payload): Json<QueuePayload>,
) -> Result<Json<ApiResponse<QueueResponse>>, (StatusCode, Json<ApiResponse<QueueResponse>>)> {
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let driver_ref = match state.players.get(&player_key) {
        Some(d) => d.clone(),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiResponse::err("No hay reproductor de voz activo para este servidor/bot")),
            ));
        }
    };

    let driver = driver_ref.lock().await;
    let handles = driver.queue().current_queue();
    let is_empty = driver.queue().is_empty();

    let mut all_tracks = Vec::new();
    for handle in handles {
        let meta: Arc<TrackMetadata> = handle.data();
        all_tracks.push(meta.to_response());
    }

    // Agregar pistas de playlist que aún se están cargando en segundo plano
    if let Some(pending) = state.pending_playlists.get(&player_key) {
        for item in pending.iter() {
            all_tracks.push(TrackResponse {
                title: format!("[Pendiente] {}", item.title),
                source_url: item.url.clone(),
                stream_url: String::new(),
                duration_secs: item.duration_secs,
                author: item.author.clone(),
                thumbnail: item.thumbnail.clone(),
                is_live: None,
                suggested_preset: None,
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
    let loop_mode = state.get_loop_mode(player_key).as_str().to_string();
    let autoplay = state.is_autoplay(player_key);
    let twenty_four_seven = state.is_247(player_key);

    let res = QueueResponse {
        guild_id: payload.guild_id,
        bot_id: Some(player_key.bot_id),
        is_playing: current.is_some(),
        current,
        current_position_secs,
        loop_mode,
        autoplay,
        twenty_four_seven,
        queue: all_tracks,
        total_tracks: total,
    };

    Ok(Json(ApiResponse::ok(res)))
}

pub async fn player_control(
    State(state): State<AppState>,
    Json(payload): Json<ControlPayload>,
) -> Result<Json<ApiResponse<String>>, (StatusCode, Json<ApiResponse<String>>)> {
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let driver_ref = match state.players.get(&player_key) {
        Some(d) => d.clone(),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiResponse::err("No hay reproductor de voz activo para este servidor/bot")),
            ));
        }
    };

    state.touch_activity(player_key);
    state.resolves_count.fetch_add(1, Ordering::Relaxed);

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
            state.next_epoch(player_key);
            state.pending_playlists.remove(&player_key);
            state.end_prefetch(player_key);
            state.reset_consecutive_errors(player_key);
            driver.queue().stop();
            driver.stop();
            Ok(Json(ApiResponse::ok("Reproductor detenido".to_string())))
        }
        "skip" => {
            let _ = driver.queue().skip();
            Ok(Json(ApiResponse::ok("Pista saltada".to_string())))
        }
        "clear" => {
            state.next_epoch(player_key);
            state.pending_playlists.remove(&player_key);
            state.end_prefetch(player_key);
            state.reset_consecutive_errors(player_key);
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
                state.set_volume(player_key, clamped);
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

            state.set_loop_mode(player_key, mode);

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

            if let Some(mut pend) = state.pending_playlists.get_mut(&player_key) {
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
                    if let Some(mut pend) = state.pending_playlists.get_mut(&player_key) {
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
                .get(&player_key)
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
                        .get(&player_key)
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
                if let Some(mut pend) = state.pending_playlists.get_mut(&player_key) {
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
                    let mut pend = state.pending_playlists.entry(player_key).or_default();
                    let to_p = (to - new_q_upcoming_len).min(pend.len());
                    pend.insert(to_p, item);
                    trigger_prefetch = true;
                }
            } else {
                // Caso 4: Mover desde la cola pendiente hacia la cola interna activa (to < q_upcoming_len)
                let from_p = from - q_upcoming_len;
                let item_opt = state
                    .pending_playlists
                    .get_mut(&player_key)
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

                    let mut pend = state.pending_playlists.entry(player_key).or_default();
                    pend.insert(0, item);
                    for (i, d) in displaced.into_iter().enumerate() {
                        pend.insert(1 + i, d);
                    }
                    state.end_prefetch(player_key);
                    trigger_prefetch = true;
                }
            }

            if trigger_prefetch {
                let st_clone = state.clone();
                tokio::spawn(async move {
                    maybe_prefetch_next_track(&st_clone, player_key).await;
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
                state.set_autoplay(player_key, e);
                e
            } else {
                let toggled = !state.is_autoplay(player_key);
                state.set_autoplay(player_key, toggled);
                toggled
            };

            if new_val {
                let st_clone = state.clone();
                tokio::spawn(async move {
                    maybe_prefetch_next_track(&st_clone, player_key).await;
                });
            }

            Ok(Json(ApiResponse::ok(format!(
                "Autoplay {}",
                if new_val { "activado" } else { "desactivado" }
            ))))
        }
        "247" | "24_7" | "twenty_four_seven" => {
            let new_val = if let Some(e) = payload.enabled {
                state.set_247(player_key, e);
                e
            } else {
                let toggled = !state.is_247(player_key);
                state.set_247(player_key, toggled);
                toggled
            };

            Ok(Json(ApiResponse::ok(format!(
                "Modo 24/7 {}",
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
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let new_val = if let Some(e) = payload.enabled {
        state.set_autoplay(player_key, e);
        e
    } else {
        let toggled = !state.is_autoplay(player_key);
        state.set_autoplay(player_key, toggled);
        toggled
    };

    if new_val {
        let st_clone = state.clone();
        tokio::spawn(async move {
            maybe_prefetch_next_track(&st_clone, player_key).await;
        });
    }

    Ok(Json(ApiResponse::ok(new_val)))
}

pub async fn player_247(
    State(state): State<AppState>,
    Json(payload): Json<TwentyFourSevenPayload>,
) -> Result<Json<ApiResponse<String>>, (StatusCode, Json<ApiResponse<String>>)> {
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let new_val = if let Some(e) = payload.enabled {
        state.set_247(player_key, e);
        e
    } else {
        let toggled = !state.is_247(player_key);
        state.set_247(player_key, toggled);
        toggled
    };

    Ok(Json(ApiResponse::ok(format!(
        "Modo 24/7 {}",
        if new_val { "activado" } else { "desactivado" }
    ))))
}

/// Desconecta el driver de Songbird y purga todos los registros del servidor en AppState
pub async fn cleanup_guild_player(state: &AppState, key: PlayerKey) -> bool {
    state.next_epoch(key);
    state.pending_playlists.remove(&key);
    state.end_prefetch(key);
    state.guild_play_options.remove(&key);
    state.dsp_params.remove(&key);
    state.load_epochs.remove(&key);
    state.guild_volumes.remove(&key);
    state.loop_modes.remove(&key);
    state.autoplay_guilds.remove(&key);
    state.twenty_four_seven_players.remove(&key);
    state.seen_tracks.remove(&key);
    state.last_tracks.remove(&key);
    state.last_activity.remove(&key);

    if let Some((_, driver_ref)) = state.players.remove(&key) {
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
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    if cleanup_guild_player(&state, player_key).await {
        info!(guild = payload.guild_id, bot = player_key.bot_id, "Driver desconectado y reproductor eliminado");
        Ok(Json(ApiResponse::ok("Desconectado y reproductor limpiado")))
    } else {
        warn!(guild = payload.guild_id, bot = player_key.bot_id, "Intento de abandonar una sesión de voz inexistente");
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
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let dsp_params = state.get_dsp_params(player_key);
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
    drop(params);
    state.persist_guild_settings();

    let res = DspResponse {
        guild_id: payload.guild_id,
        bot_id: Some(player_key.bot_id),
        status,
        message,
    };

    Ok(Json(ApiResponse::ok(res)))
}

pub async fn player_dsp(
    State(state): State<AppState>,
    Json(payload): Json<DspPayload>,
) -> Result<Json<ApiResponse<DspResponse>>, (StatusCode, Json<ApiResponse<DspResponse>>)> {
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let dsp_params = state.get_dsp_params(player_key);
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

    if let Some(reverb) = payload.reverb {
        params.set_reverb(reverb);
        if message.is_empty() {
            message = "Configuración de reverb acústico FDN actualizada".to_string();
        }
    }

    if let Some(ref bands) = payload.bands {
        params.set_bands(bands);
    }

    let status = params.get_status();
    drop(params);
    state.persist_guild_settings();

    let res = DspResponse {
        guild_id: payload.guild_id,
        bot_id: Some(player_key.bot_id),
        status,
        message,
    };

    Ok(Json(ApiResponse::ok(res)))
}

pub async fn player_dsp_reverb(
    State(state): State<AppState>,
    Json(payload): Json<ReverbPayload>,
) -> Result<Json<ApiResponse<DspResponse>>, (StatusCode, Json<ApiResponse<DspResponse>>)> {
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let dsp_params = state.get_dsp_params(player_key);
    let mut params = match dsp_params.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    params.set_reverb(payload.settings);
    let status = params.get_status();
    drop(params);
    state.persist_guild_settings();

    Ok(Json(ApiResponse::ok(DspResponse {
        guild_id: payload.guild_id,
        bot_id: Some(player_key.bot_id),
        status,
        message: "Configuración de Reverb FDN actualizada".to_string(),
    })))
}

pub async fn player_dsp_spatial(
    State(state): State<AppState>,
    Json(payload): Json<SpatialPayload>,
) -> Result<Json<ApiResponse<DspResponse>>, (StatusCode, Json<ApiResponse<DspResponse>>)> {
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let dsp_params = state.get_dsp_params(player_key);
    let mut params = match dsp_params.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    params.set_spatial(payload.settings);
    let status = params.get_status();
    drop(params);
    state.persist_guild_settings();

    Ok(Json(ApiResponse::ok(DspResponse {
        guild_id: payload.guild_id,
        bot_id: Some(player_key.bot_id),
        status,
        message: "Configuración Espacial 3D actualizada".to_string(),
    })))
}

pub async fn player_dsp_timescale(
    State(state): State<AppState>,
    Json(payload): Json<TimescalePayload>,
) -> Result<Json<ApiResponse<DspResponse>>, (StatusCode, Json<ApiResponse<DspResponse>>)> {
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let dsp_params = state.get_dsp_params(player_key);
    let mut params = match dsp_params.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    params.set_timescale(payload.settings);
    let status = params.get_status();
    drop(params);
    state.persist_guild_settings();

    Ok(Json(ApiResponse::ok(DspResponse {
        guild_id: payload.guild_id,
        bot_id: Some(player_key.bot_id),
        status,
        message: "Ajuste de Timescale (velocidad/pitch) actualizado".to_string(),
    })))
}

pub async fn player_dsp_loudness(
    State(state): State<AppState>,
    Json(payload): Json<LoudnessPayload>,
) -> Result<Json<ApiResponse<DspResponse>>, (StatusCode, Json<ApiResponse<DspResponse>>)> {
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let dsp_params = state.get_dsp_params(player_key);
    let mut params = match dsp_params.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    params.set_loudness(payload.settings);
    let status = params.get_status();
    drop(params);
    state.persist_guild_settings();

    Ok(Json(ApiResponse::ok(DspResponse {
        guild_id: payload.guild_id,
        bot_id: Some(player_key.bot_id),
        status,
        message: "Normalización de volumen EBU R128 actualizada".to_string(),
    })))
}

pub async fn player_dsp_modulation(
    State(state): State<AppState>,
    Json(payload): Json<ModulationPayload>,
) -> Result<Json<ApiResponse<DspResponse>>, (StatusCode, Json<ApiResponse<DspResponse>>)> {
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let dsp_params = state.get_dsp_params(player_key);
    let mut params = match dsp_params.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    params.set_modulation(payload.settings);
    let status = params.get_status();
    drop(params);
    state.persist_guild_settings();

    Ok(Json(ApiResponse::ok(DspResponse {
        guild_id: payload.guild_id,
        bot_id: Some(player_key.bot_id),
        status,
        message: "Efectos de modulación (8D/Chorus/Flanger/Phaser) actualizados".to_string(),
    })))
}

pub async fn player_dsp_reset(
    State(state): State<AppState>,
    Json(payload): Json<DspResetPayload>,
) -> Result<Json<ApiResponse<DspResponse>>, (StatusCode, Json<ApiResponse<DspResponse>>)> {
    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let dsp_params = state.get_dsp_params(player_key);
    let mut params = match dsp_params.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *params = DspParams::default();
    let status = params.get_status();
    drop(params);
    state.persist_guild_settings();

    Ok(Json(ApiResponse::ok(DspResponse {
        guild_id: payload.guild_id,
        bot_id: Some(player_key.bot_id),
        status,
        message: "Pipeline DSP restablecido a plano (bypass)".to_string(),
    })))
}

/// Evalúa el estado del reproductor y, si restan 10-15s para el final de la pista actual
/// (o si la cola está vacía tras un salto), pre-resuelve y encola la siguiente pista de la cola.
pub async fn maybe_prefetch_next_track(state: &AppState, key: PlayerKey) {
    // 0. Si el Circuit Breaker está disparado (3 errores continuos), abortar para evitar bucle
    if state.is_circuit_tripped(key) {
        return;
    }

    // 1. Si el modo de repetición es 'track', la pista actual se repite indefinidamente
    if state.get_loop_mode(key) == LoopMode::Track {
        return;
    }

    // 2. Si ya hay una operación de prefetch en curso para este reproductor, evitar llamadas concurrentes
    if state.is_prefetching(key) {
        return;
    }

    // 3. Verificar si hay pistas pendientes en la lista de reproducción o si autoplay está activo
    let has_pending = state
        .pending_playlists
        .get(&key)
        .map(|p| !p.is_empty())
        .unwrap_or(false);

    if !has_pending && !state.is_autoplay(key) {
        return;
    }

    // 4. Obtener el reproductor activo
    let driver_ref = match state.players.get(&key) {
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
    if !has_pending && state.is_autoplay(key) {
        let seed_opt = if let Some(ref curr) = current_track {
            let meta: Arc<TrackMetadata> = curr.data();
            Some(meta.source_url.clone())
        } else {
            state.get_last_track(key).map(|t| t.source_url)
        };

        if let Some(seed_url) = seed_opt {
            let clean_id = SourceResolver::extract_video_id(&seed_url).unwrap_or_else(|| seed_url.clone());
            info!(guild = key.guild_id, bot = key.bot_id, seed = %clean_id, "Autoplay activo: consultando recomendaciones en InnerTube (radio WEB_REMIX)...");
            if let Ok(items) = state.resolver.related_innertube(&clean_id, 25).await {
                let mut fresh: Vec<_> = items
                    .into_iter()
                    .filter(|item| {
                        if let Some(vid) = SourceResolver::extract_video_id(&item.url) {
                            !state.is_track_seen(key, &vid) && vid != clean_id
                        } else {
                            true
                        }
                    })
                    .collect();

                if fresh.is_empty() {
                    state.clear_seen_tracks(key);
                    fresh = state
                        .resolver
                        .related_innertube(&clean_id, 25)
                        .await
                        .unwrap_or_default();
                }

                if !fresh.is_empty() {
                    info!(
                        guild = key.guild_id,
                        bot = key.bot_id,
                        count = fresh.len(),
                        next = %fresh[0].title,
                        "Autoplay: recomendaciones obtenidas y añadidas a la cola pendiente"
                    );
                    let mut list = state.pending_playlists.entry(key).or_insert_with(Vec::new);
                    list.extend(fresh);
                }
            }
        }
    }

    // 7. Activar flag de prefetch y extraer la siguiente pista de pending_playlists
    if !state.start_prefetch(key) {
        return;
    }

    let next_item = match state.pending_playlists.get_mut(&key) {
        Some(mut list) if !list.is_empty() => list.remove(0),
        _ => {
            state.end_prefetch(key);
            return;
        }
    };

    let play_opts = state
        .guild_play_options
        .get(&key)
        .map(|o| o.clone())
        .unwrap_or_default();

    let state_clone = state.clone();
    let driver_clone = driver_ref.clone();

    // CleanupGuard: asegura que end_prefetch se llame siempre al salir del scope,
    // incluso si el tokio::spawn panica. Esto previene la fuga del flag prefetching_guilds.
    let guard_key = key;
    let state_clone_for_guard = state_clone.clone();

    tokio::spawn(async move {
        struct CleanupGuard {
            key: PlayerKey,
            state: AppState,
        }
        impl Drop for CleanupGuard {
            fn drop(&mut self) {
                self.state.end_prefetch(self.key);
            }
        }
        let _guard = CleanupGuard {
            key: guard_key,
            state: state_clone_for_guard,
        };

        let mut current_item = next_item;
        loop {
            info!(
                guild = guard_key.guild_id,
                bot = guard_key.bot_id,
                next_title = %current_item.title,
                trigger = %reason,
                "Prefetch anticipado: resolviendo y precalentando flujo de audio (gapless)..."
            );

            let resolve_res = state_clone
                .resolver
                .resolve_single(
                    &current_item.url,
                    play_opts.extractor_args.as_deref(),
                    play_opts.extra_args.as_deref(),
                    play_opts.cookies.as_deref(),
                    play_opts.format.as_deref(),
                )
                .await;

            match resolve_res {
                Ok(track) => {
                    let dsp_params = state_clone.get_dsp_params(guard_key);
                    let target_vol = state_clone.get_volume(guard_key);
                    let effective_vol = (target_vol * 0.95).clamp(0.0, 2.0);
                    let track_meta = Arc::new(track.clone());

                    match create_track_with_dsp(
                        state_clone.http_client.clone(),
                        track.stream_url.clone(),
                        track_meta.clone(),
                        dsp_params,
                        play_opts.user_agent.as_deref(),
                    )
                    .await
                    {
                        Ok(songbird_track) => {
                            let handle = {
                                let mut driver = driver_clone.lock().await;
                                let h = driver.enqueue(songbird_track).await;
                                let _ = h.set_volume(effective_vol);
                                h
                            };

                            attach_track_events(
                                guard_key,
                                track_meta,
                                &handle,
                                &state_clone.event_tx,
                                Some(state_clone.clone()),
                            );

                            if state_clone.get_loop_mode(guard_key) == LoopMode::Track {
                                let _ = handle.enable_loop();
                            }

                            info!(
                                guild = guard_key.guild_id,
                                bot = guard_key.bot_id,
                                title = %track.title,
                                "Prefetch exitoso: decodificador y stream listos en cola Songbird (transición gapless)"
                            );
                            break;
                        }
                        Err(err) => {
                            warn!(
                                guild = guard_key.guild_id,
                                bot = guard_key.bot_id,
                                title = %track.title,
                                error = %err,
                                "Fallo al abrir stream de pista prefetch; descartando pista fallida"
                            );
                            let is_idle = {
                                let driver = driver_clone.lock().await;
                                driver.queue().is_empty()
                            };
                            if is_idle {
                                if state_clone.record_track_error(guard_key) {
                                    warn!(
                                        guild = guard_key.guild_id,
                                        bot = guard_key.bot_id,
                                        "Circuit Breaker activado durante prefetch/autoplay: deteniendo cola."
                                    );
                                    state_clone.pending_playlists.remove(&guard_key);
                                    let _ = state_clone.event_tx.send(PlayerEvent::TrackError {
                                        guild_id: guard_key.guild_id,
                                        bot_id: Some(guard_key.bot_id),
                                        track: Some(track.to_response()),
                                        error: "Reproducción detenida por protección anti-bucle: múltiples canciones consecutivas no pudieron reproducirse.".to_string(),
                                    });
                                    break;
                                }
                                tokio::time::sleep(Duration::from_millis(500)).await;
                                if let Some(mut list) = state_clone.pending_playlists.get_mut(&guard_key) {
                                    if !list.is_empty() {
                                        current_item = list.remove(0);
                                        continue;
                                    }
                                }
                            }
                            break;
                        }
                    }
                }
                Err(err) => {
                    warn!(
                        guild = guard_key.guild_id,
                        bot = guard_key.bot_id,
                        url = %current_item.url,
                        title = %current_item.title,
                        error = %err,
                        "Fallo al resolver pista durante prefetch anticipado; continuará con la siguiente de la cola"
                    );
                    let is_idle = {
                        let driver = driver_clone.lock().await;
                        driver.queue().is_empty()
                    };
                    if is_idle {
                        if state_clone.record_track_error(guard_key) {
                            warn!(
                                guild = guard_key.guild_id,
                                bot = guard_key.bot_id,
                                "Circuit Breaker activado durante resolución prefetch: deteniendo cola."
                            );
                            state_clone.pending_playlists.remove(&guard_key);
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        if let Some(mut list) = state_clone.pending_playlists.get_mut(&guard_key) {
                            if !list.is_empty() {
                                current_item = list.remove(0);
                                continue;
                            }
                        }
                    }
                    break;
                }
            }
        }
    });
}

/// Verifica si la pista en reproducción actual contiene segmentos de SponsorBlock
/// y adelanta la posición automáticamente si el cabezal entra en una sección no musical.
async fn check_sponsorblock_skip(state: &AppState, key: PlayerKey) {
    let driver_ref = match state.players.get(&key) {
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
                            guild = key.guild_id,
                            bot = key.bot_id,
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
/// Si el modo 24/7 o un modo de bucle (Track/Queue) está activo, NO se desconecta.
async fn check_idle_cleanup(state: &AppState, key: PlayerKey) {
    // Protección 24/7 y Loop: si el modo 24/7 está activado o la repetición está activa,
    // nunca desconectar el bot por inactividad.
    if state.is_247(key) || state.get_loop_mode(key) != LoopMode::Off {
        state.touch_activity(key);
        return;
    }

    let timeout_secs: u64 = std::env::var("AXAUDIO_IDLE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);

    if timeout_secs == 0 {
        return;
    }

    let driver_ref = match state.players.get(&key) {
        Some(d) => d.clone(),
        None => return,
    };

    let is_empty = {
        let driver = driver_ref.lock().await;
        driver.queue().is_empty()
    };

    let has_pending = state
        .pending_playlists
        .get(&key)
        .map(|p| !p.is_empty())
        .unwrap_or(false);

    if !is_empty || has_pending {
        state.touch_activity(key);
    } else {
        let last = state.get_last_activity(key);
        if last.elapsed() >= std::time::Duration::from_secs(timeout_secs) {
            info!(
                guild = key.guild_id,
                bot = key.bot_id,
                idle_secs = last.elapsed().as_secs(),
                timeout = timeout_secs,
                "Garbage Collector: reproductor inactivo por más del límite configurado; liberando driver y recursos UDP"
            );
            cleanup_guild_player(state, key).await;
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

        let player_keys: Vec<PlayerKey> = state.players.iter().map(|entry| *entry.key()).collect();
        for key in player_keys {
            maybe_prefetch_next_track(&state, key).await;
            check_sponsorblock_skip(&state, key).await;
            check_idle_cleanup(&state, key).await;
        }
    }
}

/// Crea o actualiza un preset personalizado copiando los DSP params actuales de un guild
pub async fn preset_save(
    State(state): State<AppState>,
    Json(payload): Json<PresetSavePayload>,
) -> Result<Json<ApiResponse<PresetInfo>>, (StatusCode, Json<ApiResponse<PresetInfo>>)> {
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::err("El nombre del preset no puede estar vacío")),
        ));
    }

    // Copia los DSP params actuales del guild/bot, o usa defaults
    let params = if let Some(guild_id) = payload.guild_id {
        let player_key = state.resolve_player_key(guild_id, payload.bot_id);
        let dsp_params = state.get_dsp_params(player_key);
        match dsp_params.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    } else {
        crate::dsp::DspParams::default()
    };

    state.save_custom_preset(name.clone(), params.clone());

    let info = PresetInfo {
        name: name.clone(),
        description: payload.description,
        status: params.get_status(),
    };

    Ok(Json(ApiResponse::ok(info)))
}

/// Aplica un preset personalizado a un guild
pub async fn preset_apply(
    State(state): State<AppState>,
    Json(payload): Json<DspPayload>,
) -> Result<Json<ApiResponse<DspResponse>>, (StatusCode, Json<ApiResponse<DspResponse>>)> {
    let preset_name = payload.preset.as_ref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::err("Se requiere el campo 'preset'")),
        )
    })?;

    let player_key = state.resolve_player_key(payload.guild_id, payload.bot_id);
    let dsp_params = state.get_dsp_params(player_key);

    // Primero intenta preset personalizado, luego built-in
    let custom = state.get_custom_preset(preset_name);
    let (desc, new_params) = if let Some(cp) = &custom {
        (
            format!("Preset personalizado '{}' aplicado", preset_name),
            cp.clone(),
        )
    } else {
        match apply_preset_to_params(preset_name, &mut crate::dsp::DspParams::default()) {
            Ok(d) => {
                let mut p = crate::dsp::DspParams::default();
                apply_preset_to_params(preset_name, &mut p).ok();
                (d, p)
            }
            Err(e) => {
                return Err((StatusCode::BAD_REQUEST, Json(ApiResponse::err(e))));
            }
        }
    };

    let mut params = match dsp_params.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *params = new_params;

    let status = params.get_status();
    drop(params);
    state.persist_guild_settings();

    let res = DspResponse {
        guild_id: payload.guild_id,
        bot_id: Some(player_key.bot_id),
        status,
        message: desc,
    };

    Ok(Json(ApiResponse::ok(res)))
}

/// Lista todos los presets personalizados
pub async fn preset_list(State(state): State<AppState>) -> Json<ApiResponse<PresetListResponse>> {
    let presets: Vec<PresetInfo> = state
        .list_custom_presets()
        .into_iter()
        .filter_map(|name| {
            state.get_custom_preset(&name).map(|params| PresetInfo {
                name: name.clone(),
                description: None,
                status: params.get_status(),
            })
        })
        .collect();

    let response = PresetListResponse {
        count: presets.len(),
        presets,
    };
    Json(ApiResponse::ok(response))
}

/// Elimina un preset personalizado
pub async fn preset_delete(
    State(state): State<AppState>,
    Json(payload): Json<PresetSavePayload>,
) -> Result<Json<ApiResponse<()>>, (StatusCode, Json<ApiResponse<()>>)> {
    let name = payload.name.trim().to_string();
    if state.delete_custom_preset(&name) {
        Ok(Json(ApiResponse::ok(())))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ApiResponse::err(format!("Preset '{}' no encontrado", name))),
        ))
    }
}

