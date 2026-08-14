//! Whisper relay — server entrypoint.
//!
//! A zero-knowledge message relay for the Whisper E2EE chat.
//! The server is deliberately dumb: it forwards opaque, client-encrypted
//! envelopes between peers and holds zero plaintext, zero keys and
//! (by design) zero message content.

mod proxy;
mod relay;
mod store;

use std::net::SocketAddr;
use std::time::Duration;

use axum::{
    extract::{ws::WebSocketUpgrade, ConnectInfo, FromRef, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

use proxy::TrustedProxies;
use relay::Relay;

/// Shared router state. Both the relay handle and the trusted-proxy list are
/// exposed to handlers through `FromRef`, so the router carries a single
/// type (axum 0.8 does not destructure state tuples automatically).
#[derive(Clone)]
struct AppState {
    relay: Relay,
    proxies: TrustedProxies,
}

impl FromRef<AppState> for Relay {
    fn from_ref(state: &AppState) -> Self {
        state.relay.clone()
    }
}

impl FromRef<AppState> for TrustedProxies {
    fn from_ref(state: &AppState) -> Self {
        state.proxies.clone()
    }
}

/// Formats timestamps as local wall-clock RFC 3339 so relay logs match the
/// operator's clock instead of always showing UTC.
struct LocalTimer;

impl tracing_subscriber::fmt::time::FormatTime for LocalTimer {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        use time::format_description::well_known::Rfc3339;
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        let rendered = now.format(&Rfc3339).unwrap_or_else(|_| "?".into());
        w.write_str(&rendered)
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_timer(LocalTimer)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // Shared relay state: presence map + SQLite-backed offline queue.
    let relay = Relay::new();

    // Trusted reverse proxies (WHISPER_TRUSTED_PROXIES): when the relay runs
    // behind nginx/Caddy/Cloudflare, per-IP rate limiting must key on the
    // real client address from the forwarded headers, not the proxy's IP.
    let proxies = TrustedProxies::from_env();
    let app_state = AppState {
        relay: relay.clone(),
        proxies,
    };

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
        .with_state(app_state);

    let addr: SocketAddr = std::env::var("WHISPER_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .expect("invalid WHISPER_ADDR");

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "whisper-relay listening on {addr}"
    );
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind listener");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .expect("server error");
    tracing::info!("whisper-relay shut down cleanly");
}

/// Wait for a shutdown signal so the relay can drain gracefully.
///
/// Ctrl+C (SIGINT) is handled on every platform via `tokio::signal::ctrl_c`;
/// on Unix, SIGTERM (e.g. `systemctl stop`) is handled too. Instead of the
/// process being hard-killed (the `STATUS_CONTROL_C_EXIT` exit code Windows
/// reports for an unhandled Ctrl+C), `axum::serve` now stops accepting new
/// connections, lets in-flight handlers finish and returns, so `main` exits
/// with a clean status.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("SIGINT received, draining gracefully");
        }
        _ = terminate => {
            tracing::info!("SIGTERM received, draining gracefully");
        }
    }
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
            tracing::debug!(
                hash = %hash,
                size = bytes.len(),
                content_type,
                "media blob served"
            );
            let mut response = Response::new(bytes.into());
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
            response
        }
        // A missing blob is the expected 404 case (never uploaded or purged),
        // so it is logged at debug rather than warned about.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(hash = %hash, "media blob not found");
            StatusCode::NOT_FOUND.into_response()
        }
        // Unexpected I/O errors (permissions, disk, ...) are real problems and
        // deserve a warning with the full context.
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
/// passing the peer's source IP for rate limiting. When the connection comes
/// from a configured trusted proxy, the real client IP is recovered from the
/// forwarded headers first (see [`TrustedProxies`]).
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(relay): State<Relay>,
    State(proxies): State<TrustedProxies>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    let ip = proxies.resolve_client_ip(addr, &headers).ip().to_string();
    ws.on_upgrade(move |socket| {
        // Clone so the spawned future owns its relay handle outright.
        let relay = relay.clone();
        async move { relay.handle_socket(socket, ip).await }
    })
}
