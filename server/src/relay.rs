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
//! - Groups are metadata only: the relay stores the group roster and fans
//!   `send_group_message` envelopes out to every member. The Megolm session
//!   key is SECRET and is never seen or stored by the relay — it travels
//!   end-to-end between members inside Double Ratchet envelopes.
//! - Envelope throughput and pre-key traffic are rate limited per source IP
//!   (token buckets).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use base64::{engine::general_purpose::STANDARD, Engine};
use e2ee_core::prekey::PreKeyBundle;
use e2ee_core::profile::{validate_username, verify_username_signature};
use e2ee_core::{Identity, SignedHello};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, RwLock};
use vodozemac::{Curve25519PublicKey, Ed25519PublicKey, Ed25519Signature};

use crate::store::{unix_now, Store};

/// Upper bound for a single relayed envelope (ciphertext blob size cap).
/// Keeps the server DoS-resistant and the network light.
const MAX_ENVELOPE_BYTES: usize = 8 * 1024 * 1024; // 8 MiB

/// Seconds a client may take to send its `hello` before being dropped.
const HELLO_TIMEOUT_SECS: u64 = 10;

/// Maximum length of a public display name, in Unicode characters.
const MAX_DISPLAY_NAME_CHARS: usize = 64;

/// Maximum length of a group name, in Unicode characters.
const MAX_GROUP_NAME_CHARS: usize = 64;

/// Maximum size of an uploaded avatar blob, in bytes (2 MiB). The check runs
/// on the decoded blob so a client cannot smuggle more data than advertised.
const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;

/// Default per-IP token bucket: burst of 60 envelopes, refilled at 1/sec
/// (~60 envelopes per minute).
const DEFAULT_RATE_BURST: f64 = 60.0;
const DEFAULT_RATE_REFILL_PER_SEC: f64 = 1.0;

/// Default per-IP profile token bucket: 5 mutations, refilled at 5/hour.
/// Registration, search and profile lookups all draw from it.
const DEFAULT_PROFILE_RATE_BURST: f64 = 5.0;
const DEFAULT_PROFILE_RATE_REFILL_PER_SEC: f64 = 5.0 / 3600.0;

/// Subdirectory (next to the SQLite database) where avatar blobs are stored
/// as `<sha256>.bin`.
const MEDIA_SUBDIR: &str = "media";

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
    /// the peer ID to the sender's public keys. The optional `display_name`
    /// is public profile data (Signal-style) stored alongside the keys.
    Hello {
        peer_id: String,
        curve25519_key: String,
        ed25519_key: String,
        signature: String,
        display_name: Option<String>,
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
    /// Set the caller's public display name (Signal-style profile name). The
    /// new name replaces the stored one and is visible to everyone in
    /// subsequent pre-key lookups.
    #[serde(rename = "update_profile")]
    UpdateProfile { display_name: String },
    /// Subscribe to presence pushes for `peer_id`: whenever that peer comes
    /// online or goes offline the relay sends this socket a
    /// `ServerMessage::Presence`. One channel per watching peer — re-watching
    /// replaces the previous registration for the same watcher.
    #[serde(rename = "watch_presence")]
    WatchPresence { peer_id: String },
    /// Request the current presence of `peer_id`: the relay replies with a
    /// single `ServerMessage::Presence` carrying whether the peer is online
    /// right now plus (when offline) its last-seen timestamp.
    #[serde(rename = "get_presence")]
    GetPresence { peer_id: String },
    /// Register (or refresh) a signed username binding for the authenticated
    /// peer. `signature` is the base64 Ed25519 signature over the canonical
    /// bytes `username || 0x00 || curve25519_key_raw`; `avatar` is an optional
    /// base64 image blob of at most 2 MiB.
    #[serde(rename = "register_profile")]
    RegisterProfile {
        username: String,
        signature: String,
        display_name: Option<String>,
        avatar: Option<String>,
    },
    /// Prefix-search the public directory by username or peer ID.
    #[serde(rename = "search_users")]
    SearchUsers { query: String, limit: Option<usize> },
    /// Fetch another peer's public profile by its peer ID.
    #[serde(rename = "get_profile")]
    GetProfile { peer_id: String },
    /// Create a group: the authenticated peer becomes the owner and first
    /// member. The Megolm session key is NOT exchanged here — it is shared to
    /// members later over an end-to-end encrypted envelope.
    #[serde(rename = "create_group")]
    CreateGroup { name: String },
    /// Add `peer_id` to a group's roster. Only the owner or an existing
    /// member may add members.
    #[serde(rename = "add_group_member")]
    AddGroupMember { group_id: String, peer_id: String },
    /// Remove the caller from a group's roster.
    #[serde(rename = "leave_group")]
    LeaveGroup { group_id: String },
    /// Request the public metadata and member roster of a group. Membership
    /// is required to see the roster.
    #[serde(rename = "get_group_info")]
    GetGroupInfo { group_id: String },
    /// Send one (already client-encrypted) envelope to every member of a
    /// group except the sender. The relay rewrites `recipient` per member and
    /// routes the opaque payload as usual.
    #[serde(rename = "send_group_message")]
    SendGroupMessage {
        group_id: String,
        envelope: Envelope,
    },
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
    /// The requested peer's pre-key bundle plus its public display name
    /// (`null` when the peer has not set one).
    #[serde(rename = "prekeys")]
    Prekeys {
        bundle: Box<PreKeyBundle>,
        display_name: Option<String>,
    },
    /// Confirmation that the caller's display name was updated.
    #[serde(rename = "profile_updated")]
    ProfileUpdated,
    /// Presence report: the current state of `peer_id`. Sent as a push to
    /// every `watch_presence` subscriber when the peer connects/disconnects
    /// and as the reply to a `get_presence` request. `last_seen` is the peer's
    /// unix-seconds disconnect timestamp; it is `None` while the peer is
    /// online or when it has never been seen.
    #[serde(rename = "presence")]
    Presence {
        peer_id: String,
        online: bool,
        last_seen: Option<i64>,
    },
    /// Confirmation that the caller's username binding was registered.
    #[serde(rename = "profile_registered")]
    ProfileRegistered { username: String },
    /// Reply to `search_users`: matching profiles from the public directory.
    #[serde(rename = "users_search")]
    UsersSearch { results: Vec<SearchResult> },
    /// A peer's public profile (`get_profile` reply).
    Profile {
        username: Option<String>,
        peer_id: String,
        display_name: Option<String>,
        avatar_url: Option<String>,
        curve25519_key: Option<String>,
    },
    /// Confirmation that a group was created (`create_group` reply). The
    /// creator is the first entry in `members`.
    #[serde(rename = "group_created")]
    GroupCreated {
        group_id: String,
        name: String,
        members: Vec<String>,
    },
    /// Confirmation that a member was added to a group (`add_group_member`
    /// reply).
    #[serde(rename = "group_member_added")]
    GroupMemberAdded { group_id: String, peer_id: String },
    /// Confirmation that the caller left a group (`leave_group` reply).
    #[serde(rename = "group_member_left")]
    GroupMemberLeft { group_id: String, peer_id: String },
    /// The public metadata + member roster of a group (`get_group_info`
    /// reply).
    #[serde(rename = "group_info")]
    GroupInfo {
        group_id: String,
        name: String,
        owner_peer_id: String,
        members: Vec<String>,
    },
    /// Protocol error.
    Error { code: String },
}

/// One hit from the public username directory search.
#[derive(Debug, Serialize)]
pub struct SearchResult {
    /// The registered username.
    pub username: String,
    /// The peer's fingerprint.
    pub peer_id: String,
    /// The peer's public display name, if set.
    pub display_name: Option<String>,
    /// URL of the peer's avatar blob, if uploaded.
    pub avatar_url: Option<String>,
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

