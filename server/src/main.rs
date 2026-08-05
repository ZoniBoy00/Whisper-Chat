//! Whisper relay — server entrypoint.
//!
//! A zero-knowledge message relay for the Whisper E2EE chat.
//! The server is deliberately dumb: it forwards opaque, client-encrypted
//! envelopes between peers and holds zero plaintext, zero keys and
//! (by design) zero message content.

mod relay;
mod store;

use std::net::SocketAddr;
use std::time::Duration;

use axum::{
    extract::{ws::WebSocketUpgrade, ConnectInfo, State},
    response::IntoResponse,
    routing::get,
    Router,
};

use relay::Relay;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,whisper_relay=debug".into()),
        )
        .init();

    // Shared relay state: presence map + SQLite-backed offline queue.
    let relay = Relay::new();
    let state = relay.clone();

    // Hourly sweep: purge offline envelopes past their 7-day TTL.
    {
        let relay = relay.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(3600));
            loop {
                tick.tick().await;
                let purged = relay.purge_expired().await;
                if purged > 0 {
                    tracing::info!(purged, "expired envelopes purged");
                }
            }
        });
    }

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr: SocketAddr = std::env::var("WHISPER_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .expect("invalid WHISPER_ADDR");

    tracing::info!("whisper-relay listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind listener");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .expect("server error");
}

/// Liveness probe — no sensitive info ever exposed here.
async fn health() -> &'static str {
    "whisper-relay: ok"
}

/// Upgrade an incoming connection to WebSocket and hand it to the relay,
/// passing the peer's source IP for rate limiting.
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(relay): State<Relay>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| {
        // Clone so the spawned future owns its relay handle outright.
        let relay = relay.clone();
        let ip = addr.ip().to_string();
        async move { relay.handle_socket(socket, ip).await }
    })
}
