//! Core relay logic: connection registry + opaque envelope routing.
//!
//! SECURITY MODEL
//! --------------
//! - The relay NEVER reads or decrypts message payloads.
//! - The relay NEVER sees private keys or plaintext.
//! - The relay only routes `Envelope`s between peer IDs.
//! - Peer IDs are derived from public keys (X25519), not personal data.
//! - Each socket authenticates with a [`SignedHello`]; envelopes are only
//!   routed when the sender field matches the authenticated peer ID.
//! - Only opaque ciphertext blobs are persisted (SQLite), bounded per peer
//!   (MAX_OFFLINE_BLOBS) and expired by TTL (7 days).
//! - Peers publish their public X3DH pre-key bundle (PreKeyBundle) so other
//!   peers can start encrypted sessions. Bundles are verified (Ed25519
//!   signature) and bound to the publishing peer before being stored.
//! - Envelope throughput and pre-key traffic are rate limited per source IP
//!   (token buckets).

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use e2ee_core::prekey::PreKeyBundle;
use e2ee_core::{Identity, SignedHello};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};

use crate::store::{unix_now, Store};

/// Upper bound for a single relayed envelope (ciphertext blob size cap).
/// Keeps the server DoS-resistant and the network light.
const MAX_ENVELOPE_BYTES: usize = 8 * 1024 * 1024; // 8 MiB

/// Seconds a client may take to send its `hello` before being dropped.
const HELLO_TIMEOUT_SECS: u64 = 10;

/// Default per-IP token bucket: burst of 60 envelopes, refilled at 1/sec
/// (~60 envelopes per minute).
const DEFAULT_RATE_BURST: f64 = 60.0;
const DEFAULT_RATE_REFILL_PER_SEC: f64 = 1.0;

// ---------------------------------------------------------------------------
// Wire format
// ---------------------------------------------------------------------------

/// Routing envelope — the ONLY message shape the server understands.
///
/// `payload` is ALWAYS fully encrypted by the client (Double Ratchet).
/// The server treats it as opaque bytes and routes it by `recipient`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    /// Sender peer ID (hash of the sender's X25519 identity key).
    pub sender: String,
    /// Recipient peer ID.
    pub recipient: String,
    /// Opaque ciphertext blob, base64-encoded by the client.
    /// The server never decodes or inspects it — it is routed as-is.
    pub payload: String,
    /// Client-generated monotonic hint for replay/ordering awareness.
    pub seq: u64,
}

impl Envelope {
    /// Size guard; oversized envelopes are dropped before routing.
    /// base64 inflates by ~33%, so compare against the encoded length.
    pub fn within_limits(&self) -> bool {
        self.payload.len() <= MAX_ENVELOPE_BYTES
    }
}

/// Messages the CLIENT sends to the server.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    /// First message on a fresh socket — announces a signed hello binding
    /// the peer ID to the sender's public keys.
    Hello {
        peer_id: String,
        curve25519_key: String,
        ed25519_key: String,
        signature: String,
    },
    /// An opaque envelope to route onward.
    Envelope { envelope: Envelope },
    /// Offline sync: request every queued envelope for this peer with
    /// `seq > since`. The server replies with a single
    /// `{"type":"envelopes","envelopes":[...]}` batch and removes the
    /// delivered rows (a successful fetch acknowledges delivery).
    #[serde(rename = "fetch_since")]
    FetchSince { since: u64 },
    /// Publish the caller's current X3DH pre-key bundle so other peers can
    /// fetch it and start an encrypted session. The bundle is verified and
    /// bound to the authenticated peer ID before being persisted.
    #[serde(rename = "publish_prekeys")]
    PublishPrekeys { bundle: Box<PreKeyBundle> },
    /// Request another peer's pre-key bundle (public directory lookup).
    #[serde(rename = "fetch_prekeys")]
    FetchPrekeys { peer_id: String },
}

