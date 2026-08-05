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
    extract::{ws::WebSocketUpgrade, ConnectInfo, Path, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
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
        .route("/media/{hash}", get(media))
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

/// Serve a stored avatar blob by its SHA-256 hash.
///
/// The hash is validated strictly (64 lowercase hex chars) before any file
/// access, so the endpoint cannot be used to read arbitrary paths; unknown
/// blobs yield 404. The Content-Type is sniffed from the blob's magic bytes
/// and defaults to `image/png` for anything unrecognized.
async fn media(Path(hash): Path<String>, State(relay): State<Relay>) -> Response {
    let hash = hash.to_lowercase();
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(relay.media_path(&hash)).await {
        Ok(bytes) => {
            let content_type = image_content_type(&bytes);
            let mut response = Response::new(bytes.into());
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
            response
        }
        Err(err) => {
            tracing::warn!(hash = %hash, path = %relay.media_path(&hash).display(), "media blob read failed: {err}");
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

/// Sniff the image content type from the blob's leading magic bytes.
fn image_content_type(bytes: &[u8]) -> &'static str {
    let head = &bytes[..bytes.len().min(12)];
    if head.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if head.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if head.starts_with(b"GIF8") {
        "image/gif"
    } else if head.len() >= 12 && head[..4] == *b"RIFF" && head[8..12] == *b"WEBP" {
        "image/webp"
    } else {
        "image/png"
    }
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