    /// Build the profile limiter (see [`DEFAULT_PROFILE_RATE_BURST`]).
    ///
    /// Burst/refill are overridable via `WHISPER_PROFILE_RATE_BURST` and
    /// `WHISPER_PROFILE_RATE_REFILL`; when those are unset the generic
    /// `WHISPER_RATE_BURST` / `WHISPER_RATE_REFILL` overrides apply, so a
    /// single smoke-test configuration can bound every bucket.
    fn from_profile_env() -> Self {
        let burst = std::env::var("WHISPER_PROFILE_RATE_BURST")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .or_else(|| {
                std::env::var("WHISPER_RATE_BURST")
                    .ok()
                    .and_then(|v| v.parse::<f64>().ok())
            })
            .unwrap_or(DEFAULT_PROFILE_RATE_BURST);
        let refill = std::env::var("WHISPER_PROFILE_RATE_REFILL")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .or_else(|| {
                std::env::var("WHISPER_RATE_REFILL")
                    .ok()
                    .and_then(|v| v.parse::<f64>().ok())
            })
            .unwrap_or(DEFAULT_PROFILE_RATE_REFILL_PER_SEC);
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

/// One presence subscription. The peer ID lets the relay de-duplicate
/// re-watches (one channel per watching peer) and clean up its own
/// registrations when the watcher disconnects — an `UnboundedSender` alone
/// carries no identity, so it cannot serve either purpose.
#[derive(Clone)]
struct PresenceWatcher {
    /// Peer ID of the subscribing socket.
    peer_id: PeerId,
    /// The watcher's outbound WS channel (its `online` entry).
    tx: Outbound,
}

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
    /// Presence subscriptions: watched peer_id -> its watchers' channels.
    presence_watchers: RwLock<HashMap<PeerId, Vec<PresenceWatcher>>>,
    /// SQLite-backed offline queue of opaque ciphertext blobs.
    store: Store,
    /// Per-IP envelope throughput guard.
    limiter: RateLimiter,
    /// Per-IP guard for profile mutations and directory lookups.
    profile_limiter: RateLimiter,
    /// Directory holding uploaded avatar blobs (`<sha256>.bin`).
    media_dir: PathBuf,
}

impl Relay {
    /// Build a relay backed by the on-disk SQLite store
    /// (`server/data/relay.db`, overridable via `WHISPER_DB_PATH`).
    pub fn new() -> Self {
        let path = std::env::var("WHISPER_DB_PATH").unwrap_or_else(|_| "data/relay.db".into());
        let store = Store::open(&path).expect("failed to open SQLite store");
        let media_dir = Self::resolve_media_dir(&path);
        Self::with_parts(
            store,
            media_dir,
            RateLimiter::from_env(),
            RateLimiter::from_profile_env(),
        )
    }

    /// Build a relay over a pre-opened store (tests use in-memory stores).
    #[cfg(test)]
    fn with_store(store: Store) -> Self {
        Self::with_parts(
            store,
            Self::default_media_dir(),
            RateLimiter::from_env(),
            RateLimiter::from_profile_env(),
        )
    }

    /// Build a relay over a pre-opened store with a deterministic rate
    /// limiter (unit tests need exact bucket sizes).
    #[cfg(test)]
    fn with_limiter(store: Store, burst: f64, refill: f64) -> Self {
        Self::with_parts(
            store,
            Self::default_media_dir(),
            RateLimiter::new(burst, refill),
            RateLimiter::new(burst, refill),
        )
    }

    /// Build a relay over a pre-opened store with a scratch media directory
    /// and a generous profile bucket (unit tests only).
    fn with_parts(
        store: Store,
        media_dir: PathBuf,
        limiter: RateLimiter,
        profile_limiter: RateLimiter,
    ) -> Self {
        Self {
            inner: Arc::new(RelayInner {
                online: RwLock::new(HashMap::new()),
                presence_watchers: RwLock::new(HashMap::new()),
                store,
                limiter,
                profile_limiter,
                media_dir,
            }),
        }
    }

    /// The media directory used when no database path is known: `data/media`
    /// (i.e. `server/data/media` when the relay runs from the server dir).
    #[cfg(test)]
    fn default_media_dir() -> PathBuf {
        PathBuf::from("data").join(MEDIA_SUBDIR)
    }

    /// The media directory lives next to the SQLite database, so switching
    /// the database path (via `WHISPER_DB_PATH`) also relocates uploads.
    fn resolve_media_dir(db_path: &str) -> PathBuf {
        let parent = Path::new(db_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        parent.join(MEDIA_SUBDIR)
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
        //    only ever happens for a verified owner reconnecting. The channel
        //    is cloned because presence watchers registered by this socket
        //    below reuse it.
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        self.inner
            .online
            .write()
            .await
            .insert(peer_id.clone(), out_tx.clone());

        // 2b) Announce the peer is online to everyone watching them. Any peer
        //     that reconnects mid-watch sees a fresh `online: true` push.
        self.broadcast_presence(&peer_id, true).await;

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
                        Ok(ClientMessage::UpdateProfile { display_name }) => {
                            self.update_profile(&peer_id, &ip, &display_name).await;
                        }
                        // Presence: register this socket as a watcher of
                        // `watched`, or answer a one-shot status query. Both
                        // share the `presence:<ip>` rate bucket.
                        Ok(ClientMessage::WatchPresence { peer_id: watched }) => {
                            self.watch_presence(&peer_id, &ip, &watched, out_tx.clone())
                                .await;
                        }
                        Ok(ClientMessage::GetPresence { peer_id: watched }) => {
                            self.get_presence(&peer_id, &ip, &watched).await;
                        }
                        Ok(ClientMessage::RegisterProfile {
                            username,
                            signature,
                            display_name,
                            avatar,
                        }) => {
                            self.register_profile(
                                &peer_id,
                                &ip,
                                &username,
                                &signature,
                                display_name.as_deref(),
                                avatar.as_deref(),
                            )
                            .await;
                        }
                        Ok(ClientMessage::SearchUsers { query, limit }) => {
                            self.search_users(&peer_id, &ip, &query, limit).await;
                        }
                        Ok(ClientMessage::GetProfile { peer_id: target }) => {
                            self.get_profile(&peer_id, &ip, &target).await;
                        }
                        Ok(ClientMessage::CreateGroup { name }) => {
                            self.create_group(&peer_id, &ip, &name).await;
                        }
                        Ok(ClientMessage::AddGroupMember {
                            group_id,
                            peer_id: target,
                        }) => {
                            self.add_group_member(&peer_id, &ip, &group_id, &target)
                                .await;
                        }
                        Ok(ClientMessage::LeaveGroup { group_id }) => {
                            self.leave_group(&peer_id, &ip, &group_id).await;
                        }
                        Ok(ClientMessage::GetGroupInfo { group_id }) => {
                            self.get_group_info(&peer_id, &ip, &group_id).await;
                        }
                        Ok(ClientMessage::SendGroupMessage { group_id, envelope }) => {
                            self.send_group_message(&peer_id, &ip, &group_id, envelope)
                                .await;
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

        // 6) Cleanup: unregister, persist last-seen and notify watchers.
        self.inner.online.write().await.remove(&peer_id);
        let _ = self.inner.store.set_last_seen(&peer_id, unix_now());
        self.broadcast_presence(&peer_id, false).await;
        // Drop this socket's own watch registrations: its channel is dead, so
        // keeping it would only build up stale entries until the next push.
        self.inner
            .presence_watchers
            .write()
            .await
            .iter_mut()
            .for_each(|(_, watchers)| watchers.retain(|w| w.peer_id != peer_id));
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
                        display_name,
                    }) = serde_json::from_str(&text)
                    {
                        let hello = SignedHello {
                            peer_id,
                            curve25519_key,
                            ed25519_key,
                            signature,
                        };
                        return Some(self.validate_hello(&hello, display_name.as_deref()));
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
    ///
    /// The optional `display_name` is stored on the first hello and refreshed
    /// on later hellos, but an absent or invalid name never clears an
    /// existing one. Invalid names are ignored rather than failing the
    /// handshake: a cosmetic profile field must not block an otherwise valid
    /// authenticated connection.
    fn validate_hello(&self, hello: &SignedHello, display_name: Option<&str>) -> HelloOutcome {
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

        if let Some(name) = display_name {
            if !Self::is_valid_display_name(name) {
                tracing::warn!(peer = %hello.peer_id, "ignoring invalid display name on hello");
            } else if let Err(err) = self.inner.store.set_display_name(&hello.peer_id, name) {
                tracing::error!(peer = %hello.peer_id, "failed to persist display name: {err}");
            }
        }
        HelloOutcome::Accepted(hello.peer_id.clone())
    }

    /// Whether `name` is acceptable as a public display name: non-empty, at
    /// most [`MAX_DISPLAY_NAME_CHARS`] Unicode characters and free of control
    /// characters (newlines, tabs, terminal escapes, ...).
    fn is_valid_display_name(name: &str) -> bool {
        !name.is_empty()
            && name.chars().count() <= MAX_DISPLAY_NAME_CHARS
            && !name.chars().any(char::is_control)
    }

    /// Whether `name` is acceptable as a group name: 1-64 Unicode characters
    /// and free of control characters.
    fn is_valid_group_name(name: &str) -> bool {
        !name.is_empty()
            && name.chars().count() <= MAX_GROUP_NAME_CHARS
            && !name.chars().any(char::is_control)
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

        let seq = envelope.seq;

        // Deliver live if possible, otherwise persist the ciphertext blob in
        // SQLite. The per-peer cap and 7-day TTL keep the database bounded.
        self.deliver_one(&envelope).await;

        // Delivery confirmation to the sender: the relay accepted the blob.
        // This is NOT a read receipt — read receipts are end-to-end and
        // travel as regular encrypted envelopes between clients.
        let _ = self
            .send(sender_peer, ServerMessage::Acknowledged { seq })
            .await;
    }

    /// Deliver a single envelope to its recipient: live to an online socket,
    /// otherwise into the SQLite offline queue. Never rate limits and never
    /// acks — shared by 1:1 routing and group fan-out.
    async fn deliver_one(&self, envelope: &Envelope) {
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
            let _ = self.inner.store.enqueue(envelope, unix_now());
            tracing::trace!(recipient = %envelope.recipient, "envelope queued offline");
        }
    }

    /// Consume one token from the per-IP group bucket. On exhaustion, send a
    /// `rate_limited` error to `peer_id` and return `false`.
    async fn take_group_slot(&self, peer_id: &str, ip: &str) -> bool {
        if self.inner.limiter.try_take(&format!("group:{ip}")) {
            true
        } else {
            tracing::warn!(ip = %ip, "group rate limit exceeded");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "rate_limited".into(),
                    },
                )
                .await;
            false
        }
    }