/// Messages the SERVER sends to the client.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    /// A routed envelope destined for this peer.
    Envelope { envelope: Envelope },
    /// Batch reply to `fetch_since`: all queued envelopes with `seq > since`,
    /// oldest first. Empty array when there is nothing new.
    Envelopes { envelopes: Vec<Envelope> },
    /// Delivery confirmation for a sent envelope (by client seq).
    #[serde(rename = "ack")]
    Acknowledged { seq: u64 },
    /// Confirmation that a published pre-key bundle was accepted and stored.
    #[serde(rename = "prekeys_published")]
    PrekeysPublished,
    /// The requested peer's pre-key bundle.
    #[serde(rename = "prekeys")]
    Prekeys { bundle: Box<PreKeyBundle> },
    /// Protocol error.
    Error { code: String },
}

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------

/// Per-IP token bucket. Each accepted envelope consumes one token; tokens are
/// refilled continuously up to the burst capacity.
///
/// Pre-key traffic shares the same limiter but keys its buckets as
/// `prekey:<ip>`, so a pre-key storm can neither starve envelope routing nor
/// leak into the envelope budget (and vice versa).
pub struct RateLimiter {
    buckets: std::sync::Mutex<HashMap<String, Bucket>>,
    burst: f64,
    refill_per_sec: f64,
}

#[derive(Clone, Copy)]
struct Bucket {
    tokens: f64,
    last: std::time::Instant,
}

impl RateLimiter {
    fn new(burst: f64, refill_per_sec: f64) -> Self {
        Self {
            buckets: std::sync::Mutex::new(HashMap::new()),
            burst,
            refill_per_sec,
        }
    }

    /// Build a limiter from environment overrides:
    /// `WHISPER_RATE_BURST` (max burst) and `WHISPER_RATE_REFILL` (tokens/sec).
    fn from_env() -> Self {
        let burst = std::env::var("WHISPER_RATE_BURST")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(DEFAULT_RATE_BURST);
        let refill = std::env::var("WHISPER_RATE_REFILL")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(DEFAULT_RATE_REFILL_PER_SEC);
        Self::new(burst, refill)
    }

