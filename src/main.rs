mod dsp;
mod handlers;
mod models;
mod sources;
mod state;

use axum::{
    routing::{get, post},
    Router,
};
use state::AppState;
use std::net::SocketAddr;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Sintonización de CPU para entornos VPS con 2 vCPUs:
    // Limitar worker_threads a 2 y max_blocking_threads a 4 para prevenir contención de cores
    let worker_threads = std::env::var("TOKIO_WORKER_THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(2);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .max_blocking_threads(4)
        .enable_all()
        .build()?;

    runtime.block_on(async_main())
}

async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "axAudio=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("⚡ Starting axAudio Native Audio Engine (Lavalink Replacement in Rust)...");

    let state = AppState::new();

    // Tarea en segundo plano para purgar entradas de caché expiradas cada 10 minutos
    let resolver_for_cleanup = state.resolver.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(600));
        loop {
            interval.tick().await;
            resolver_for_cleanup.prune_cache();
        }
    });

    let app = Router::new()
        .route("/health", get(handlers::health))
        .route("/v1/stats", get(handlers::get_stats))
        .route("/v1/tracks/resolve", post(handlers::resolve_track))
        .route("/v1/tracks/search", post(handlers::search_tracks))
        .route("/v1/voice/join", post(handlers::voice_join))
        .route("/v1/voice/leave", post(handlers::voice_leave))
        .route("/v1/player/play", post(handlers::player_play))
        .route("/v1/player/control", post(handlers::player_control))
        .route("/v1/player/queue", post(handlers::player_queue))
        .route("/v1/player/eq", post(handlers::player_eq))
        .route("/v1/player/dsp", post(handlers::player_dsp))
        .route("/v1/events", get(handlers::ws_events))
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(2333);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("🚀 axAudio server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut stream) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            stream.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("🛑 Graceful shutdown signal received, terminating axAudio safely...");
}