    /// Create a group: generate a unique group ID, persist the public metadata
    /// and register the caller as the owner/first member.
    ///
    /// The Megolm session key is deliberately NOT part of this flow. It is
    /// secret and is shared to members end-to-end over an encrypted envelope
    /// by the desktop client; the relay never sees it.
    async fn create_group(&self, peer_id: &str, ip: &str, name: &str) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }

        if !Self::is_valid_group_name(name) {
            tracing::warn!(peer = %peer_id, "rejecting invalid group name");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "invalid_group_name".into(),
                    },
                )
                .await;
            return;
        }

        let group_id = uuid::Uuid::new_v4().to_string();
        match self
            .inner
            .store
            .create_group(&group_id, name, peer_id, unix_now())
        {
            Ok(()) => {
                let members = self.inner.store.list_group_members(&group_id);
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::GroupCreated {
                            group_id,
                            name: name.to_string(),
                            members,
                        },
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, "failed to persist group: {err}");
            }
        }
    }

    /// Add `target` to a group's roster. Only the owner or an existing member
    /// may add members.
    async fn add_group_member(&self, peer_id: &str, ip: &str, group_id: &str, target: &str) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }

        if self.inner.store.get_group(group_id).is_none() {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "group_not_found".into(),
                    },
                )
                .await;
            return;
        }
        if !self.inner.store.is_group_member(group_id, peer_id) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_a_member".into(),
                    },
                )
                .await;
            return;
        }

        match self
            .inner
            .store
            .add_group_member(group_id, target, unix_now())
        {
            Ok(()) => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::GroupMemberAdded {
                            group_id: group_id.to_string(),
                            peer_id: target.to_string(),
                        },
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, group = %group_id, "failed to add member: {err}");
            }
        }
    }

    /// Remove the caller from a group's roster.
    async fn leave_group(&self, peer_id: &str, ip: &str, group_id: &str) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }

        if self.inner.store.get_group(group_id).is_none() {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "group_not_found".into(),
                    },
                )
                .await;
            return;
        }
        if !self.inner.store.is_group_member(group_id, peer_id) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_a_member".into(),
                    },
                )
                .await;
            return;
        }

        match self.inner.store.remove_group_member(group_id, peer_id) {
            Ok(()) => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::GroupMemberLeft {
                            group_id: group_id.to_string(),
                            peer_id: peer_id.to_string(),
                        },
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, group = %group_id, "failed to remove member: {err}");
            }
        }
    }

    /// Reply with a group's public metadata and member roster. The roster is
    /// only visible to current members.
    async fn get_group_info(&self, peer_id: &str, ip: &str, group_id: &str) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }

        let group = match self.inner.store.get_group(group_id) {
            Some(group) => group,
            None => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "group_not_found".into(),
                        },
                    )
                    .await;
                return;
            }
        };
        if !self.inner.store.is_group_member(group_id, peer_id) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_a_member".into(),
                    },
                )
                .await;
            return;
        }

        let members = self.inner.store.list_group_members(group_id);
        let _ = self
            .send(
                peer_id,
                ServerMessage::GroupInfo {
                    group_id: group.id,
                    name: group.name,
                    owner_peer_id: group.owner_peer_id,
                    members,
                },
            )
            .await;
    }

    /// Fan out one client-encrypted envelope to every group member except the
    /// sender.
    ///
    /// The relay rewrites `recipient` per member and reuses the standard
    /// live/offline delivery path, so the ciphertext stays opaque and members
    /// who are offline get the copy on their next fetch. Group sends draw from
    /// the per-IP `group:<ip>` rate bucket.
    async fn send_group_message(
        &self,
        peer_id: &str,
        ip: &str,
        group_id: &str,
        envelope: Envelope,
    ) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }

        if self.inner.store.get_group(group_id).is_none() {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "group_not_found".into(),
                    },
                )
                .await;
            return;
        }
        if !self.inner.store.is_group_member(group_id, peer_id) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_a_member".into(),
                    },
                )
                .await;
            return;
        }

        // Spoofing guard and size cap mirror the 1:1 routing path.
        if envelope.sender != peer_id {
            tracing::warn!(
                claimed = %envelope.sender,
                authenticated = %peer_id,
                "group envelope sender does not match the authenticated peer"
            );
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "sender_mismatch".into(),
                    },
                )
                .await;
            return;
        }
        if !envelope.within_limits() {
            tracing::warn!(sender = %envelope.sender, group = %group_id, "dropping oversized group envelope");
            return;
        }

        let seq = envelope.seq;
        let members = self.inner.store.list_group_members(group_id);
        for member in &members {
            if member == peer_id {
                continue;
            }
            let mut copy = envelope.clone();
            copy.recipient = member.clone();
            self.deliver_one(&copy).await;
        }

        // Single delivery confirmation to the sender; the fan-out copies share
        // the same client `seq`.
        let _ = self
            .send(peer_id, ServerMessage::Acknowledged { seq })
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
                    let display_name = self.inner.store.get_display_name(target);
                    let _ = self
                        .send(
                            peer_id,
                            ServerMessage::Prekeys {
                                bundle: Box::new(bundle),
                                display_name,
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

    /// Update the caller's public display name.
    ///
    /// Like pre-key traffic, profile updates are rate limited per source IP
    /// under the `profile:<ip>` bucket so a client cannot spam renames.
    /// Invalid names are rejected with `invalid_display_name` and leave any
    /// existing name untouched.
    async fn update_profile(&self, peer_id: &str, ip: &str, display_name: &str) {
        if !self.inner.limiter.try_take(&format!("profile:{ip}")) {
            tracing::warn!(ip = %ip, "profile rate limit exceeded");
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

        if !Self::is_valid_display_name(display_name) {
            tracing::warn!(peer = %peer_id, "rejecting invalid display name");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "invalid_display_name".into(),
                    },
                )
                .await;
            return;
        }

        match self.inner.store.set_display_name(peer_id, display_name) {
            Ok(()) => {
                let _ = self.send(peer_id, ServerMessage::ProfileUpdated).await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, "failed to persist display name: {err}");
            }
        }
    }

    /// Register a signed username binding for the authenticated peer.
    ///
    /// SECURITY: the username is bound to the peer's stored X25519 identity
    /// key by an Ed25519 signature over the canonical bytes
    /// (`username || 0x00 || curve_key_raw`). The signature is re-verified
    /// against the peer's stored public keys before anything is persisted, so
    /// a compromised relay cannot reassign usernames or squat reserved ones.
    ///
    /// The optional `avatar` (base64 image, ≤ 2 MiB) is stored on disk as
    /// `media/<sha256>.bin`; identical content hashes to the same blob, so
    /// re-uploads are idempotent.
    ///
    /// Rate limiting: registration is throttled per source IP under the
    /// `profile:<ip>` bucket (default 5/hour; burst/refill overridable via
    /// `WHISPER_PROFILE_RATE_BURST` / `WHISPER_PROFILE_RATE_REFILL`).
    async fn register_profile(
        &self,
        peer_id: &str,
        ip: &str,
        username: &str,
        signature_b64: &str,
        display_name: Option<&str>,
        avatar_b64: Option<&str>,
    ) {
        // 1) Rate limit profile mutations per source IP.
        if !self
            .inner
            .profile_limiter
            .try_take(&format!("profile:{ip}"))
        {
            tracing::warn!(ip = %ip, "profile rate limit exceeded");
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

        // 2) Username shape validation (charset, length, reserved names).
        if !validate_username(username) {
            tracing::warn!(peer = %peer_id, "rejecting invalid username");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "invalid_username".into(),
                    },
                )
                .await;
            return;
        }

        if let Some(name) = display_name {
            if !Self::is_valid_display_name(name) {
                tracing::warn!(peer = %peer_id, "rejecting invalid display name");
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "invalid_display_name".into(),
                        },
                    )
                    .await;
                return;
            }
        }

        // 3) Decode and size-check the avatar early so nothing is persisted
        //    for a request that will be rejected later. The blob itself is
        //    only written to disk after the signature has been verified.
        let decoded_avatar = match avatar_b64 {
            Some(raw) => match Self::decode_avatar(raw) {
                Ok(bytes) => Some(bytes),
                Err(()) => {
                    let _ = self
                        .send(
                            peer_id,
                            ServerMessage::Error {
                                code: "invalid_avatar".into(),
                            },
                        )
                        .await;
                    return;
                }
            },
            None => None,
        };

        // 4) Signature verification: only the peer that owns the stored curve
        //    key can produce a valid binding. The relay is authenticated by
        //    the signed hello (handle_socket), so the peer's keys are present.
        let (curve_b64, ed_b64) = match self.inner.store.get_user_keys(peer_id) {
            Some(keys) => keys,
            None => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "no_profile".into(),
                        },
                    )
                    .await;
                return;
            }
        };
        let parsed = match Self::parse_binding(&curve_b64, &ed_b64, signature_b64) {
            Some(keys) => keys,
            None => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "bad_signature".into(),
                        },
                    )
                    .await;
                return;
            }
        };
        if !verify_username_signature(username, &parsed.0, &parsed.1, &parsed.2) {
            tracing::warn!(peer = %peer_id, username = %username, "username signature verification failed");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "bad_signature".into(),
                    },
                )
                .await;
            return;
        }

        // 5) Uniqueness + persistence of the username binding.
        let now = unix_now();
        match self
            .inner
            .store
            .register_username(peer_id, username, signature_b64, now)
        {
            Err(crate::store::StoreError::UsernameTaken) => {
                tracing::warn!(peer = %peer_id, username = %username, "username already taken");
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "username_taken".into(),
                        },
                    )
                    .await;
                return;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, "failed to persist username: {err}");
                return;
            }
            Ok(()) => {}
        }

        // 6) Persist the profile extras (display name + avatar).
        if let Some(name) = display_name {
            if let Err(err) = self.inner.store.set_display_name(peer_id, name) {
                tracing::error!(peer = %peer_id, "failed to persist display name: {err}");
            }
        }
        if let Some(bytes) = decoded_avatar {
            match Self::store_avatar(&self.inner.media_dir, &bytes) {
                Ok(hash) => {
                    if let Err(err) = self.inner.store.set_avatar_hash(peer_id, &hash) {
                        tracing::error!(peer = %peer_id, "failed to persist avatar hash: {err}");
                    }
                }
                Err(()) => {
                    tracing::error!(peer = %peer_id, "failed to write avatar blob");
                }
            }
        }

        let _ = self
            .send(
                peer_id,
                ServerMessage::ProfileRegistered {
                    username: username.to_string(),
                },
            )
            .await;
    }

    /// Prefix-search the public directory by username or peer ID.
    ///
    /// Results are capped at 25 entries (default 10). Like profile
    /// registration, search consumes the `profile:<ip>` rate bucket.
    async fn search_users(&self, peer_id: &str, ip: &str, query: &str, limit: Option<usize>) {
        if !self
            .inner
            .profile_limiter
            .try_take(&format!("profile:{ip}"))
        {
            tracing::warn!(ip = %ip, "profile rate limit exceeded");
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

        let query = query.trim();
        let limit = limit.unwrap_or(10).clamp(1, 25);
        let results = if query.is_empty() {
            Vec::new()
        } else {
            self.inner
                .store
                .search_users(query, limit)
                .into_iter()
                .map(|p| SearchResult {
                    username: p.username.unwrap_or_default(),
                    peer_id: p.peer_id,
                    display_name: p.display_name,
                    avatar_url: Self::avatar_url(p.avatar_hash.as_deref()),
                })
                .collect()
        };
        let _ = self
            .send(peer_id, ServerMessage::UsersSearch { results })
            .await;
    }

    /// Fetch another peer's public profile by peer ID, or answer `no_profile`
    /// when the peer has never been seen by the relay. Directory lookups are
    /// rate limited per source IP like every other profile operation.
    async fn get_profile(&self, peer_id: &str, ip: &str, target: &str) {
        if !self
            .inner
            .profile_limiter
            .try_take(&format!("profile:{ip}"))
        {
            tracing::warn!(ip = %ip, "profile rate limit exceeded");
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

        match self.inner.store.get_profile(target) {
            Some(profile) => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Profile {
                            username: profile.username,
                            peer_id: profile.peer_id,
                            display_name: profile.display_name,
                            avatar_url: Self::avatar_url(profile.avatar_hash.as_deref()),
                            curve25519_key: profile.curve25519_key,
                        },
                    )
                    .await;
            }
            None => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "no_profile".into(),
                        },
                    )
                    .await;
            }
        }
    }

    /// Map a stored avatar hash to the public URL the relay serves it under.
    fn avatar_url(avatar_hash: Option<&str>) -> Option<String> {
        avatar_hash.map(|h| format!("/media/{h}"))
    }

    /// Decode a base64 avatar blob and enforce the size bound. Returns `Err`
    /// when the input is not valid base64, empty or larger than
    /// [`MAX_AVATAR_BYTES`].
    fn decode_avatar(raw: &str) -> Result<Vec<u8>, ()> {
        let bytes = STANDARD.decode(raw).map_err(|_| ())?;
        if bytes.is_empty() || bytes.len() > MAX_AVATAR_BYTES {
            return Err(());
        }
        Ok(bytes)
    }

    /// Write an avatar blob to `media/<sha256>.bin` and return the hex SHA-256
    /// used as its storage key. Content-addressed: identical blobs share one
    /// file, so re-uploads are idempotent.
    fn store_avatar(media_dir: &Path, bytes: &[u8]) -> Result<String, ()> {
        let digest = Sha256::digest(bytes);
        let hash = Self::hex_encode(&digest);
        let path = media_dir.join(format!("{hash}.bin"));
        if let Some(parent) = path.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                tracing::error!(path = %parent.display(), "failed to create media dir: {err}");
                return Err(());
            }
        }
        if let Err(err) = std::fs::write(&path, bytes) {
            tracing::error!(path = %path.display(), "failed to write avatar blob: {err}");
            return Err(());
        }
        Ok(hash)
    }

    /// Lowercase hex encoding of a byte slice (SHA-256 digests, peer IDs).
    fn hex_encode(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
        out
    }

    /// Parse the stored curve/ed25519 keys and the submitted signature into
    /// verifiable vodozemac types. `None` when any piece is malformed.
    fn parse_binding(
        curve_b64: &str,
        ed_b64: &str,
        sig_b64: &str,
    ) -> Option<(Curve25519PublicKey, Ed25519PublicKey, Ed25519Signature)> {
        let curve = Curve25519PublicKey::from_base64(curve_b64).ok()?;
        let ed = Ed25519PublicKey::from_base64(ed_b64).ok()?;
        let sig = Ed25519Signature::from_base64(sig_b64).ok()?;
        Some((curve, ed, sig))
    }

    /// Resolve the on-disk path of a media blob. The `/media/{hash}` endpoint
    /// in main.rs uses this; the caller is responsible for validating `hash`.
    pub fn media_path(&self, hash: &str) -> PathBuf {
        self.inner.media_dir.join(format!("{hash}.bin"))
    }

    /// Register `watcher`'s socket as a presence subscriber of `watched`.
    ///
    /// Re-watching the same peer replaces the watcher's previous registration,
    /// so a peer can never hold two live channels in one watched list (and
    /// reconnecting cannot duplicate pushes). Watching a peer you are already
    /// watching is a no-op apart from the replacement.
    ///
    /// Presence traffic (both this and `get_presence`) is rate limited per
    /// source IP under the `presence:<ip>` bucket.
    async fn watch_presence(&self, watcher: &str, ip: &str, watched: &str, tx: Outbound) {
        if !self.inner.limiter.try_take(&format!("presence:{ip}")) {
            tracing::warn!(ip = %ip, "presence rate limit exceeded");
            let _ = self
                .send(
                    watcher,
                    ServerMessage::Error {
                        code: "rate_limited".into(),
                    },
                )
                .await;
            return;
        }

        let mut watchers = self.inner.presence_watchers.write().await;
        let list = watchers.entry(watched.to_string()).or_default();
        list.retain(|w| w.peer_id != watcher);
        list.push(PresenceWatcher {
            peer_id: watcher.to_string(),
            tx,
        });
    }

    /// Answer a one-shot presence query for `target`: whether the peer is
    /// online right now, plus its stored last-seen timestamp when offline.
    /// Unknown peers report `online: false` with `last_seen: null`.
    async fn get_presence(&self, requester: &str, ip: &str, target: &str) {
        if !self.inner.limiter.try_take(&format!("presence:{ip}")) {
            tracing::warn!(ip = %ip, "presence rate limit exceeded");
            let _ = self
                .send(
                    requester,
                    ServerMessage::Error {
                        code: "rate_limited".into(),
                    },
                )
                .await;
            return;
        }

        let online = self.inner.online.read().await.contains_key(target);
        let last_seen = if online {
            None
        } else {
            self.inner.store.get_last_seen(target)
        };
        let _ = self
            .send(
                requester,
                ServerMessage::Presence {
                    peer_id: target.to_string(),
                    online,
                    last_seen,
                },
            )
            .await;
    }

    /// Push a presence change for `peer_id` to every registered watcher.
    ///
    /// Watchers whose channel is gone (closed socket, or the peer itself
    /// disconnected) are dropped in the same pass, so dead subscriptions
    /// cannot accumulate. The `presence_watchers` lock is held while sending;
    /// sends into unbounded channels never block, so this is safe.
    async fn broadcast_presence(&self, peer_id: &str, online: bool) {
        let last_seen = if online {
            None
        } else {
            self.inner.store.get_last_seen(peer_id)
        };
        let text = serde_json::to_string(&ServerMessage::Presence {
            peer_id: peer_id.to_string(),
            online,
            last_seen,
        })
        .ok();

        let mut watchers = self.inner.presence_watchers.write().await;
        if let Some(list) = watchers.get_mut(peer_id) {
            match text {
                Some(text) => list.retain(|w| w.tx.send(WsMessage::Text(text.clone())).is_ok()),
                None => list.clear(),
            }
            if list.is_empty() {
                watchers.remove(peer_id);
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

        let outcome = relay.validate_hello(&hello, None);
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

        let outcome = relay.validate_hello(&hello, None);
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
        relay.validate_hello(&hello, None);

        // Same curve key (hence same peer ID) but a different Ed25519 key:
        // the signature verifies, yet the identity conflicts with the
        // already registered one.
        let other = Identity::new();
        let mut conflict = hello.clone();
        conflict.ed25519_key = other.ed25519_key().to_base64();
        conflict.signature = other.sign(conflict.peer_id.as_bytes()).to_base64();

        let outcome = relay.validate_hello(&conflict, None);
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

    // -- Display names ------------------------------------------------------

    #[test]
    fn validate_hello_stores_display_name() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let identity = Identity::new();
        let hello = identity.signed_hello();

        let outcome = relay.validate_hello(&hello, Some("Test Alice"));
        assert!(matches!(outcome, HelloOutcome::Accepted(_)));
        assert_eq!(
            relay
                .inner
                .store
                .get_display_name(&hello.peer_id)
                .as_deref(),
            Some("Test Alice")
        );
    }

    #[test]
    fn validate_hello_updates_display_name_on_later_hellos() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let identity = Identity::new();
        let hello = identity.signed_hello();

        relay.validate_hello(&hello, Some("First Name"));
        relay.validate_hello(&hello, Some("New Name"));
        assert_eq!(
            relay
                .inner
                .store
                .get_display_name(&hello.peer_id)
                .as_deref(),
            Some("New Name")
        );
    }

    #[test]
    fn validate_hello_keeps_existing_name_when_absent() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let identity = Identity::new();
        let hello = identity.signed_hello();

        relay.validate_hello(&hello, Some("First Name"));
        relay.validate_hello(&hello, None);
        assert_eq!(
            relay
                .inner
                .store
                .get_display_name(&hello.peer_id)
                .as_deref(),
            Some("First Name"),
            "a hello without a name must not clear the stored one"
        );
    }

    #[test]
    fn validate_hello_ignores_invalid_display_name() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let identity = Identity::new();
        let hello = identity.signed_hello();

        let outcome = relay.validate_hello(&hello, Some("bad\nname"));
        assert!(
            matches!(outcome, HelloOutcome::Accepted(_)),
            "an invalid display name must not fail the handshake"
        );
        assert_eq!(
            relay.inner.store.get_display_name(&hello.peer_id),
            None,
            "an invalid display name must not be stored"
        );
    }

    #[tokio::test]
    async fn update_profile_persists_display_name() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let peer_id = "peer-profile".to_string();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert(peer_id.clone(), out_tx);

        relay
            .update_profile(&peer_id, "127.0.0.1", "Alice Prime")
            .await;
        assert_eq!(
            relay.inner.store.get_display_name(&peer_id).as_deref(),
            Some("Alice Prime")
        );
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["type"].as_str(), Some("profile_updated"));
    }

    #[tokio::test]
    async fn update_profile_rejects_invalid_display_name() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let peer_id = "peer-profile".to_string();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert(peer_id.clone(), out_tx);

        let too_long = "x".repeat(MAX_DISPLAY_NAME_CHARS + 1);
        relay.update_profile(&peer_id, "127.0.0.1", &too_long).await;
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("invalid_display_name"));
        assert_eq!(
            relay.inner.store.get_display_name(&peer_id),
            None,
            "a rejected name must not touch the stored profile"
        );
    }

    #[tokio::test]
    async fn update_profile_is_rate_limited() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 1.0, 0.0);
        let peer_id = "peer-profile".to_string();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert(peer_id.clone(), out_tx);

        relay.update_profile(&peer_id, "10.0.0.1", "First").await;
        relay.update_profile(&peer_id, "10.0.0.1", "Second").await;

        let first = read_reply(&mut out_rx);
        assert_eq!(first["type"].as_str(), Some("profile_updated"));
        let second = read_reply(&mut out_rx);
        assert_eq!(second["type"].as_str(), Some("error"));
        assert_eq!(second["code"].as_str(), Some("rate_limited"));
        assert_eq!(
            relay.inner.store.get_display_name(&peer_id).as_deref(),
            Some("First"),
            "the rejected rename must not overwrite the accepted one"
        );
    }

    #[tokio::test]
    async fn fetch_prekeys_reply_includes_display_name() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let mut owner = Identity::new();
        let owner_id = owner.peer_id();
        relay
            .inner
            .store
            .register_user_with_keys(
                &owner_id,
                &owner.curve25519_key().to_base64(),
                &owner.ed25519_key().to_base64(),
                unix_now(),
            )
            .unwrap();
        relay
            .inner
            .store
            .set_display_name(&owner_id, "Test Alice")
            .unwrap();
        relay
            .publish_prekeys(&owner_id, "127.0.0.1", owner.pre_key_bundle(2))
            .await;

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert("requester".into(), out_tx);

        relay
            .fetch_prekeys("requester", "127.0.0.1", &owner_id)
            .await;
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["type"].as_str(), Some("prekeys"));
        assert_eq!(reply["display_name"].as_str(), Some("Test Alice"));
    }

    // -- Presence (watch / get / last seen) ----------------------------------

    #[tokio::test]
    async fn watcher_receives_online_and_offline_presence_pushes() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let watched = "bob".to_string();
        let (watch_tx, mut watch_rx) = mpsc::unbounded_channel::<WsMessage>();

        relay
            .watch_presence("alice", "127.0.0.1", &watched, watch_tx)
            .await;

        // Bob comes online: the watcher must get an `online: true` push.
        let (bob_tx, _bob_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert(watched.clone(), bob_tx);
        relay.broadcast_presence(&watched, true).await;
        let reply = read_reply(&mut watch_rx);
        assert_eq!(reply["type"].as_str(), Some("presence"));
        assert_eq!(reply["peer_id"].as_str(), Some("bob"));
        assert_eq!(reply["online"].as_bool(), Some(true));
        assert!(
            reply["last_seen"].is_null(),
            "online pushes carry no last_seen"
        );

        // Bob goes offline: last_seen must be included in the push.
        relay.inner.online.write().await.remove(&watched);
        relay
            .inner
            .store
            .set_last_seen(&watched, 1_700_000_000)
            .unwrap();
        relay.broadcast_presence(&watched, false).await;
        let reply = read_reply(&mut watch_rx);
        assert_eq!(reply["type"].as_str(), Some("presence"));
        assert_eq!(reply["online"].as_bool(), Some(false));
        assert_eq!(reply["last_seen"].as_i64(), Some(1_700_000_000));
    }

    #[tokio::test]
    async fn watch_presence_replaces_previous_channel_for_same_peer() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let watched = "bob".to_string();
        let (tx1, mut rx1) = mpsc::unbounded_channel::<WsMessage>();
        let (tx2, mut rx2) = mpsc::unbounded_channel::<WsMessage>();

        // Alice watches bob, then re-watches bob on a fresh socket: the old
        // registration must be replaced, not appended.
        relay
            .watch_presence("alice", "127.0.0.1", &watched, tx1)
            .await;
        relay
            .watch_presence("alice", "127.0.0.1", &watched, tx2)
            .await;

        relay.broadcast_presence(&watched, true).await;
        let reply = read_reply(&mut rx2);
        assert_eq!(reply["type"].as_str(), Some("presence"));
        assert_eq!(reply["online"].as_bool(), Some(true));
        assert!(
            rx1.try_recv().is_err(),
            "the replaced channel must not receive pushes"
        );
    }

    #[tokio::test]
    async fn get_presence_reports_online_status_and_last_seen() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert("requester".into(), out_tx);

        // Unknown peer: offline, no last_seen.
        relay.get_presence("requester", "127.0.0.1", "ghost").await;
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["type"].as_str(), Some("presence"));
        assert_eq!(reply["peer_id"].as_str(), Some("ghost"));
        assert_eq!(reply["online"].as_bool(), Some(false));
        assert!(reply["last_seen"].is_null());

        // Offline peer with a stored last_seen.
        relay
            .inner
            .store
            .set_last_seen("bob", 1_700_000_000)
            .unwrap();
        relay.get_presence("requester", "127.0.0.1", "bob").await;
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["online"].as_bool(), Some(false));
        assert_eq!(reply["last_seen"].as_i64(), Some(1_700_000_000));

        // Online peer reports online:true regardless of the stored value.
        let (bob_tx, _bob_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert("bob".into(), bob_tx);
        relay.get_presence("requester", "127.0.0.1", "bob").await;
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["online"].as_bool(), Some(true));
        assert!(reply["last_seen"].is_null());
    }

    #[tokio::test]
    async fn presence_is_rate_limited_per_ip() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 1.0, 0.0);
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert("requester".into(), out_tx);

        relay.get_presence("requester", "10.0.0.1", "bob").await;
        let first = read_reply(&mut out_rx);
        assert_eq!(first["type"].as_str(), Some("presence"));

        relay.get_presence("requester", "10.0.0.1", "bob").await;
        let second = read_reply(&mut out_rx);
        assert_eq!(second["type"].as_str(), Some("error"));
        assert_eq!(second["code"].as_str(), Some("rate_limited"));

        // A different IP has its own bucket and is not blocked.
        relay.get_presence("requester", "10.0.0.2", "bob").await;
        let third = read_reply(&mut out_rx);
        assert_eq!(third["type"].as_str(), Some("presence"));
    }

    #[tokio::test]
    async fn disconnect_records_last_seen_and_pushes_offline() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let watched = "bob".to_string();
        let (watch_tx, mut watch_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .watch_presence("alice", "127.0.0.1", &watched, watch_tx)
            .await;

        // Simulate bob's online -> disconnect sequence as handle_socket does:
        // unregister, persist last_seen, broadcast offline.
        let (bob_tx, _bob_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert(watched.clone(), bob_tx);
        relay.inner.online.write().await.remove(&watched);
        let _ = relay.inner.store.set_last_seen(&watched, unix_now());
        relay.broadcast_presence(&watched, false).await;

        let reply = read_reply(&mut watch_rx);
        assert_eq!(reply["online"].as_bool(), Some(false));
        let last_seen = reply["last_seen"].as_i64().expect("last_seen must be set");
        assert!(
            last_seen <= unix_now() && last_seen > unix_now() - 60,
            "last_seen must be near now"
        );
    }

    // -- Usernames & profiles ------------------------------------------------

    /// Register an identity's keys in the store and wire an outbound channel
    /// so the peer can receive relay replies.
    async fn online_peer(relay: &Relay, identity: &Identity) -> mpsc::UnboundedReceiver<WsMessage> {
        relay
            .inner
            .store
            .register_user_with_keys(
                &identity.peer_id(),
                &identity.curve25519_key().to_base64(),
                &identity.ed25519_key().to_base64(),
                unix_now(),
            )
            .unwrap();
        let (out_tx, out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert(identity.peer_id(), out_tx);
        out_rx
    }

    fn sign_username(identity: &Identity, username: &str) -> String {
        e2ee_core::sign_username(identity, username).to_base64()
    }

    #[tokio::test]
    async fn register_profile_then_get_profile_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                Some("Test Alice"),
                None,
            )
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("profile_registered"));
        assert_eq!(reply["username"].as_str(), Some("alice"));

        relay
            .register_profile(
                &bob.peer_id(),
                "127.0.0.1",
                "bob",
                &sign_username(&bob, "bob"),
                None,
                None,
            )
            .await;
        assert_eq!(
            read_reply(&mut bob_rx)["type"].as_str(),
            Some("profile_registered")
        );

        // Alice looks Bob up by peer ID and sees his full public profile.
        relay
            .get_profile(&alice.peer_id(), "127.0.0.1", &bob.peer_id())
            .await;
        let profile = read_reply(&mut alice_rx);
        assert_eq!(profile["type"].as_str(), Some("profile"));
        assert_eq!(profile["username"].as_str(), Some("bob"));
        assert_eq!(profile["peer_id"].as_str(), Some(bob.peer_id().as_str()));
        assert_eq!(
            profile["curve25519_key"].as_str(),
            Some(bob.curve25519_key().to_base64().as_str())
        );
        assert_eq!(profile["avatar_url"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn register_profile_rejects_username_signed_for_another_name() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        // The signature binds "bob", not the claimed "alice".
        let wrong = sign_username(&alice, "bob");
        relay
            .register_profile(&alice.peer_id(), "127.0.0.1", "alice", &wrong, None, None)
            .await;
        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("bad_signature"));
    }

    #[tokio::test]
    async fn register_profile_rejects_signature_from_another_key() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mallory = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        // Mallory signs for her own username; Alice claims it.
        let forged = sign_username(&mallory, "alice");
        relay
            .register_profile(&alice.peer_id(), "127.0.0.1", "alice", &forged, None, None)
            .await;
        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("bad_signature"));
        assert_eq!(
            relay
                .inner
                .store
                .get_profile(&alice.peer_id())
                .unwrap()
                .username,
            None
        );
    }

    #[tokio::test]
    async fn register_profile_rejects_reserved_username() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "admin",
                &sign_username(&alice, "admin"),
                None,
                None,
            )
            .await;
        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("invalid_username"));
    }

    #[tokio::test]
    async fn register_profile_rejects_invalid_username() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        // Uppercase is not part of the `[a-z0-9_]` charset.
        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "Alice",
                &sign_username(&alice, "Alice"),
                None,
                None,
            )
            .await;
        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("invalid_username"));
    }

    #[tokio::test]
    async fn register_profile_rejects_duplicate_username() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                None,
                None,
            )
            .await;
        assert_eq!(
            read_reply(&mut alice_rx)["type"].as_str(),
            Some("profile_registered")
        );

        // Bob's signature is valid — the uniqueness check is what rejects him.
        relay
            .register_profile(
                &bob.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&bob, "alice"),
                None,
                None,
            )
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("username_taken"));
        assert_eq!(
            relay
                .inner
                .store
                .get_profile(&bob.peer_id())
                .unwrap()
                .username,
            None,
            "the rejected registration must not be persisted"
        );
    }

    #[tokio::test]
    async fn register_profile_is_rate_limited() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 1.0, 0.0);
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        relay
            .register_profile(
                &alice.peer_id(),
                "10.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                None,
                None,
            )
            .await;
        assert_eq!(
            read_reply(&mut rx)["type"].as_str(),
            Some("profile_registered")
        );

        relay
            .register_profile(
                &alice.peer_id(),
                "10.0.0.1",
                "bob",
                &sign_username(&alice, "bob"),
                None,
                None,
            )
            .await;
        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("rate_limited"));
    }

    #[tokio::test]
    async fn register_profile_stores_avatar_blob() {
        let store = Store::open_in_memory().unwrap();
        let dir =
            std::env::temp_dir().join(format!("whisper-relay-media-test-{}", uuid::Uuid::new_v4()));
        let relay = Relay::with_parts(
            store,
            dir.clone(),
            RateLimiter::new(100.0, 0.0),
            RateLimiter::new(100.0, 0.0),
        );
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        let png: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d,
        ];
        let encoded = STANDARD.encode(png);

        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                None,
                Some(&encoded),
            )
            .await;
        assert_eq!(
            read_reply(&mut rx)["type"].as_str(),
            Some("profile_registered")
        );

        let digest = Sha256::digest(png);
        let hash = Relay::hex_encode(&digest);
        assert!(
            dir.join(format!("{hash}.bin")).exists(),
            "the avatar blob must be written to the media directory"
        );
        let profile = relay.inner.store.get_profile(&alice.peer_id()).unwrap();
        assert_eq!(profile.avatar_hash.as_deref(), Some(hash.as_str()));
        assert_eq!(
            relay
                .inner
                .store
                .get_avatar_hash(&alice.peer_id())
                .as_deref(),
            Some(hash.as_str())
        );

        // A re-upload of identical content is idempotent (same hash, one file).
        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                None,
                Some(&encoded),
            )
            .await;
        assert_eq!(
            read_reply(&mut rx)["type"].as_str(),
            Some("profile_registered")
        );
        assert_eq!(
            relay
                .inner
                .store
                .get_avatar_hash(&alice.peer_id())
                .as_deref(),
            Some(hash.as_str())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn register_profile_rejects_oversized_avatar() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        let big = vec![0x00u8; MAX_AVATAR_BYTES + 1];
        let encoded = STANDARD.encode(&big);
        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                None,
                Some(&encoded),
            )
            .await;
        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("invalid_avatar"));
        assert_eq!(
            relay
                .inner
                .store
                .get_profile(&alice.peer_id())
                .unwrap()
                .username,
            None,
            "an oversized avatar must abort the whole registration"
        );
    }

    #[tokio::test]
    async fn search_users_returns_matching_profiles() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                None,
                None,
            )
            .await;
        relay
            .register_profile(
                &bob.peer_id(),
                "127.0.0.1",
                "bob",
                &sign_username(&bob, "bob"),
                None,
                None,
            )
            .await;
        read_reply(&mut alice_rx);
        read_reply(&mut bob_rx);

        // Bob searches by username prefix.
        relay
            .search_users(&bob.peer_id(), "127.0.0.1", "ali", Some(10))
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("users_search"));
        let results = reply["results"]
            .as_array()
            .expect("results must be an array");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["username"].as_str(), Some("alice"));
        assert_eq!(
            results[0]["peer_id"].as_str(),
            Some(alice.peer_id().as_str())
        );

        // Alice searches by peer-ID prefix and finds Bob.
        let prefix = &bob.peer_id()[..8];
        relay
            .search_users(&alice.peer_id(), "127.0.0.1", prefix, None)
            .await;
        let reply = read_reply(&mut alice_rx);
        let results = reply["results"]
            .as_array()
            .expect("results must be an array");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["username"].as_str(), Some("bob"));
    }

    #[tokio::test]
    async fn get_profile_returns_no_profile_for_unknown_peer() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        let ghost = "000000000000000000000000";
        relay
            .get_profile(&alice.peer_id(), "127.0.0.1", ghost)
            .await;
        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("no_profile"));
    }

    // -- Groups ------------------------------------------------------------------

    #[tokio::test]
    async fn create_group_replies_with_owner_membership() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Ghost Squad")
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("group_created"));
        let group_id = reply["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();
        assert_eq!(reply["name"].as_str(), Some("Ghost Squad"));
        let members = reply["members"]
            .as_array()
            .expect("members must be an array");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].as_str(), Some(alice.peer_id().as_str()));
        assert!(
            relay
                .inner
                .store
                .is_group_member(&group_id, &alice.peer_id()),
            "the owner must be a member"
        );
    }

    #[tokio::test]
    async fn create_group_rejects_invalid_name() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;

        relay.create_group(&alice.peer_id(), "127.0.0.1", "").await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("invalid_group_name"));

        let too_long = "x".repeat(MAX_GROUP_NAME_CHARS + 1);
        relay
            .create_group(&alice.peer_id(), "127.0.0.1", &too_long)
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("invalid_group_name"));
    }

    #[tokio::test]
    async fn add_group_member_and_get_group_info_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();

        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("group_member_added"));
        assert_eq!(reply["group_id"].as_str(), Some(group_id.as_str()));
        assert_eq!(reply["peer_id"].as_str(), Some(bob.peer_id().as_str()));

        relay
            .get_group_info(&alice.peer_id(), "127.0.0.1", &group_id)
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("group_info"));
        assert_eq!(reply["name"].as_str(), Some("Squad"));
        assert_eq!(
            reply["owner_peer_id"].as_str(),
            Some(alice.peer_id().as_str())
        );
        let members = reply["members"]
            .as_array()
            .expect("members must be an array");
        let ids: Vec<&str> = members.iter().filter_map(|m| m.as_str()).collect();
        assert!(ids.contains(&alice.peer_id().as_str()));
        assert!(ids.contains(&bob.peer_id().as_str()));

        // Bob (a member) may also read the info.
        relay
            .get_group_info(&bob.peer_id(), "127.0.0.1", &group_id)
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("group_info"));
    }

    #[tokio::test]
    async fn add_group_member_rejects_non_member_and_unknown_group() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut carol_rx = online_peer(&relay, &carol).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();

        // Carol is not a member and cannot add anyone.
        relay
            .add_group_member(&carol.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        let reply = read_reply(&mut carol_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_a_member"));

        // An unknown group id.
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", "ghost", &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("group_not_found"));
    }

    #[tokio::test]
    async fn send_group_message_fans_out_to_members_except_sender() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;
        let mut carol_rx = online_peer(&relay, &carol).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        read_reply(&mut alice_rx);
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &carol.peer_id())
            .await;
        read_reply(&mut alice_rx);

        relay
            .send_group_message(
                &alice.peer_id(),
                "127.0.0.1",
                &group_id,
                env(&alice.peer_id(), "ignored", 42),
            )
            .await;

        // Alice gets a single ack (and no envelope copy for herself).
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("ack"));
        assert_eq!(reply["seq"].as_u64(), Some(42));
        assert!(
            alice_rx.try_recv().is_err(),
            "the sender must not receive its own group copy"
        );

        // Bob and carol each get a copy with the recipient rewritten.
        let bob_msg = read_reply(&mut bob_rx);
        assert_eq!(bob_msg["type"].as_str(), Some("envelope"));
        assert_eq!(
            bob_msg["envelope"]["recipient"].as_str(),
            Some(bob.peer_id().as_str())
        );
        assert_eq!(
            bob_msg["envelope"]["sender"].as_str(),
            Some(alice.peer_id().as_str())
        );
        let carol_msg = read_reply(&mut carol_rx);
        assert_eq!(carol_msg["type"].as_str(), Some("envelope"));
        assert_eq!(
            carol_msg["envelope"]["recipient"].as_str(),
            Some(carol.peer_id().as_str())
        );
    }

    #[tokio::test]
    async fn send_group_message_queues_for_offline_members() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new(); // registered in the store but never online
        relay
            .inner
            .store
            .register_user_with_keys(
                &bob.peer_id(),
                &bob.curve25519_key().to_base64(),
                &bob.ed25519_key().to_base64(),
                unix_now(),
            )
            .unwrap();
        let mut alice_rx = online_peer(&relay, &alice).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        read_reply(&mut alice_rx);

        relay
            .send_group_message(
                &alice.peer_id(),
                "127.0.0.1",
                &group_id,
                env(&alice.peer_id(), "any", 7),
            )
            .await;
        assert_eq!(read_reply(&mut alice_rx)["type"].as_str(), Some("ack"));

        // Bob is offline, so his copy lands in the SQLite queue for him.
        assert_eq!(relay.inner.store.count_for(&bob.peer_id()), 1);
        let queued = relay.inner.store.list_for(&bob.peer_id(), unix_now());
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].recipient, bob.peer_id());
        assert_eq!(queued[0].sender, alice.peer_id());
    }

    #[tokio::test]
    async fn send_group_message_rejects_non_member() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut carol_rx = online_peer(&relay, &carol).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();

        relay
            .send_group_message(
                &carol.peer_id(),
                "127.0.0.1",
                &group_id,
                env(&carol.peer_id(), "x", 1),
            )
            .await;
        let reply = read_reply(&mut carol_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_a_member"));

        // Unknown group.
        relay
            .send_group_message(
                &alice.peer_id(),
                "127.0.0.1",
                "ghost",
                env(&alice.peer_id(), "x", 2),
            )
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("group_not_found"));
    }

    #[tokio::test]
    async fn send_group_message_rejects_spoofed_sender() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        read_reply(&mut alice_rx);

        // Alice claims to be Bob inside a group envelope.
        relay
            .send_group_message(
                &alice.peer_id(),
                "127.0.0.1",
                &group_id,
                env(&bob.peer_id(), "spoofed", 99),
            )
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("sender_mismatch"));
        assert!(
            bob_rx.try_recv().is_err(),
            "a spoofed group envelope must not be delivered"
        );
    }

    #[tokio::test]
    async fn leave_group_removes_member_and_revokes_send() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        read_reply(&mut alice_rx);

        relay
            .leave_group(&bob.peer_id(), "127.0.0.1", &group_id)
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("group_member_left"));
        assert_eq!(reply["group_id"].as_str(), Some(group_id.as_str()));
        assert!(!relay.inner.store.is_group_member(&group_id, &bob.peer_id()));

        // Bob can no longer send to the group.
        relay
            .send_group_message(
                &bob.peer_id(),
                "127.0.0.1",
                &group_id,
                env(&bob.peer_id(), "x", 1),
            )
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_a_member"));
    }

    #[tokio::test]
    async fn get_group_info_requires_membership() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut carol_rx = online_peer(&relay, &carol).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();

        relay
            .get_group_info(&carol.peer_id(), "127.0.0.1", &group_id)
            .await;
        let reply = read_reply(&mut carol_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_a_member"));
    }

    #[tokio::test]
    async fn group_operations_are_rate_limited_per_ip() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 1.0, 0.0);
        let alice = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;

        relay
            .create_group(&alice.peer_id(), "10.0.0.1", "First")
            .await;
        assert_eq!(
            read_reply(&mut alice_rx)["type"].as_str(),
            Some("group_created")
        );

        relay
            .create_group(&alice.peer_id(), "10.0.0.1", "Second")
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("rate_limited"));

        // A different IP has its own group bucket.
        relay
            .create_group(&alice.peer_id(), "10.0.0.2", "Third")
            .await;
        assert_eq!(
            read_reply(&mut alice_rx)["type"].as_str(),
            Some("group_created")
        );
    }

    /// Read the single text reply queued for a peer and parse it as JSON.
    fn read_reply(rx: &mut mpsc::UnboundedReceiver<WsMessage>) -> serde_json::Value {
        let msg = rx.try_recv().expect("a reply must be queued");
        let text = match msg {
            WsMessage::Text(t) => t,
            _ => panic!("expected a text reply"),
        };
        serde_json::from_str(&text).expect("reply must be valid JSON")
    }
}