    /// Try to consume one token for `key`. Returns `false` when the bucket is
    /// exhausted (rate limit hit).
    fn try_take(&self, key: &str) -> bool {
        let mut buckets = self.buckets.lock().unwrap();
        let now = std::time::Instant::now();
        let bucket = buckets.entry(key.to_string()).or_insert_with(|| Bucket {
            tokens: self.burst,
            last: now,
        });
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(self.burst);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Relay state
// ---------------------------------------------------------------------------

type PeerId = String;
/// Outbound channel: WS messages queued for a connected peer.
type Outbound = mpsc::UnboundedSender<WsMessage>;

/// Outcome of authenticating a signed hello.
enum HelloOutcome {
    /// The hello verified and the peer is authenticated.
    Accepted(PeerId),
    /// The hello was rejected; the socket must be dropped.
    Rejected { code: String },
}

/// Clonable handle to shared relay state.
#[derive(Clone)]
pub struct Relay {
    inner: Arc<RelayInner>,
}

struct RelayInner {
    /// Online peers: peer_id -> outbound channel.
    online: RwLock<HashMap<PeerId, Outbound>>,
    /// SQLite-backed offline queue of opaque ciphertext blobs.
    store: Store,
    /// Per-IP envelope throughput guard.
    limiter: RateLimiter,
}

impl Relay {
    /// Build a relay backed by the on-disk SQLite store
    /// (`server/data/relay.db`, overridable via `WHISPER_DB_PATH`).
    pub fn new() -> Self {
        let path = std::env::var("WHISPER_DB_PATH").unwrap_or_else(|_| "data/relay.db".into());
        let store = Store::open(&path).expect("failed to open SQLite store");
        Self::with_store(store)
    }

    /// Build a relay over a pre-opened store (tests use in-memory stores).
    fn with_store(store: Store) -> Self {
        Self {
            inner: Arc::new(RelayInner {
                online: RwLock::new(HashMap::new()),
                store,
                limiter: RateLimiter::from_env(),
            }),
        }
    }

    /// Accept a connected socket: wait for hello, register, pump messages.
    /// `ip` is the peer's source address, used for rate limiting.
    pub async fn handle_socket(&self, socket: WebSocket, ip: String) {
        let (mut tx, mut rx) = socket.split();

        // 1) Authenticate the socket: wait for the peer's signed hello.
        let peer_id = match self.await_hello(&mut rx, &mut tx).await {
            Some(id) => id,
            None => return,
        };
        tracing::info!(peer = %peer_id, ip = %ip, "peer connected");

        // 2) Register the peer as online. A second socket claiming the same
        //    peer ID replaces the previous one (last-socket-wins); identity
        //    binding itself was already enforced by the signed hello, so this
        //    only ever happens for a verified owner reconnecting.
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        self.inner
            .online
            .write()
            .await
            .insert(peer_id.clone(), out_tx);

        // 3) Push any ciphertext blobs persisted while the peer was offline.
        //    Rows are left in the DB until a fetch_since drains them, so the
        //    client can re-pull its offline history.
        let blobs = self.inner.store.list_for(&peer_id, unix_now());
        for env in blobs {
            if let Ok(text) = serde_json::to_string(&ServerMessage::Envelope { envelope: env }) {
                if tx.send(WsMessage::Text(text)).await.is_err() {
                    break;
                }
            }
        }

        // 4) Pump outbound queue -> socket (spawned so routing never blocks).
        let pump_out = tokio::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                if tx.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // 5) Inbound loop: read envelopes and sync requests from the socket.
        while let Some(Ok(msg)) = rx.next().await {
            match msg {
                WsMessage::Text(text) => {
                    let parsed: Result<ClientMessage, _> = serde_json::from_str(&text);
                    match parsed {
                        Ok(ClientMessage::Envelope { envelope }) => {
                            self.route(envelope, &peer_id, &ip).await;
                        }
                        Ok(ClientMessage::FetchSince { since }) => {
                            let blobs = self.fetch_since(&peer_id, since).await;
                            let _ = self
                                .send(&peer_id, ServerMessage::Envelopes { envelopes: blobs })
                                .await;
                        }
                        Ok(ClientMessage::PublishPrekeys { bundle }) => {
                            self.publish_prekeys(&peer_id, &ip, *bundle).await;
                        }
                        Ok(ClientMessage::FetchPrekeys { peer_id: target }) => {
                            self.fetch_prekeys(&peer_id, &ip, &target).await;
                        }
                        // Re-registration or protocol violations: ignore for now.
                        Ok(_) => {}
                        Err(_) => {
                            let _ = self
                                .send(
                                    &peer_id,
                                    ServerMessage::Error {
                                        code: "bad_message".into(),
                                    },
                                )
                                .await;
                        }
                    }
                }
                WsMessage::Close(_) => break,
                // Ping/pong are handled by the underlying websocket library.
                WsMessage::Ping(_) | WsMessage::Pong(_) => {}
                WsMessage::Binary(_) => {
                    let _ = self
                        .send(
                            &peer_id,
                            ServerMessage::Error {
                                code: "binary_not_supported".into(),
                            },
                        )
                        .await;
                }
            }
        }

        // 6) Cleanup: unregister and drop the outbound pump.
        self.inner.online.write().await.remove(&peer_id);
        pump_out.abort();
        tracing::info!(peer = %peer_id, "peer disconnected");
    }

    /// Wait for the client's signed `hello` within a timeout window.
    /// Returns the authenticated peer ID, or None if the handshake failed.
    async fn await_hello<S, T>(&self, rx: &mut S, tx: &mut T) -> Option<PeerId>
    where
        S: StreamExt<Item = Result<WsMessage, axum::Error>> + Unpin,
        T: SinkExt<WsMessage> + Unpin,
    {
        let deadline = tokio::time::Duration::from_secs(HELLO_TIMEOUT_SECS);
        let result = tokio::time::timeout(deadline, async {
            while let Some(Ok(msg)) = rx.next().await {
                if let WsMessage::Text(text) = msg {
                    if let Ok(ClientMessage::Hello {
                        peer_id,
                        curve25519_key,
                        ed25519_key,
                        signature,
                    }) = serde_json::from_str(&text)
                    {
                        let hello = SignedHello {
                            peer_id,
                            curve25519_key,
                            ed25519_key,
                            signature,
                        };
                        return Some(self.validate_hello(&hello));
                    }
                }
            }
            None
        })
        .await;

        match result {
            Ok(Some(HelloOutcome::Accepted(peer_id))) => Some(peer_id),
            Ok(Some(HelloOutcome::Rejected { code })) => {
                // Signature, key binding or identity conflict — drop the socket.
                tracing::warn!(code = %code, "hello rejected");
                let _ = tx
                    .send(WsMessage::Text(
                        serde_json::to_string(&ServerMessage::Error { code }).unwrap(),
                    ))
                    .await;
                None
            }
            Ok(None) => None,
            Err(_) => {
                // Hello timeout — drop the connection.
                let _ = tx
                    .send(WsMessage::Text(
                        serde_json::to_string(&ServerMessage::Error {
                            code: "hello_timeout".into(),
                        })
                        .unwrap(),
                    ))
                    .await;
                None
            }
        }
    }

    /// Authenticate a signed hello and bind the peer ID to its public keys.
    ///
    /// Rejects with `invalid_hello` when the signature/key binding fails and
    /// with `identity_conflict` when the peer ID is already bound to a
    /// different Ed25519 key ("first-seen wins"). Newly seen peers are
    /// registered with their keys; returning peers keep their existing keys.
    fn validate_hello(&self, hello: &SignedHello) -> HelloOutcome {
        if let Err(err) = Identity::verify_signed_hello(hello) {
            tracing::warn!(peer = %hello.peer_id, "hello verification failed: {err}");
            return HelloOutcome::Rejected {
                code: "invalid_hello".into(),
            };
        }

        if let Some((_, existing_ed)) = self.inner.store.get_user_keys(&hello.peer_id) {
            if existing_ed != hello.ed25519_key {
                tracing::warn!(peer = %hello.peer_id, "identity conflict: peer id already bound to a different ed25519 key");
                return HelloOutcome::Rejected {
                    code: "identity_conflict".into(),
                };
            }
        }

        let _ = self.inner.store.register_user_with_keys(
            &hello.peer_id,
            &hello.curve25519_key,
            &hello.ed25519_key,
            unix_now(),
        );
        HelloOutcome::Accepted(hello.peer_id.clone())
    }

    /// Route an envelope to its recipient, or persist it for offline delivery.
    /// Envelope sends are rate limited per source IP.
    async fn route(&self, envelope: Envelope, sender_peer: &str, ip: &str) {
        // Rate limit BEFORE any other processing: the bucket protects the
        // whole relay regardless of envelope size.
        if !self.inner.limiter.try_take(ip) {
            tracing::warn!(ip = %ip, "rate limit exceeded");
            let _ = self
                .send(
                    sender_peer,
                    ServerMessage::Error {
                        code: "rate_limited".into(),
                    },
                )
                .await;
            return;
        }

        // Spoofing guard: the claimed sender must be the authenticated peer.
        // A mismatched envelope is never routed, queued or acked.
        if envelope.sender != sender_peer {
            tracing::warn!(
                claimed = %envelope.sender,
                authenticated = %sender_peer,
                "envelope sender does not match the authenticated peer"
            );
            let _ = self
                .send(
                    sender_peer,
                    ServerMessage::Error {
                        code: "sender_mismatch".into(),
                    },
                )
                .await;
            return;
        }

        if !envelope.within_limits() {
            tracing::warn!(sender = %envelope.sender, "dropping oversized envelope");
            return;
        }

        // Grab the seq and recipient before `envelope` may be moved into the
        // offline store.
        let seq = envelope.seq;

        // Deliver live if possible...
        let delivered = {
            let online = self.inner.online.read().await;
            match online.get(&envelope.recipient) {
                Some(tx) => {
                    let msg = serde_json::to_string(&ServerMessage::Envelope {
                        envelope: envelope.clone(),
                    });
                    match msg {
                        Ok(text) => {
                            let _ = tx.send(WsMessage::Text(text));
                            true
                        }
                        Err(_) => false,
                    }
                }
                None => false,
            }
        };

        if delivered {
            tracing::trace!(recipient = %envelope.recipient, "envelope delivered live");
        } else {
            // ...otherwise persist the ciphertext blob in SQLite. The per-peer
            // cap and 7-day TTL keep the database bounded.
            let _ = self.inner.store.enqueue(&envelope, unix_now());
            tracing::trace!(recipient = %envelope.recipient, "envelope queued offline");
        }

        // Delivery confirmation to the sender: the relay accepted the blob.
        // This is NOT a read receipt — read receipts are end-to-end and
        // travel as regular encrypted envelopes between clients.
        let _ = self
            .send(sender_peer, ServerMessage::Acknowledged { seq })
            .await;
    }

    /// Fetch and drain the offline queue for a peer (the fetch_since sync
    /// mechanism): returns every stored envelope with `seq > since`, oldest
    /// first, and removes them from the store.
    pub async fn fetch_since(&self, peer_id: &str, since: u64) -> Vec<Envelope> {
        self.inner.store.drain_since(peer_id, since, unix_now())
    }

    /// Purge expired offline envelopes. Returns the number of purged rows.
    pub async fn purge_expired(&self) -> usize {
        self.inner
            .store
            .purge_expired(unix_now())
            .unwrap_or_default()
    }

    /// Publish a peer's pre-key bundle so other peers can fetch it for the
    /// X3DH handshake. The bundle is only accepted when:
    /// 1. its Ed25519 signature verifies over the identity and one-time keys
    ///    (`ensure_valid`), and
    /// 2. the identity key fingerprints to the authenticated peer ID.
    ///
    /// Pre-key traffic is rate limited per source IP under the `prekey:<ip>`
    /// bucket (see [`RateLimiter`]).
    async fn publish_prekeys(&self, peer_id: &str, ip: &str, bundle: PreKeyBundle) {
        if !self.inner.limiter.try_take(&format!("prekey:{ip}")) {
            tracing::warn!(ip = %ip, "pre-key rate limit exceeded");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "rate_limited".into(),
                    },
                )
                .await;
            return;
        }

        if let Err(err) = bundle.ensure_valid() {
            tracing::warn!(peer = %peer_id, "rejecting invalid pre-key bundle: {err}");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "invalid_bundle".into(),
                    },
                )
                .await;
            return;
        }

        let derived = Identity::peer_id_from_curve25519(&bundle.identity_key);
        if derived != peer_id {
            tracing::warn!(peer = %peer_id, derived = %derived, "pre-key bundle identity mismatch");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "identity_mismatch".into(),
                    },
                )
                .await;
            return;
        }

        match serde_json::to_string(&bundle) {
            Ok(json) => match self.inner.store.set_prekeys(peer_id, &json, unix_now()) {
                Ok(()) => {
                    let _ = self.send(peer_id, ServerMessage::PrekeysPublished).await;
                }
                Err(err) => {
                    tracing::error!(peer = %peer_id, "failed to persist pre-key bundle: {err}");
                }
            },
            Err(err) => {
                tracing::error!(peer = %peer_id, "failed to serialize pre-key bundle: {err}")
            }
        }
    }

    /// Return the pre-key bundle another peer published, or `no_prekeys` when
    /// none is stored. Pre-key fetches are public directory lookups: any
    /// authenticated peer may query any other peer. Fetching is rate limited
    /// per source IP under the `prekey:<ip>` bucket like publishing.
    async fn fetch_prekeys(&self, peer_id: &str, ip: &str, target: &str) {
        if !self.inner.limiter.try_take(&format!("prekey:{ip}")) {
            tracing::warn!(ip = %ip, "pre-key rate limit exceeded");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "rate_limited".into(),
                    },
                )
                .await;
            return;
        }

        match self.inner.store.get_prekeys(target) {
            Some(json) => match serde_json::from_str::<PreKeyBundle>(&json) {
                Ok(bundle) => {
                    let _ = self
                        .send(
                            peer_id,
                            ServerMessage::Prekeys {
                                bundle: Box::new(bundle),
                            },
                        )
                        .await;
                }
                Err(err) => {
                    tracing::error!(peer = %target, "stored pre-key bundle is corrupt: {err}");
                }
            },
            None => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "no_prekeys".into(),
                        },
                    )
                    .await;
            }
        }
    }

    /// Send a server message to a specific peer if they are online.
    async fn send(&self, peer_id: &str, msg: ServerMessage) -> bool {
        let online = self.inner.online.read().await;
        match online.get(peer_id) {
            Some(tx) => match serde_json::to_string(&msg) {
                Ok(text) => {
                    let _ = tx.send(WsMessage::Text(text));
                    true
                }
                Err(_) => false,
            },
            None => false,
        }
    }
}

