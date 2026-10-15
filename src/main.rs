mod handlers;
mod models;
mod state;

use axum::{
    routing::{get, post},
    Router,
};
use state::AppState;
use std::net::SocketAddr;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "axAudio=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("⚡ Starting axAudio Native Audio Engine (Lavalink Replacement in Rust)...");

    let state = AppState::new();

    let app = Router::new()
        .route("/health", get(handlers::health))
        .route("/v1/stats", get(handlers::get_stats))
        .route("/v1/voice/join", post(handlers::voice_join))
        .route("/v1/voice/leave", post(handlers::voice_leave))
        .route("/v1/player/play", post(handlers::player_play))
        .route("/v1/player/control", post(handlers::player_control))
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(2333);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("🚀 axAudio server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