impl Default for Relay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ENVELOPE_TTL_SECS, MAX_OFFLINE_BLOBS};

    fn env(sender: &str, recipient: &str, seq: u64) -> Envelope {
        Envelope {
            sender: sender.into(),
            recipient: recipient.into(),
            payload: format!("blob-{seq}"),
            seq,
        }
    }

    #[test]
    fn limiter_allows_burst_then_rejects() {
        let l = RateLimiter::new(2.0, 0.0);
        assert!(l.try_take("ip-a"));
        assert!(l.try_take("ip-a"));
        assert!(!l.try_take("ip-a"));
    }

    #[test]
    fn limiter_is_per_key() {
        let l = RateLimiter::new(1.0, 0.0);
        assert!(l.try_take("ip-a"));
        assert!(!l.try_take("ip-a"), "ip-a must be exhausted");
        assert!(l.try_take("ip-b"), "ip-b has its own bucket");
    }

    #[test]
    fn limiter_refills_over_time() {
        let l = RateLimiter::new(2.0, 1000.0); // 1000 tokens/sec
        assert!(l.try_take("ip-a"));
        assert!(l.try_take("ip-a"));
        assert!(!l.try_take("ip-a"));
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(l.try_take("ip-a"), "tokens must refill over time");
    }

    #[tokio::test]
    async fn relay_fetch_since_returns_and_drains_stored_envelopes() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        for seq in 1..=3 {
            store.enqueue(&env("a", "b", seq), now).unwrap();
        }
        let relay = Relay::with_store(store);
        // seq > 1, oldest first: seq 2 and 3.
        let fetched = relay.fetch_since("b", 1).await;
        assert_eq!(fetched.len(), 2);
        assert_eq!(fetched[0].seq, 2);
        assert_eq!(fetched[1].seq, 3);
        // Delivered rows are removed; seq 1 remains.
        assert_eq!(relay.inner.store.count_for("b"), 1);
        assert!(relay.fetch_since("b", 1).await.is_empty());
    }

    #[tokio::test]
    async fn relay_purge_expired_removes_ttl_rows() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        store
            .enqueue(&env("a", "b", 1), now - ENVELOPE_TTL_SECS - 1)
            .unwrap();
        store.enqueue(&env("a", "b", 2), now).unwrap();
        let relay = Relay::with_store(store);
        assert_eq!(relay.purge_expired().await, 1);
        let remaining = relay.fetch_since("b", 0).await;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].seq, 2);
    }

    #[tokio::test]
    async fn relay_offline_cap_is_enforced_via_store() {
        let store = Store::open_in_memory().unwrap();
        let now = unix_now();
        for seq in 0..(MAX_OFFLINE_BLOBS as u64 + 5) {
            store.enqueue(&env("a", "b", seq), now).unwrap();
        }
        let relay = Relay::with_store(store);
        assert_eq!(relay.inner.store.count_for("b"), MAX_OFFLINE_BLOBS);
    }

    #[test]
    fn validate_hello_accepts_and_registers_keys() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let identity = Identity::new();
        let hello = identity.signed_hello();

        let outcome = relay.validate_hello(&hello);
        let accepted = match outcome {
            HelloOutcome::Accepted(id) => id,
            HelloOutcome::Rejected { code } => panic!("expected acceptance, got {code}"),
        };
        assert_eq!(accepted, hello.peer_id);

        let (curve, ed) = relay
            .inner
            .store
            .get_user_keys(&hello.peer_id)
            .expect("keys must be stored");
        assert_eq!(curve, hello.curve25519_key);
        assert_eq!(ed, hello.ed25519_key);
    }

    #[test]
    fn validate_hello_rejects_invalid_signature() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let identity = Identity::new();
        let mut hello = identity.signed_hello();
        // Truncating the base64 makes it an invalid signature.
        hello.signature.truncate(10);

        let outcome = relay.validate_hello(&hello);
        match outcome {
            HelloOutcome::Accepted(_) => panic!("an invalid signature must be rejected"),
            HelloOutcome::Rejected { code } => assert_eq!(code, "invalid_hello"),
        }
    }

    #[test]
    fn validate_hello_rejects_identity_conflict() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let first = Identity::new();
        let hello = first.signed_hello();
        relay.validate_hello(&hello);

        // Same curve key (hence same peer ID) but a different Ed25519 key:
        // the signature verifies, yet the identity conflicts with the
        // already registered one.
        let other = Identity::new();
        let mut conflict = hello.clone();
        conflict.ed25519_key = other.ed25519_key().to_base64();
        conflict.signature = other.sign(conflict.peer_id.as_bytes()).to_base64();

        let outcome = relay.validate_hello(&conflict);
        match outcome {
            HelloOutcome::Accepted(_) => panic!("a conflicting identity must be rejected"),
            HelloOutcome::Rejected { code } => assert_eq!(code, "identity_conflict"),
        }

        // The original identity is untouched.
        let (_, ed) = relay.inner.store.get_user_keys(&hello.peer_id).unwrap();
        assert_eq!(ed, hello.ed25519_key);
    }

    #[tokio::test]
    async fn publish_and_fetch_prekeys_roundtrip_preserves_bundle() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let mut identity = Identity::new();
        let peer_id = identity.peer_id();
        let bundle = identity.pre_key_bundle(3);

        relay
            .publish_prekeys(&peer_id, "127.0.0.1", bundle.clone())
            .await;
        let json = relay
            .inner
            .store
            .get_prekeys(&peer_id)
            .expect("bundle must be persisted");
        let restored: PreKeyBundle =
            serde_json::from_str(&json).expect("stored bundle must deserialize");
        assert_eq!(restored, bundle);
    }

    #[tokio::test]
    async fn publish_prekeys_rejects_identity_mismatch() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let owner = Identity::new();
        let peer_id = owner.peer_id();
        // A valid bundle owned by a different identity must be rejected: its
        // identity key fingerprints to the other peer, not to `peer_id`.
        let mut other = Identity::new();
        let foreign = other.pre_key_bundle(3);

        relay.publish_prekeys(&peer_id, "127.0.0.1", foreign).await;
        assert_eq!(relay.inner.store.get_prekeys(&peer_id), None);
    }

    #[tokio::test]
    async fn publish_prekeys_rejects_invalid_bundle() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let mut identity = Identity::new();
        let peer_id = identity.peer_id();
        let mut bundle = identity.pre_key_bundle(2);
        // Swapping a one-time key invalidates the signature.
        bundle.one_time_keys[0] = Identity::new().curve25519_key();

        relay.publish_prekeys(&peer_id, "127.0.0.1", bundle).await;
        assert_eq!(relay.inner.store.get_prekeys(&peer_id), None);
    }
}
