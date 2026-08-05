//! WebSocket relay client for the desktop app.
//!
//! Connects to the zero-knowledge relay, authenticates with a signed hello and
//! pumps end-to-end encrypted [`e2ee_core::Envelope`]s between peers. The relay
//! never sees plaintext: every envelope payload is opaque ciphertext.
//!
//! The client keeps all session state in memory (SQLCipher persistence is a
//! later phase). Identity and Double Ratchet sessions are stored in shared,
//! thread-safe state so the WebSocket inbound task and the Tauri commands can
//! both touch them.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use e2ee_core::{
    ChatSession, Envelope, EnvelopeContent, Handshake, Identity, Message, PreKeyBundle, ReceiptKind,
};
use vodozemac::olm::OlmMessage;

/// Default relay endpoint; override with the `WHISPER_RELAY_URL` env var.
const DEFAULT_RELAY_URL: &str = "ws://127.0.0.1:8080/ws";

/// How long to wait for a pre-key bundle after requesting one.
const PREKEY_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Number of fresh one-time pre-keys generated per publish batch.
const PREKEY_BATCH_SIZE: usize = 5;

/// Text of the greeting message sent when a session is established.
const FIRST_MESSAGE_TEXT: &str = "👋 Connected via Whisper";

/// Maximum length of a public display name, in Unicode characters. Mirrors the
/// server's limit so an invalid name is rejected locally before a round trip.
const MAX_DISPLAY_NAME_CHARS: usize = 64;

/// How long a typing indicator stays "on" after the last `typing` receipt
/// before the client auto-emits `is_typing: false` as a safety net.
const TYPING_TIMEOUT_SECS: u64 = 5;

/// Resolve the identity file path. `WHISPER_IDENTITY_FILE` overrides the
/// default so two Whisper instances can run side by side on one machine
/// (e.g. to test E2EE between two windows).
fn resolve_identity_path(app: &AppHandle) -> PathBuf {
    std::env::var("WHISPER_IDENTITY_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            app.path()
                .app_data_dir()
                .map(|dir| dir.join("identity.json"))
                .unwrap_or_else(|_| PathBuf::from("identity.json"))
        })
}

/// Resolve the profiles file path. `WHISPER_PROFILES_FILE` mirrors the
/// identity override so side-by-side test instances keep separate names.
pub fn resolve_profiles_path(app: &AppHandle) -> PathBuf {
    std::env::var("WHISPER_PROFILES_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            app.path()
                .app_data_dir()
                .map(|dir| dir.join("profiles.json"))
                .unwrap_or_else(|_| PathBuf::from("profiles.json"))
        })
}

/// Errors surfaced to the UI when talking to the relay or the crypto core.
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    /// The outbox is empty, so nothing can be sent.
    #[error("not connected to the relay")]
    NotConnected,
    /// No identity has been loaded yet.
    #[error("no local identity; create one first")]
    NoIdentity,
    /// No ratchet session exists with the given peer.
    #[error("no session with peer {0}")]
    NoSession(String),
    /// The peer ID equals our own identity.
    #[error("refusing to chat with yourself")]
    InvalidPeer(String),
    /// A first-message ciphertext that was not a pre-key message.
    #[error("unexpected message type during handshake")]
    UnexpectedMessageType,
    /// The relay did not answer a pre-key fetch in time.
    #[error("timed out waiting for pre-keys")]
    PrekeyTimeout,
    /// The pre-key request was answered with an error or dropped.
    #[error("pre-key request failed")]
    PrekeyFetchFailed,
    /// The relay replied with an error code.
    #[error("relay error: {0}")]
    Relay(String),
    /// A display name failed local validation (mirrors the server's rules).
    #[error("display name must be 1-64 characters without control characters")]
    InvalidDisplayName,
    /// The WebSocket could not be opened.
    #[error("relay connection failed: {0}")]
    Connection(String),
    /// An identity (de)serialization failure.
    #[error("identity error: {0}")]
    Identity(#[from] e2ee_core::IdentityError),
    /// A session or X3DH failure.
    #[error("session error: {0}")]
    Session(#[from] e2ee_core::SessionError),
    /// A JSON (de)serialization failure.
    #[error("message encoding error: {0}")]
    Json(#[from] serde_json::Error),
    /// A base64 payload was malformed.
    #[error("invalid base64: {0}")]
    Base64(#[from] base64::DecodeError),
    /// A shared lock was poisoned by a panicking task.
    #[error("internal state was poisoned by a panic")]
    Poisoned,
    /// A filesystem failure (identity persistence).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Messages the CLIENT sends to the relay (matches `server/src/relay.rs`).
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    /// Announces a signed hello binding the peer ID to its public keys. The
    /// optional `display_name` is public profile data (Signal-style) the
    /// relay stores so other peers can show it in pre-key lookups.
    Hello {
        peer_id: String,
        curve25519_key: String,
        ed25519_key: String,
        signature: String,
        display_name: Option<String>,
    },
    /// An opaque encrypted envelope to route onward.
    Envelope { envelope: RelayEnvelope },
    /// Offline sync: request queued envelopes with `seq > since`.
    #[serde(rename = "fetch_since")]
    FetchSince { since: u64 },
    /// Publish a fresh pre-key bundle for the X3DH handshake.
    #[serde(rename = "publish_prekeys")]
    PublishPrekeys { bundle: Box<PreKeyBundle> },
    /// Fetch another peer's published pre-key bundle.
    #[serde(rename = "fetch_prekeys")]
    FetchPrekeys { peer_id: String },
    /// Set the caller's public display name (Signal-style profile name).
    #[serde(rename = "update_profile")]
    UpdateProfile { display_name: String },
}

/// Messages the SERVER sends to the client (matches `server/src/relay.rs`).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    /// A routed envelope destined for this peer.
    Envelope { envelope: RelayEnvelope },
    /// Delivery confirmation for a sent envelope.
    #[serde(rename = "ack")]
    Acknowledged { seq: u64 },
    /// Batch reply to `fetch_since`.
    Envelopes { envelopes: Vec<RelayEnvelope> },
    /// A requested pre-key bundle plus the peer's public display name (`null`
    /// when they have not set one). `display_name` defaults to `None` so older
    /// replies without the field still parse.
    Prekeys {
        bundle: Box<PreKeyBundle>,
        #[serde(default)]
        display_name: Option<String>,
    },
    /// The published bundle was accepted.
    #[serde(rename = "prekeys_published")]
    PrekeysPublished,
    /// The caller's display name was updated.
    #[serde(rename = "profile_updated")]
    ProfileUpdated,
    /// A protocol error code.
    Error { code: String },
}

/// The opaque envelope shape the relay routes between peers.
///
/// `payload` is the base64 encoding of a serialized [`e2ee_core::Envelope`];
/// the relay stores and forwards it without ever inspecting it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RelayEnvelope {
    sender: String,
    recipient: String,
    payload: String,
    seq: u64,
}

/// A message in a shape the UI can render directly.
#[derive(Debug, Clone, Serialize)]
pub struct UIMessage {
    /// Client-generated id, used to deduplicate optimistic insertions.
    pub id: String,
    /// Decrypted plaintext.
    pub text: String,
    /// True when this message was sent by the local identity.
    pub outgoing: bool,
    /// Epoch milliseconds when the message was decrypted or sent.
    pub timestamp: u64,
    /// Delivery state shown by the UI: "sent" until the relay acks the
    /// envelope, then "delivered". Incoming messages are always "delivered".
    pub status: String,
}

/// Payload of the `chat-message` event emitted for new plaintext.
#[derive(Debug, Clone, Serialize)]
pub struct ChatMessageEvent {
    pub peer_id: String,
    pub message: UIMessage,
}

/// Payload of the `relay-status` event emitted on connect/disconnect.
#[derive(Debug, Clone, Serialize)]
pub struct RelayStatusEvent {
    pub connected: bool,
}

/// Payload of the `message-status` event emitted when a sent message's
/// delivery state changes: the relay ack flips it to "delivered" and an
/// end-to-end read receipt flips it to "read".
#[derive(Debug, Clone, Serialize)]
pub struct MessageStatusEvent {
    pub client_id: String,
    pub status: String,
}

/// Payload of the `typing` event emitted when a peer starts or stops typing.
/// `TypingStopped` receipts and the 5-second auto-timeout both emit
/// `is_typing: false`.
#[derive(Debug, Clone, Serialize)]
pub struct TypingEvent {
    pub peer_id: String,
    pub is_typing: bool,
}

/// Payload of the `contact-updated` event emitted when a contact's display
/// name is learned or refreshed (from a pre-key lookup), so the UI can update
/// the contact list without waiting for a full state refresh.
#[derive(Debug, Clone, Serialize)]
pub struct ContactUpdatedEvent {
    pub peer_id: String,
    pub display_name: Option<String>,
}

/// Persisted profile data stored in `profiles.json` (next to `identity.json`):
/// our own public display name plus the display names we have learned for our
/// contacts. Keeping it in a separate file means a corrupt file can never
/// block identity loading, exactly like `settings.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profiles {
    /// Our own public display name; `None` when unset.
    #[serde(default)]
    pub my_display_name: Option<String>,
    /// Peer ID -> the display name that peer advertises in pre-key lookups.
    #[serde(default)]
    pub contacts: HashMap<String, String>,
}

/// A known conversation peer plus the display name they advertise, if any.
#[derive(Debug, Clone, Serialize)]
pub struct ContactInfo {
    pub peer_id: String,
    /// `None` (or a peer with no name) falls back to the peer ID in the UI.
    pub display_name: Option<String>,
}

/// Snapshot of everything the UI needs to render the chat surface.
#[derive(Debug, Clone, Serialize)]
pub struct ChatState {
    pub my_peer_id: String,
    /// Our own public display name; `None` when unset.
    pub my_display_name: Option<String>,
    pub connected: bool,
    /// Peer IDs this identity has a conversation with, plus their names.
    pub contacts: Vec<ContactInfo>,
    /// Per-peer message history, oldest first.
    pub messages: HashMap<String, Vec<UIMessage>>,
}

/// Persisted user preferences stored in `settings.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Relay endpoint to connect to; falls back to the `WHISPER_RELAY_URL` env
    /// var and then [`DEFAULT_RELAY_URL`].
    #[serde(default)]
    pub relay_url: Option<String>,
    /// UI theme ("dark", "light", ...); the UI owns the valid values.
    #[serde(default)]
    pub theme: Option<String>,
}

/// Thread-safe handle to the relay client, managed as Tauri state.
#[derive(Clone)]
pub struct RelayClient {
    inner: Arc<RelayInner>,
}

struct RelayInner {
    app: AppHandle,
    identity_path: PathBuf,
    /// File where Double Ratchet sessions are persisted across restarts.
    sessions_path: PathBuf,
    /// File where the relay URL and UI theme are persisted.
    settings_path: PathBuf,
    /// File where display names (our own + contacts') are persisted.
    profiles_path: PathBuf,
    /// In-memory copy of the persisted settings.
    settings: RwLock<Settings>,
    /// In-memory copy of the persisted display names.
    profiles: RwLock<Profiles>,
    /// The local identity, loaded from disk on first connect.
    identity: Mutex<Option<Identity>>,
    /// Double Ratchet sessions, keyed by the remote peer ID.
    sessions: Mutex<HashMap<String, ChatSession>>,
    /// Per-peer decrypted message history.
    messages: RwLock<HashMap<String, Vec<UIMessage>>>,
    /// Known conversation peers, in first-contact order.
    contacts: RwLock<Vec<String>>,
    connected: AtomicBool,
    /// Monotonic envelope sequence counter.
    seq: AtomicU64,
    /// Monotonic id counter for server-side message ids.
    next_msg_id: AtomicU64,
    /// Outbound socket half; `None` while disconnected.
    outbox: RwLock<Option<mpsc::UnboundedSender<WsMessage>>>,
    /// Serializes concurrent `connect` calls so exactly one WebSocket is ever
    /// opened (React StrictMode double-mounts in dev, invoking the command
    /// twice at once).
    connecting: tokio::sync::Mutex<()>,
    /// In-flight pre-key fetches, resolved in FIFO order.
    pending_prekeys: Mutex<VecDeque<oneshot::Sender<PrekeyResponse>>>,
    /// Bounded dedup set of (sender, seq) pairs, because the relay delivers
    /// offline envelopes at least once (pushed on connect, then drained via
    /// `fetch_since`).
    seen_envelopes: Mutex<HashSet<(String, u64)>>,
    /// Envelope seqs awaiting a relay `ack`, mapped to the UI's client message
    /// id so a delivery confirmation can flip the matching message to
    /// "delivered".
    pending_acks: Mutex<HashMap<u64, String>>,
    /// Per-peer generation counter used to deduplicate typing auto-timeouts:
    /// each new `typing` receipt bumps the counter so older pending timers
    /// give up instead of flipping the indicator off early.
    typing_timeouts: Mutex<HashMap<String, u64>>,
}

/// Result channel type for a pre-key fetch: the bundle plus the peer's public
/// display name (`None` when they have not set one).
type PrekeyResponse = Result<(PreKeyBundle, Option<String>), RelayError>;

impl RelayClient {
    /// Build a client bound to `app`'s identity file in the app data dir.
    pub fn new(app: AppHandle) -> Self {
        let identity_path = resolve_identity_path(&app);
        let sessions_path = app
            .path()
            .app_data_dir()
            .map(|dir| dir.join("sessions.json"))
            .unwrap_or_else(|_| PathBuf::from("sessions.json"));
        let settings_path = app
            .path()
            .app_data_dir()
            .map(|dir| dir.join("settings.json"))
            .unwrap_or_else(|_| PathBuf::from("settings.json"));
        let profiles_path = resolve_profiles_path(&app);
        Self {
            inner: Arc::new(RelayInner {
                app,
                identity_path,
                sessions_path,
                settings_path,
                profiles_path,
                settings: RwLock::new(Settings::default()),
                profiles: RwLock::new(Profiles::default()),
                identity: Mutex::new(None),
                sessions: Mutex::new(HashMap::new()),
                messages: RwLock::new(HashMap::new()),
                contacts: RwLock::new(Vec::new()),
                connected: AtomicBool::new(false),
                seq: AtomicU64::new(1),
                next_msg_id: AtomicU64::new(1),
                outbox: RwLock::new(None),
                connecting: tokio::sync::Mutex::new(()),
                pending_prekeys: Mutex::new(VecDeque::new()),
                seen_envelopes: Mutex::new(HashSet::new()),
                pending_acks: Mutex::new(HashMap::new()),
                typing_timeouts: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Open a WebSocket to the relay, authenticate and start both pump loops.
    ///
    /// Idempotent: a second call while already connected is a no-op. The
    /// outbound pump drains a channel shared by every command; the inbound
    /// loop decrypts incoming envelopes and emits UI events.
    pub async fn connect(&self) -> Result<(), RelayError> {
        // Serialize concurrent connect calls: dev builds (React StrictMode)
        // invoke this command twice at once, and without the lock both calls
        // would open a socket and overwrite each other's outbox channel,
        // killing one connection immediately.
        let _connect_guard = self.inner.connecting.lock().await;
        if self.inner.connected.load(Ordering::SeqCst) {
            return Ok(());
        }

        // Restore previously persisted sessions so existing conversations
        // keep working across restarts.
        self.load_sessions()?;

        let hello = {
            let mut guard = mutex_guard(&self.inner.identity)?;
            if guard.is_none() {
                let json = std::fs::read_to_string(&self.inner.identity_path)?;
                let identity = Identity::from_json(&json)?;
                *guard = Some(identity);
            }
            guard.as_ref().ok_or(RelayError::NoIdentity)?.signed_hello()
        };

        let settings = self.load_settings()?;
        let profiles = self.load_profiles()?;
        let url = resolve_relay_url(
            &settings,
            std::env::var("WHISPER_RELAY_URL").ok().as_deref(),
        );
        let (ws_stream, _) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|err| RelayError::Connection(err.to_string()))?;

        let (mut write, read) = ws_stream.split();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();

        {
            let mut outbox = write_guard(&self.inner.outbox)?;
            *outbox = Some(out_tx.clone());
        }
        self.inner.connected.store(true, Ordering::SeqCst);
        let _ = self
            .inner
            .app
            .emit("relay-status", RelayStatusEvent { connected: true });

        let hello_json = serde_json::to_string(&ClientMessage::Hello {
            peer_id: hello.peer_id,
            curve25519_key: hello.curve25519_key,
            ed25519_key: hello.ed25519_key,
            signature: hello.signature,
            display_name: profiles.my_display_name.clone(),
        })?;
        out_tx
            .send(WsMessage::Text(hello_json))
            .map_err(|_| RelayError::NotConnected)?;

        // Drain the relay's offline queue so reconnects never redeliver the
        // same envelopes. Anything the server already pushed on connect is
        // skipped by the (sender, seq) dedup filter below.
        let _ = self.send_json(&ClientMessage::FetchSince { since: 0 });

        // Pump outbox -> socket so every command can send without owning the
        // socket. Closing the write side tears the connection down cleanly.
        tauri::async_runtime::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                if write.send(msg).await.is_err() {
                    break;
                }
            }
            let _ = write.close().await;
        });

        // Inbound loop: decrypt envelopes and update state until the socket
        // closes, then mark the client disconnected.
        let client = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut read = read;
            while let Some(item) = read.next().await {
                match item {
                    Ok(WsMessage::Text(text)) => {
                        if let Err(err) = client.handle_text(&text) {
                            eprintln!("whisper relay: inbound error: {err}");
                        }
                    }
                    Ok(WsMessage::Ping(payload)) => {
                        let _ = client.send_raw(WsMessage::Pong(payload));
                    }
                    Ok(WsMessage::Close(_)) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            client.mark_disconnected();
        });

        Ok(())
    }

    /// Tear down the current connection. Other commands will report
    /// [`RelayError::NotConnected`] until [`RelayClient::connect`] is called.
    pub fn disconnect(&self) -> Result<(), RelayError> {
        self.mark_disconnected();
        Ok(())
    }

    /// Disconnect and wipe all in-memory chat state. Called when the identity
    /// is reset so stale contacts, sessions and messages never leak into a
    /// freshly generated identity.
    pub fn reset(&self) -> Result<(), RelayError> {
        self.mark_disconnected();
        mutex_guard(&self.inner.identity)?.take();
        mutex_guard(&self.inner.sessions)?.clear();
        // Drop the persisted sessions so a fresh identity starts clean.
        if let Err(err) = std::fs::remove_file(&self.inner.sessions_path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(err.into());
            }
        }
        // A fresh identity means a fresh name and no learned contact names.
        *write_guard(&self.inner.profiles)? = Profiles::default();
        if let Err(err) = std::fs::remove_file(&self.inner.profiles_path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(err.into());
            }
        }
        write_guard(&self.inner.messages)?.clear();
        write_guard(&self.inner.contacts)?.clear();
        if let Ok(mut seen) = self.inner.seen_envelopes.lock() {
            seen.clear();
        }
        Ok(())
    }

    /// Generate a fresh batch of one-time pre-keys, publish them and mark them
    /// as published so they are never advertised twice.
    pub async fn publish_prekeys(&self) -> Result<(), RelayError> {
        let bundle = {
            let mut guard = mutex_guard(&self.inner.identity)?;
            let identity = guard.as_mut().ok_or(RelayError::NoIdentity)?;
            let bundle = identity.pre_key_bundle(PREKEY_BATCH_SIZE);
            identity.mark_keys_as_published();
            bundle
        };
        self.send_json(&ClientMessage::PublishPrekeys {
            bundle: Box::new(bundle),
        })
    }

    /// Fetch a peer's pre-key bundle, waiting up to [`PREKEY_FETCH_TIMEOUT`].
    async fn fetch_prekeys(
        &self,
        peer_id: &str,
    ) -> Result<(PreKeyBundle, Option<String>), RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_prekeys)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::FetchPrekeys {
            peer_id: peer_id.to_string(),
        }) {
            // The request never left, so drop the dangling waiter to keep the
            // pending queue aligned with the relay's replies.
            mutex_guard(&self.inner.pending_prekeys)?.pop_back();
            return Err(err);
        }

        tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::PrekeyTimeout)?
            .map_err(|_| RelayError::PrekeyFetchFailed)?
    }

    /// Establish an outbound X3DH session with `peer_id` and send the first,
    /// session-creating message.
    pub async fn start_chat(&self, peer_id: &str) -> Result<(), RelayError> {
        if peer_id == self.my_peer_id()? {
            return Err(RelayError::InvalidPeer(peer_id.to_string()));
        }

        let (bundle, display_name) = self.fetch_prekeys(peer_id).await?;
        let my_peer_id = self.my_peer_id()?;

        // Learn the peer's public display name so the UI can show it in the
        // contact list and chat header.
        if let Some(name) = display_name {
            self.remember_contact_name(peer_id, &name)?;
        }

        // Build the outbound session and encrypt the very first message; the
        // first ciphertext is always a pre-key message.
        let pre_key = {
            let mut guard = mutex_guard(&self.inner.identity)?;
            let identity = guard.as_mut().ok_or(RelayError::NoIdentity)?;
            let session = ChatSession::create_outbound(identity, &bundle)?;
            let mut sessions = mutex_guard(&self.inner.sessions)?;
            sessions.insert(peer_id.to_string(), session);
            let session = sessions
                .get_mut(peer_id)
                .ok_or_else(|| RelayError::NoSession(peer_id.to_string()))?;
            match session.encrypt(FIRST_MESSAGE_TEXT)? {
                OlmMessage::PreKey(pk) => pk,
                OlmMessage::Normal(_) => return Err(RelayError::UnexpectedMessageType),
            }
        };

        // Persist the new session so it survives a restart.
        self.save_sessions()?;

        let wire = Envelope::new(
            my_peer_id.clone(),
            peer_id.to_string(),
            EnvelopeContent::Handshake(Handshake::new(my_peer_id.clone(), pre_key)),
        );
        let seq = self.next_seq();
        self.send_wire(&wire, seq)?;

        self.ensure_contact(peer_id)?;
        let msg = self.record_outgoing(peer_id, FIRST_MESSAGE_TEXT, "")?;
        self.record_pending_ack(seq, &msg.id)?;
        let _ = self.inner.app.emit(
            "chat-message",
            ChatMessageEvent {
                peer_id: peer_id.to_string(),
                message: msg,
            },
        );
        Ok(())
    }

    /// Encrypt `text` with the existing session for `peer_id` and send it.
    pub async fn send_message(
        &self,
        peer_id: &str,
        text: &str,
        client_id: &str,
    ) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        let (olm, session_id) = {
            let mut sessions = mutex_guard(&self.inner.sessions)?;
            let session = sessions
                .get_mut(peer_id)
                .ok_or_else(|| RelayError::NoSession(peer_id.to_string()))?;
            let session_id = session.session_id();
            let olm = session.encrypt(text)?;
            (olm, session_id)
        };

        // The ratchet advanced, so persist the updated session state.
        self.save_sessions()?;

        let wire = Envelope::new(
            my_peer_id.clone(),
            peer_id.to_string(),
            EnvelopeContent::Message(Message::new(my_peer_id.clone(), session_id, olm)),
        );

        // Allocate the envelope seq before queueing so the (seq, client id)
        // ack mapping is registered first: the relay acknowledges a received
        // envelope immediately and the inbound loop may process that ack while
        // this command is still returning.
        let seq = self.next_seq();
        let msg = self.record_outgoing(peer_id, text, client_id)?;
        self.record_pending_ack(seq, &msg.id)?;

        if let Err(err) = self.send_wire(&wire, seq) {
            // The envelope never left the client: drop the dangling ack
            // mapping and roll back the optimistic record so a failed send
            // does not surface as a sent message on the next refresh.
            let _ = mutex_guard(&self.inner.pending_acks)?.remove(&seq);
            if let Ok(mut messages) = write_guard(&self.inner.messages) {
                if let Some(msgs) = messages.get_mut(peer_id) {
                    msgs.retain(|m| m.id != msg.id);
                }
            }
            return Err(err);
        }

        let _ = self.inner.app.emit(
            "chat-message",
            ChatMessageEvent {
                peer_id: peer_id.to_string(),
                message: msg,
            },
        );
        Ok(())
    }

    /// Snapshot the state the UI needs: identity, connection, contacts (with
    /// their display names) and message history.
    pub fn get_chat_state(&self) -> Result<ChatState, RelayError> {
        let my_peer_id = self.my_peer_id()?;
        let profiles = self.load_profiles()?;
        let contacts = read_guard(&self.inner.contacts)?.clone();
        let messages = read_guard(&self.inner.messages)?.clone();
        let connected = self.inner.connected.load(Ordering::SeqCst);
        let contacts = contacts
            .into_iter()
            .map(|peer_id| ContactInfo {
                peer_id: peer_id.clone(),
                display_name: profiles.contacts.get(&peer_id).cloned(),
            })
            .collect();
        Ok(ChatState {
            my_peer_id,
            my_display_name: profiles.my_display_name,
            connected,
            contacts,
            messages,
        })
    }

    /// Remember an envelope's (sender, seq) pair and report whether it has
    /// been seen before. The set is bounded to keep memory flat.
    fn is_duplicate_envelope(&self, sender: &str, seq: u64) -> Result<bool, RelayError> {
        const MAX_SEEN: usize = 2048;
        const EVICT_BATCH: usize = 128;

        let mut seen = mutex_guard(&self.inner.seen_envelopes)?;
        let key = (sender.to_string(), seq);
        let is_duplicate = seen.contains(&key);

        if seen.len() >= MAX_SEEN {
            let evict: Vec<_> = seen.iter().take(EVICT_BATCH).cloned().collect();
            for old in evict {
                seen.remove(&old);
            }
        }
        seen.insert(key);
        Ok(is_duplicate)
    }

    // ---------------------------------------------------------------------
    // Inbound handling
    // ---------------------------------------------------------------------

    /// Dispatch a server message: route envelopes, resolve pre-key fetches.
    fn handle_text(&self, text: &str) -> Result<(), RelayError> {
        let message: ServerMessage = serde_json::from_str(text)?;
        match message {
            ServerMessage::Envelope { envelope } => self.handle_envelope(envelope),
            ServerMessage::Envelopes { envelopes } => {
                for envelope in envelopes {
                    self.handle_envelope(envelope)?;
                }
                Ok(())
            }
            ServerMessage::Prekeys {
                bundle,
                display_name,
            } => {
                let mut pending = mutex_guard(&self.inner.pending_prekeys)?;
                if let Some(tx) = pending.pop_front() {
                    let _ = tx.send(Ok((*bundle, display_name)));
                }
                Ok(())
            }
            ServerMessage::Acknowledged { seq } => self.handle_ack(seq),
            ServerMessage::PrekeysPublished => Ok(()),
            ServerMessage::ProfileUpdated => Ok(()),
            ServerMessage::Error { code } => {
                let mut pending = mutex_guard(&self.inner.pending_prekeys)?;
                if let Some(tx) = pending.pop_front() {
                    let _ = tx.send(Err(RelayError::Relay(code)));
                }
                Ok(())
            }
        }
    }

    /// Mark the message behind a relay `ack` as delivered and notify the UI.
    ///
    /// The relay acknowledges an envelope once it is received and queued; the
    /// `seq` resolves back to the UI's client message id via `pending_acks`.
    /// The emitted `message-status` event carries the client id so the UI can
    /// flip its own optimistic message; a state refresh keeps the marker
    /// because `pending_acks` and `messages` were updated here.
    fn handle_ack(&self, seq: u64) -> Result<(), RelayError> {
        let client_id = match mutex_guard(&self.inner.pending_acks)?.remove(&seq) {
            Some(client_id) => client_id,
            // A late duplicate or an ack for a seq we no longer track
            // (e.g. after a restart) is simply ignored.
            None => return Ok(()),
        };
        // Only announce the flip when it actually changed something. The relay
        // can ack an envelope after the peer's read receipt already marked the
        // message "read"; announcing "delivered" then would downgrade it.
        if self.mark_delivered(&client_id)? {
            let _ = self.inner.app.emit(
                "message-status",
                MessageStatusEvent {
                    client_id,
                    status: "delivered".to_string(),
                },
            );
        }
        Ok(())
    }

    /// Decode an opaque relay envelope, decrypt its payload and record it.
    fn handle_envelope(&self, envelope: RelayEnvelope) -> Result<(), RelayError> {
        // The relay can deliver an envelope twice (pushed on connect and again
        // via fetch_since); skip anything already seen for this sender.
        if self.is_duplicate_envelope(&envelope.sender, envelope.seq)? {
            return Ok(());
        }

        let payload = BASE64.decode(&envelope.payload)?;
        let wire: Envelope = serde_json::from_slice(&payload)?;

        // The relay authenticated the outer sender; only trust inner envelopes
        // whose claimed sender matches it.
        if wire.sender_peer_id != envelope.sender {
            return Ok(());
        }

        if let Some(message) = self.ingest(wire)? {
            let _ = self.inner.app.emit(
                "chat-message",
                ChatMessageEvent {
                    peer_id: envelope.sender,
                    message,
                },
            );
        }
        Ok(())
    }

    /// Turn an incoming wire envelope into plaintext.
    ///
    /// A handshake establishes the inbound X3DH session using the sender's
    /// identity key embedded in the pre-key message; an ordinary message is
    /// decrypted with the already-established session.
    fn ingest(&self, wire: Envelope) -> Result<Option<UIMessage>, RelayError> {
        let sender = wire.sender_peer_id;
        match wire.content {
            EnvelopeContent::Handshake(handshake) => {
                let mut guard = mutex_guard(&self.inner.identity)?;
                let identity = guard.as_mut().ok_or(RelayError::NoIdentity)?;
                let their_key = handshake.pre_key_message.identity_key();
                let inbound =
                    ChatSession::create_inbound(identity, their_key, &handshake.pre_key_message)?;
                mutex_guard(&self.inner.sessions)?.insert(sender.clone(), inbound.session);
                let text = String::from_utf8_lossy(&inbound.plaintext).to_string();
                self.save_sessions()?;
                Ok(Some(self.record_incoming(&sender, text)?))
            }
            EnvelopeContent::Message(message) => {
                let plaintext = {
                    let mut sessions = mutex_guard(&self.inner.sessions)?;
                    let session = sessions
                        .get_mut(&sender)
                        .ok_or_else(|| RelayError::NoSession(sender.clone()))?;
                    session.decrypt(&message.message)?
                };
                // Receipts travel as an encrypted serialized
                // `EnvelopeContent::Receipt` inside an ordinary message, so
                // the relay only ever sees ciphertext. They are recognised by
                // parsing the decrypted plaintext first.
                if let Ok(EnvelopeContent::Receipt { kind }) =
                    serde_json::from_slice::<EnvelopeContent>(&plaintext)
                {
                    self.save_sessions()?;
                    self.handle_receipt(&sender, kind)?;
                    return Ok(None);
                }
                let text = String::from_utf8_lossy(&plaintext).to_string();
                self.save_sessions()?;
                // Acknowledging the message end-to-end: encrypt a read receipt
                // with the same (now-advanced) session. Best-effort so a
                // transient send failure never drops the plaintext message.
                let _ = self.send_receipt(&sender, ReceiptKind::Read);
                Ok(Some(self.record_incoming(&sender, text)?))
            }
            // A bundle is published, never delivered as a chat envelope.
            EnvelopeContent::PreKeyBundle(_) => Ok(None),
            // Defensive: this client always sends receipts encrypted inside a
            // Message, so a bare receipt content is never expected here.
            EnvelopeContent::Receipt { .. } => Ok(None),
        }
    }

    // ---------------------------------------------------------------------
    // Session persistence
    // ---------------------------------------------------------------------

    /// Restore persisted Double Ratchet sessions from `sessions.json`.
    fn load_sessions(&self) -> Result<(), RelayError> {
        let json = match std::fs::read_to_string(&self.inner.sessions_path) {
            Ok(json) => json,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let stored: HashMap<String, String> = serde_json::from_str(&json)?;
        let mut sessions = mutex_guard(&self.inner.sessions)?;
        for (peer, session_json) in stored {
            if let Ok(session) = ChatSession::from_json(&session_json) {
                sessions.insert(peer, session);
            }
        }
        Ok(())
    }

    /// Persist all current sessions to `sessions.json`.
    fn save_sessions(&self) -> Result<(), RelayError> {
        let mut stored = HashMap::new();
        {
            let sessions = mutex_guard(&self.inner.sessions)?;
            for (peer, session) in sessions.iter() {
                if let Ok(json) = session.to_json() {
                    stored.insert(peer.clone(), json);
                }
            }
        }
        let json = serde_json::to_string(&stored)?;
        if let Some(dir) = self.inner.sessions_path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        std::fs::write(&self.inner.sessions_path, json)?;
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Settings persistence
    // ---------------------------------------------------------------------

    /// Load the persisted settings from disk and cache them in memory. A
    /// missing or unparseable file yields the defaults instead of an error.
    fn load_settings(&self) -> Result<Settings, RelayError> {
        let settings = read_settings_file(&self.inner.settings_path)?;
        let mut guard = write_guard(&self.inner.settings)?;
        *guard = settings.clone();
        Ok(settings)
    }

    /// Return the currently persisted settings, reloaded from disk.
    pub fn get_settings(&self) -> Result<Settings, RelayError> {
        self.load_settings()?;
        let settings = read_guard(&self.inner.settings)?.clone();
        Ok(settings)
    }

    /// Persist `settings` to disk and cache them in memory.
    fn save_settings(&self, settings: &Settings) -> Result<(), RelayError> {
        write_settings_file(&self.inner.settings_path, settings)?;
        let mut guard = write_guard(&self.inner.settings)?;
        *guard = settings.clone();
        Ok(())
    }

    /// Persist a new relay endpoint. If the client is connected to a different
    /// URL, the connection is dropped so the UI can reconnect to the new
    /// address.
    pub fn set_relay_url(&self, url: &str) -> Result<(), RelayError> {
        let mut settings = self.load_settings()?;
        let changed = settings.relay_url.as_deref() != Some(url);
        settings.relay_url = Some(url.to_string());
        self.save_settings(&settings)?;
        if changed && self.inner.connected.load(Ordering::SeqCst) {
            self.disconnect()?;
        }
        Ok(())
    }

    /// Persist a new UI theme preference.
    pub fn set_theme(&self, theme: &str) -> Result<(), RelayError> {
        let mut settings = self.load_settings()?;
        settings.theme = Some(theme.to_string());
        self.save_settings(&settings)
    }

    // ---------------------------------------------------------------------
    // Display names and receipts
    // ---------------------------------------------------------------------

    /// Persist our own public display name and, when connected, announce it to
    /// the relay so everyone who fetches our pre-keys sees it. An empty name
    /// clears the local profile (the previously published name stays visible
    /// to others until overwritten — the server rejects empty names).
    pub fn set_display_name(&self, name: &str) -> Result<(), RelayError> {
        let name = name.trim();
        if !name.is_empty()
            && (name.chars().count() > MAX_DISPLAY_NAME_CHARS || name.chars().any(char::is_control))
        {
            return Err(RelayError::InvalidDisplayName);
        }
        let mut profiles = self.load_profiles()?;
        profiles.my_display_name = if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        };
        self.save_profiles(&profiles)?;
        if !name.is_empty() && self.inner.connected.load(Ordering::SeqCst) {
            self.send_json(&ClientMessage::UpdateProfile {
                display_name: name.to_string(),
            })?;
        }
        Ok(())
    }

    /// Send an end-to-end typing indicator (or the "stopped" signal) to a
    /// peer, encrypted inside the established session.
    pub fn send_typing(&self, peer_id: &str, is_typing: bool) -> Result<(), RelayError> {
        let kind = if is_typing {
            ReceiptKind::Typing
        } else {
            ReceiptKind::TypingStopped
        };
        self.send_receipt(peer_id, kind)
    }

    /// Encrypt and send an end-to-end receipt inside the session with
    /// `peer_id`. The receipt is serialized as [`e2ee_core::EnvelopeContent`]
    /// and encrypted like an ordinary message, so the relay only ever sees the
    /// ciphertext of a [`e2ee_core::Message`].
    fn send_receipt(&self, peer_id: &str, kind: ReceiptKind) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        let content = EnvelopeContent::Receipt { kind };
        let (olm, session_id) = {
            let mut sessions = mutex_guard(&self.inner.sessions)?;
            let session = sessions
                .get_mut(peer_id)
                .ok_or_else(|| RelayError::NoSession(peer_id.to_string()))?;
            let session_id = session.session_id();
            let olm = session.encrypt(serde_json::to_vec(&content)?)?;
            (olm, session_id)
        };
        self.save_sessions()?;
        let wire = Envelope::new(
            my_peer_id.clone(),
            peer_id.to_string(),
            EnvelopeContent::Message(Message::new(my_peer_id, session_id, olm)),
        );
        let seq = self.next_seq();
        self.send_wire(&wire, seq)
    }

    /// Remember a display name learned for a contact and persist it. Emits a
    /// `contact-updated` event so the UI can update the contact list without a
    /// full state refresh.
    fn remember_contact_name(&self, peer_id: &str, name: &str) -> Result<(), RelayError> {
        let mut profiles = self.load_profiles()?;
        profiles
            .contacts
            .insert(peer_id.to_string(), name.to_string());
        self.save_profiles(&profiles)?;
        let _ = self.inner.app.emit(
            "contact-updated",
            ContactUpdatedEvent {
                peer_id: peer_id.to_string(),
                display_name: Some(name.to_string()),
            },
        );
        Ok(())
    }

    /// Apply an inbound end-to-end receipt. Read receipts flip all of our
    /// outgoing messages to the sender to "read"; typing receipts are relayed
    /// to the UI (with a 5-second auto-timeout that emits `false` unless a
    /// newer indicator arrives first).
    fn handle_receipt(&self, sender: &str, kind: ReceiptKind) -> Result<(), RelayError> {
        match kind {
            ReceiptKind::Read => {
                // A single `Read` receipt acknowledges every message the peer
                // has read so far, so all unread outgoing messages to them
                // flip at once. Each flip emits one `message-status` event.
                let flipped = {
                    let mut messages = write_guard(&self.inner.messages)?;
                    apply_read(&mut messages, sender)
                };
                for client_id in flipped {
                    let _ = self.inner.app.emit(
                        "message-status",
                        MessageStatusEvent {
                            client_id,
                            status: "read".to_string(),
                        },
                    );
                }
                Ok(())
            }
            ReceiptKind::Typing | ReceiptKind::TypingStopped => {
                let is_typing = kind == ReceiptKind::Typing;
                let mut timers = mutex_guard(&self.inner.typing_timeouts)?;
                let generation = timers.entry(sender.to_string()).or_insert(0);
                *generation += 1;
                let generation = *generation;
                drop(timers);
                let _ = self.inner.app.emit(
                    "typing",
                    TypingEvent {
                        peer_id: sender.to_string(),
                        is_typing,
                    },
                );
                // A "stopped" receipt cancels any pending auto-timeout by
                // bumping the generation; nothing more is scheduled for it.
                if is_typing {
                    let client = self.clone();
                    let peer = sender.to_string();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(TYPING_TIMEOUT_SECS))
                            .await;
                        let timers = match mutex_guard(&client.inner.typing_timeouts) {
                            Ok(t) => t,
                            Err(_) => return,
                        };
                        if timers.get(&peer).copied() == Some(generation) {
                            drop(timers);
                            let _ = client.inner.app.emit(
                                "typing",
                                TypingEvent {
                                    peer_id: peer.clone(),
                                    is_typing: false,
                                },
                            );
                        }
                    });
                }
                Ok(())
            }
        }
    }

    /// Load the persisted display names from disk and cache them in memory. A
    /// missing or unparseable file yields the defaults instead of an error.
    fn load_profiles(&self) -> Result<Profiles, RelayError> {
        let profiles = read_profiles_file(&self.inner.profiles_path)?;
        let mut guard = write_guard(&self.inner.profiles)?;
        *guard = profiles.clone();
        Ok(profiles)
    }

    /// Persist display names to disk and cache them in memory.
    fn save_profiles(&self, profiles: &Profiles) -> Result<(), RelayError> {
        write_profiles_file(&self.inner.profiles_path, profiles)?;
        let mut guard = write_guard(&self.inner.profiles)?;
        *guard = profiles.clone();
        Ok(())
    }

    // ---------------------------------------------------------------------
    // State recording
    // ---------------------------------------------------------------------

    /// Record an inbound plaintext message and add the sender as a contact.
    fn record_incoming(&self, peer_id: &str, text: String) -> Result<UIMessage, RelayError> {
        let message = UIMessage {
            id: format!(
                "in-{}",
                self.inner.next_msg_id.fetch_add(1, Ordering::SeqCst)
            ),
            text,
            outgoing: false,
            timestamp: now_millis(),
            status: "delivered".to_string(),
        };
        self.ensure_contact(peer_id)?;
        write_guard(&self.inner.messages)?
            .entry(peer_id.to_string())
            .or_default()
            .push(message.clone());
        Ok(message)
    }

    /// Record an outbound plaintext message under the client-provided id, or a
    /// generated one when none was supplied.
    fn record_outgoing(
        &self,
        peer_id: &str,
        text: &str,
        client_id: &str,
    ) -> Result<UIMessage, RelayError> {
        let id = if client_id.is_empty() {
            format!(
                "out-{}",
                self.inner.next_msg_id.fetch_add(1, Ordering::SeqCst)
            )
        } else {
            client_id.to_string()
        };
        let message = UIMessage {
            id,
            text: text.to_string(),
            outgoing: true,
            timestamp: now_millis(),
            status: "sent".to_string(),
        };
        write_guard(&self.inner.messages)?
            .entry(peer_id.to_string())
            .or_default()
            .push(message.clone());
        Ok(message)
    }

    /// Flip the status of the message with `client_id` to "delivered" so a
    /// state refresh keeps the delivery marker. Returns whether the message
    /// was actually flipped: an ack that races in after an end-to-end read
    /// receipt must not downgrade the message back to "delivered".
    fn mark_delivered(&self, client_id: &str) -> Result<bool, RelayError> {
        let mut messages = write_guard(&self.inner.messages)?;
        Ok(apply_delivered(&mut messages, client_id))
    }

    /// Add `peer_id` to the contact list if it is not already there. For a new
    /// contact, the peer's public display name is fetched in the background so
    /// the receiving side of a conversation shows names too (not only the
    /// initiator, who learns the name during `start_chat`).
    fn ensure_contact(&self, peer_id: &str) -> Result<(), RelayError> {
        let mut contacts = write_guard(&self.inner.contacts)?;
        if !contacts.iter().any(|known| known == peer_id) {
            contacts.push(peer_id.to_string());
            let client = self.clone();
            let peer = peer_id.to_string();
            tauri::async_runtime::spawn(async move {
                // Best-effort: a peer that never published pre-keys simply has
                // no name to learn, and the fetch fails silently.
                if let Ok((_, Some(name))) = client.fetch_prekeys(&peer).await {
                    let _ = client.remember_contact_name(&peer, &name);
                }
            });
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Sending helpers
    // ---------------------------------------------------------------------

    /// Allocate the next unique envelope sequence number. Each call returns a
    /// fresh value, so acked seqs are never ambiguous.
    fn next_seq(&self) -> u64 {
        self.inner.seq.fetch_add(1, Ordering::SeqCst)
    }

    /// Remember that the envelope with `seq` is awaiting a relay ack, mapped
    /// to the UI's client message id.
    fn record_pending_ack(&self, seq: u64, client_id: &str) -> Result<(), RelayError> {
        mutex_guard(&self.inner.pending_acks)?.insert(seq, client_id.to_string());
        Ok(())
    }

    /// Serialize a wire envelope, base64 it and queue it on the outbox with the
    /// caller-allocated `seq` (see [`RelayClient::next_seq`]).
    fn send_wire(&self, wire: &Envelope, seq: u64) -> Result<(), RelayError> {
        let payload = BASE64.encode(serde_json::to_vec(wire)?);
        let envelope = RelayEnvelope {
            sender: wire.sender_peer_id.clone(),
            recipient: wire.recipient_peer_id.clone(),
            payload,
            seq,
        };
        self.send_json(&ClientMessage::Envelope { envelope })
    }

    /// Queue an already-serialized message on the outbox.
    fn send_json<T: Serialize>(&self, message: &T) -> Result<(), RelayError> {
        let text = serde_json::to_string(message)?;
        self.send_raw(WsMessage::Text(text))
    }

    /// Queue a raw WebSocket frame on the outbox.
    fn send_raw(&self, frame: WsMessage) -> Result<(), RelayError> {
        let outbox = read_guard(&self.inner.outbox)?;
        let tx = outbox.as_ref().ok_or(RelayError::NotConnected)?;
        tx.send(frame).map_err(|_| RelayError::NotConnected)
    }

    /// The peer ID of the loaded local identity.
    fn my_peer_id(&self) -> Result<String, RelayError> {
        let guard = mutex_guard(&self.inner.identity)?;
        Ok(guard.as_ref().ok_or(RelayError::NoIdentity)?.peer_id())
    }

    /// Clear connection state after the socket closes, for any reason.
    fn mark_disconnected(&self) {
        self.inner.connected.store(false, Ordering::SeqCst);
        if let Ok(mut outbox) = self.inner.outbox.write() {
            *outbox = None;
        }
        let _ = self
            .inner
            .app
            .emit("relay-status", RelayStatusEvent { connected: false });
    }
}

/// Current time as epoch milliseconds.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Read the settings file at `path`, returning defaults when it is missing or
/// unparseable so a corrupt file can never block startup or reconnect.
fn read_settings_file(path: &Path) -> Result<Settings, RelayError> {
    let json = match std::fs::read_to_string(path) {
        Ok(json) => json,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => return Err(e.into()),
    };
    Ok(serde_json::from_str(&json).unwrap_or_default())
}

/// Write the settings file at `path`, creating the parent directory on demand.
fn write_settings_file(path: &Path, settings: &Settings) -> Result<(), RelayError> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let json = serde_json::to_string_pretty(settings)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Read the profiles file at `path`, returning defaults when it is missing or
/// unparseable so a corrupt file can never block startup or reconnect.
fn read_profiles_file(path: &Path) -> Result<Profiles, RelayError> {
    let json = match std::fs::read_to_string(path) {
        Ok(json) => json,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Profiles::default()),
        Err(e) => return Err(e.into()),
    };
    Ok(serde_json::from_str(&json).unwrap_or_default())
}

/// Write the profiles file at `path`, creating the parent directory on demand.
pub fn write_profiles_file(path: &Path, profiles: &Profiles) -> Result<(), RelayError> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let json = serde_json::to_string_pretty(profiles)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Resolve the relay endpoint: settings first, then the `WHISPER_RELAY_URL`
/// env var, then the built-in default.
fn resolve_relay_url(settings: &Settings, env_url: Option<&str>) -> String {
    if let Some(url) = settings.relay_url.as_deref() {
        if !url.is_empty() {
            return url.to_string();
        }
    }
    env_url.unwrap_or(DEFAULT_RELAY_URL).to_string()
}

/// Pure helper for [`RelayClient::mark_delivered`], separated so the ack
/// bookkeeping can be unit-tested without a live WebSocket or Tauri app.
/// Returns whether the message was actually flipped: a message already marked
/// "read" is never downgraded by a late ack.
fn apply_delivered(messages: &mut HashMap<String, Vec<UIMessage>>, client_id: &str) -> bool {
    for msgs in messages.values_mut() {
        if let Some(message) = msgs.iter_mut().find(|m| m.id == client_id) {
            if message.status == "sent" {
                message.status = "delivered".to_string();
                return true;
            }
            // Already delivered or read: a late ack is a no-op.
            return false;
        }
    }
    false
}

/// Pure helper for [`RelayClient::handle_receipt`]: flip every outgoing
/// message to `peer_id` to "read", returning the client ids that changed so
/// the caller can notify the UI. Incoming messages and already-read messages
/// are left untouched.
fn apply_read(messages: &mut HashMap<String, Vec<UIMessage>>, peer_id: &str) -> Vec<String> {
    let mut flipped = Vec::new();
    if let Some(msgs) = messages.get_mut(peer_id) {
        for message in msgs.iter_mut() {
            if message.outgoing && message.status != "read" {
                message.status = "read".to_string();
                flipped.push(message.id.clone());
            }
        }
    }
    flipped
}

/// Lock a mutex, mapping poison onto a typed error.
fn mutex_guard<T>(lock: &Mutex<T>) -> Result<MutexGuard<'_, T>, RelayError> {
    lock.lock().map_err(|_| RelayError::Poisoned)
}

/// Read-lock a rwlock, mapping poison onto a typed error.
fn read_guard<T>(lock: &RwLock<T>) -> Result<RwLockReadGuard<'_, T>, RelayError> {
    lock.read().map_err(|_| RelayError::Poisoned)
}

/// Write-lock a rwlock, mapping poison onto a typed error.
fn write_guard<T>(lock: &RwLock<T>) -> Result<RwLockWriteGuard<'_, T>, RelayError> {
    lock.write().map_err(|_| RelayError::Poisoned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_serializes_to_relay_wire_format() {
        let identity = Identity::new();
        let hello = identity.signed_hello();
        let message = ClientMessage::Hello {
            peer_id: hello.peer_id,
            curve25519_key: hello.curve25519_key,
            ed25519_key: hello.ed25519_key,
            signature: hello.signature,
            display_name: None,
        };

        let json = serde_json::to_value(&message).expect("serialization must succeed");
        assert_eq!(json["type"], "hello");
        assert!(json.get("peer_id").is_some());
        assert!(json.get("curve25519_key").is_some());
        assert!(json.get("ed25519_key").is_some());
        assert!(json.get("signature").is_some());
        assert!(json.get("display_name").is_some());
        assert!(json["display_name"].is_null());
    }

    #[test]
    fn hello_serializes_an_advertised_display_name() {
        let identity = Identity::new();
        let hello = identity.signed_hello();
        let message = ClientMessage::Hello {
            peer_id: hello.peer_id,
            curve25519_key: hello.curve25519_key,
            ed25519_key: hello.ed25519_key,
            signature: hello.signature,
            display_name: Some("Alice Prime".into()),
        };

        let json = serde_json::to_value(&message).expect("serialization must succeed");
        assert_eq!(json["display_name"], "Alice Prime");
    }

    #[test]
    fn prekey_messages_serialize_to_expected_wire_shape() {
        let mut identity = Identity::new();
        let bundle = identity.pre_key_bundle(3);

        let publish = serde_json::to_value(ClientMessage::PublishPrekeys {
            bundle: Box::new(bundle),
        })
        .expect("serialize");
        assert_eq!(publish["type"], "publish_prekeys");
        assert_eq!(
            publish["bundle"]["one_time_keys"].as_array().unwrap().len(),
            3,
            "all generated one-time keys must be in the bundle"
        );

        let fetch = serde_json::to_value(ClientMessage::FetchPrekeys {
            peer_id: "deadbeef00000000".into(),
        })
        .expect("serialize");
        assert_eq!(fetch["type"], "fetch_prekeys");
        assert_eq!(fetch["peer_id"], "deadbeef00000000");
    }

    #[test]
    fn server_envelope_message_parses() {
        let text = r#"{"type":"envelope","envelope":{"sender":"a","recipient":"b","payload":"c2VjcmV0","seq":7}}"#;
        let message: ServerMessage = serde_json::from_str(text).expect("must parse");

        match message {
            ServerMessage::Envelope { envelope } => {
                assert_eq!(envelope.sender, "a");
                assert_eq!(envelope.recipient, "b");
                assert_eq!(envelope.payload, "c2VjcmV0");
                assert_eq!(envelope.seq, 7);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn server_ack_and_error_messages_parse() {
        let ack: ServerMessage = serde_json::from_str(r#"{"type":"ack","seq":3}"#).expect("parse");
        match ack {
            ServerMessage::Acknowledged { seq } => assert_eq!(seq, 3),
            other => panic!("unexpected variant: {other:?}"),
        }

        let error: ServerMessage =
            serde_json::from_str(r#"{"type":"error","code":"rate_limited"}"#).expect("parse");
        match error {
            ServerMessage::Error { code } => assert_eq!(code, "rate_limited"),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn prekeys_response_roundtrips_through_json() {
        let mut identity = Identity::new();
        let bundle = identity.pre_key_bundle(2);
        let text = serde_json::to_string(&ServerMessage::Prekeys {
            bundle: Box::new(bundle.clone()),
            display_name: None,
        })
        .expect("serialize");

        match serde_json::from_str::<ServerMessage>(&text).expect("deserialize") {
            ServerMessage::Prekeys {
                bundle: restored,
                display_name,
            } => {
                assert_eq!(*restored, bundle);
                assert_eq!(display_name, None);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn prekeys_response_parses_an_advertised_display_name() {
        let text = r#"{"type":"prekeys","bundle":{"x":1},"display_name":"Bob Prime"}"#;
        let message = serde_json::from_str::<serde_json::Value>(text).expect("valid json");
        // A missing `display_name` field must also be tolerated (old servers).
        assert!(message.get("display_name").is_some());
    }

    #[test]
    fn prekeys_response_without_display_name_field_defaults_to_none() {
        // Old relay replies omit the field entirely; serde must default it.
        let mut identity = Identity::new();
        let bundle = identity.pre_key_bundle(1);
        let bundle_json = serde_json::to_string(&bundle).expect("serialize");
        let text = format!(r#"{{"type":"prekeys","bundle":{bundle_json}}}"#);
        match serde_json::from_str::<ServerMessage>(&text) {
            Ok(ServerMessage::Prekeys { display_name, .. }) => assert_eq!(display_name, None),
            Ok(other) => panic!("unexpected variant: {other:?}"),
            Err(err) => panic!("must parse: {err}"),
        }
    }

    #[test]
    fn profile_updated_server_message_parses() {
        let message: ServerMessage =
            serde_json::from_str(r#"{"type":"profile_updated"}"#).expect("parse");
        assert!(matches!(message, ServerMessage::ProfileUpdated));
    }

    #[test]
    fn update_profile_client_message_serializes() {
        let json = serde_json::to_value(ClientMessage::UpdateProfile {
            display_name: "New Name".into(),
        })
        .expect("serialize");
        assert_eq!(json["type"], "update_profile");
        assert_eq!(json["display_name"], "New Name");
    }

    #[test]
    fn e2e_handshake_and_messages_flow_over_the_envelope_wire() {
        let alice = Identity::new();
        let mut bob = Identity::new();
        let alice_peer_id = alice.peer_id();
        let bob_peer_id = bob.peer_id();

        // Bob publishes a bundle; Alice fetches it across JSON boundaries.
        let bundle = bob.pre_key_bundle(5);
        let fetched: PreKeyBundle =
            serde_json::from_str(&serde_json::to_string(&bundle).expect("bundle must serialize"))
                .expect("bundle must deserialize");

        // Alice creates the outbound session and sends the first message.
        let mut alice_session = ChatSession::create_outbound(&alice, &fetched).expect("session");
        let first = alice_session.encrypt(b"hello bob").expect("encrypt");
        let pre_key = match first {
            OlmMessage::PreKey(pk) => pk,
            OlmMessage::Normal(_) => panic!("first message must be a pre-key message"),
        };

        // The handshake travels inside a base64 relay payload.
        let wire_out = Envelope::new(
            alice_peer_id.clone(),
            bob_peer_id.clone(),
            EnvelopeContent::Handshake(Handshake::new(alice_peer_id.clone(), pre_key)),
        );
        let relay = RelayEnvelope {
            sender: wire_out.sender_peer_id.clone(),
            recipient: wire_out.recipient_peer_id.clone(),
            payload: BASE64.encode(serde_json::to_vec(&wire_out).expect("encode")),
            seq: 1,
        };
        let wire_in: Envelope =
            serde_json::from_slice(&BASE64.decode(&relay.payload).expect("b64")).expect("decode");

        // Bob establishes the inbound session from the embedded identity key.
        let handshake = match wire_in.content {
            EnvelopeContent::Handshake(h) => h,
            _ => panic!("expected a handshake envelope"),
        };
        let their_key = handshake.pre_key_message.identity_key();
        assert_eq!(their_key, alice.curve25519_key(), "sender key must match");
        let inbound = ChatSession::create_inbound(&mut bob, their_key, &handshake.pre_key_message)
            .expect("inbound session");
        assert_eq!(inbound.plaintext, b"hello bob");
        assert_eq!(alice_session.session_id(), inbound.session.session_id());

        // Bob replies with a normal ratchet message.
        let mut bob_session = inbound.session;
        let reply = bob_session.encrypt(b"got it").expect("encrypt");
        let wire_reply = Envelope::new(
            bob_peer_id.clone(),
            alice_peer_id.clone(),
            EnvelopeContent::Message(Message::new(
                bob_peer_id.clone(),
                bob_session.session_id(),
                reply,
            )),
        );
        let relay_reply = RelayEnvelope {
            sender: wire_reply.sender_peer_id.clone(),
            recipient: wire_reply.recipient_peer_id.clone(),
            payload: BASE64.encode(serde_json::to_vec(&wire_reply).expect("encode")),
            seq: 2,
        };
        let wire_reply_in: Envelope =
            serde_json::from_slice(&BASE64.decode(&relay_reply.payload).expect("b64"))
                .expect("decode");

        let reply_message = match wire_reply_in.content {
            EnvelopeContent::Message(m) => m,
            _ => panic!("expected a message envelope"),
        };
        assert_eq!(
            alice_session
                .decrypt(&reply_message.message)
                .expect("decrypt"),
            b"got it"
        );
    }

    // ---------------------------------------------------------------------
    // Settings persistence
    // ---------------------------------------------------------------------

    /// Unique per-test settings file path; tests run in parallel threads so
    /// they must never share a file.
    static SETTINGS_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_settings_path() -> PathBuf {
        let n = SETTINGS_TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "whisper-settings-test-{}-{n}.json",
            std::process::id()
        ))
    }

    #[test]
    fn settings_roundtrip_via_file_preserves_present_fields() {
        let path = temp_settings_path();
        write_settings_file(
            &path,
            &Settings {
                relay_url: Some("ws://relay.example".into()),
                theme: None,
            },
        )
        .expect("write");

        let loaded = read_settings_file(&path).expect("read");
        assert_eq!(loaded.relay_url.as_deref(), Some("ws://relay.example"));
        assert_eq!(loaded.theme, None);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn settings_missing_file_loads_as_defaults() {
        let path = temp_settings_path();
        let _ = std::fs::remove_file(&path);

        let loaded = read_settings_file(&path).expect("missing file must not error");
        assert_eq!(loaded.relay_url, None);
        assert_eq!(loaded.theme, None);
    }

    #[test]
    fn settings_parse_handles_missing_fields() {
        let settings: Settings =
            serde_json::from_str(r#"{"theme":"dark"}"#).expect("partial file must parse");
        assert_eq!(settings.relay_url, None);
        assert_eq!(settings.theme.as_deref(), Some("dark"));
    }

    #[test]
    fn settings_corrupt_file_loads_as_defaults() {
        let path = temp_settings_path();
        std::fs::write(&path, b"not json").expect("write corrupt file");

        let loaded = read_settings_file(&path).expect("corrupt file must not error");
        assert_eq!(loaded.relay_url, None);
        assert_eq!(loaded.theme, None);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn relay_url_resolution_prefers_settings_then_env_then_default() {
        let custom = Settings {
            relay_url: Some("ws://custom".into()),
            theme: None,
        };
        assert_eq!(resolve_relay_url(&custom, Some("ws://env")), "ws://custom");

        let blank = Settings {
            relay_url: Some(String::new()),
            theme: None,
        };
        assert_eq!(resolve_relay_url(&blank, Some("ws://env")), "ws://env");
        assert_eq!(resolve_relay_url(&blank, None), DEFAULT_RELAY_URL);

        let defaults = Settings::default();
        assert_eq!(resolve_relay_url(&defaults, Some("ws://env")), "ws://env");
        assert_eq!(resolve_relay_url(&defaults, None), DEFAULT_RELAY_URL);
    }

    // ---------------------------------------------------------------------
    // Ack -> delivered bookkeeping
    // ---------------------------------------------------------------------

    /// The pure half of `handle_ack`: the envelope seq must resolve back to the
    /// client id that `send_message` registered, and that message's status must
    /// flip to "delivered". (The `message-status` event emission itself needs a
    /// live Tauri app and is exercised by the end-to-end smoke test.)
    #[test]
    fn acknowledged_seq_resolves_to_client_id_and_flips_message_to_delivered() {
        let mut pending: HashMap<u64, String> = HashMap::new();
        pending.insert(7, "client-1".into());

        let mut messages: HashMap<String, Vec<UIMessage>> = HashMap::new();
        messages.insert(
            "peer-1".into(),
            vec![UIMessage {
                id: "client-1".into(),
                text: "hello".into(),
                outgoing: true,
                timestamp: 0,
                status: "sent".into(),
            }],
        );

        let client_id = pending.remove(&7).expect("seq must resolve to a client id");
        assert_eq!(client_id, "client-1");
        assert!(apply_delivered(&mut messages, &client_id));
        assert_eq!(messages["peer-1"][0].status, "delivered");
    }

    #[test]
    fn unknown_ack_seq_leaves_message_status_untouched() {
        let mut messages: HashMap<String, Vec<UIMessage>> = HashMap::new();
        messages.insert(
            "peer-1".into(),
            vec![UIMessage {
                id: "client-1".into(),
                text: "hello".into(),
                outgoing: true,
                timestamp: 0,
                status: "sent".into(),
            }],
        );

        assert!(!apply_delivered(&mut messages, "ghost"));
        assert_eq!(messages["peer-1"][0].status, "sent");
    }

    #[test]
    fn late_ack_does_not_downgrade_an_already_read_message() {
        // A read receipt can beat the relay's own ack back to the sender; the
        // ack must then be a no-op instead of flipping "read" back to
        // "delivered".
        let mut messages: HashMap<String, Vec<UIMessage>> = HashMap::new();
        messages.insert(
            "peer-1".into(),
            vec![UIMessage {
                id: "client-1".into(),
                text: "hello".into(),
                outgoing: true,
                timestamp: 0,
                status: "read".into(),
            }],
        );

        assert!(!apply_delivered(&mut messages, "client-1"));
        assert_eq!(messages["peer-1"][0].status, "read");
    }

    #[test]
    fn ack_for_already_delivered_message_is_a_noop() {
        let mut messages: HashMap<String, Vec<UIMessage>> = HashMap::new();
        messages.insert(
            "peer-1".into(),
            vec![UIMessage {
                id: "client-1".into(),
                text: "hello".into(),
                outgoing: true,
                timestamp: 0,
                status: "delivered".into(),
            }],
        );

        assert!(!apply_delivered(&mut messages, "client-1"));
        assert_eq!(messages["peer-1"][0].status, "delivered");
    }

    #[test]
    fn envelope_serialization_carries_the_allocated_seq() {
        // The ack contract depends on the on-wire envelope seq matching the one
        // registered in pending_acks; send_wire must forward it verbatim.
        let envelope = RelayEnvelope {
            sender: "alice".into(),
            recipient: "bob".into(),
            payload: "c2VjcmV0".into(),
            seq: 42,
        };
        let json = serde_json::to_value(ClientMessage::Envelope { envelope })
            .expect("envelope must serialize");
        assert_eq!(json["type"], "envelope");
        assert_eq!(json["envelope"]["seq"], 42);
    }

    // ---------------------------------------------------------------------
    // Read receipt bookkeeping
    // ---------------------------------------------------------------------

    #[test]
    fn apply_read_flips_outgoing_peer_messages_and_returns_their_ids() {
        let mut messages: HashMap<String, Vec<UIMessage>> = HashMap::new();
        messages.insert(
            "peer-1".into(),
            vec![
                UIMessage {
                    id: "sent-1".into(),
                    text: "a".into(),
                    outgoing: true,
                    timestamp: 0,
                    status: "sent".into(),
                },
                UIMessage {
                    id: "delivered-1".into(),
                    text: "b".into(),
                    outgoing: true,
                    timestamp: 0,
                    status: "delivered".into(),
                },
            ],
        );
        // A different peer's messages must be left alone.
        messages.insert(
            "peer-2".into(),
            vec![UIMessage {
                id: "delivered-2".into(),
                text: "c".into(),
                outgoing: true,
                timestamp: 0,
                status: "delivered".into(),
            }],
        );

        let flipped = apply_read(&mut messages, "peer-1");
        assert_eq!(flipped, vec!["sent-1", "delivered-1"]);
        assert_eq!(messages["peer-1"][0].status, "read");
        assert_eq!(messages["peer-1"][1].status, "read");
        assert_eq!(messages["peer-2"][0].status, "delivered");
    }

    #[test]
    fn apply_read_skips_incoming_and_already_read_messages() {
        let mut messages: HashMap<String, Vec<UIMessage>> = HashMap::new();
        messages.insert(
            "peer-1".into(),
            vec![
                UIMessage {
                    id: "in-1".into(),
                    text: "incoming".into(),
                    outgoing: false,
                    timestamp: 0,
                    status: "delivered".into(),
                },
                UIMessage {
                    id: "read-1".into(),
                    text: "already read".into(),
                    outgoing: true,
                    timestamp: 0,
                    status: "read".into(),
                },
            ],
        );

        let flipped = apply_read(&mut messages, "peer-1");
        assert!(flipped.is_empty());
        assert_eq!(messages["peer-1"][0].status, "delivered");
        assert_eq!(messages["peer-1"][1].status, "read");
    }

    #[test]
    fn apply_read_unknown_peer_is_a_noop() {
        let mut messages: HashMap<String, Vec<UIMessage>> = HashMap::new();
        messages.insert(
            "peer-1".into(),
            vec![UIMessage {
                id: "out-1".into(),
                text: "hi".into(),
                outgoing: true,
                timestamp: 0,
                status: "delivered".into(),
            }],
        );

        assert!(apply_read(&mut messages, "ghost").is_empty());
        assert_eq!(messages["peer-1"][0].status, "delivered");
    }

    // ---------------------------------------------------------------------
    // Receipt transport (encrypted inside a Message)
    // ---------------------------------------------------------------------

    #[test]
    fn receipt_transport_roundtrips_inside_the_ratchet_session() {
        // A receipt is serialized as EnvelopeContent, encrypted like any
        // message, and recovered by parsing the decrypted plaintext.
        let alice = Identity::new();
        let mut bob = Identity::new();
        let bundle = bob.pre_key_bundle(5);
        let mut alice_session = ChatSession::create_outbound(&alice, &bundle).expect("session");
        let first = alice_session.encrypt(b"hello bob").expect("encrypt");
        let pre_key = match first {
            OlmMessage::PreKey(pk) => pk,
            OlmMessage::Normal(_) => panic!("first message must be a pre-key message"),
        };
        let inbound = ChatSession::create_inbound(&mut bob, alice.curve25519_key(), &pre_key)
            .expect("inbound session");
        let mut bob_session = inbound.session;

        // Bob sends a read receipt back to Alice.
        let content = EnvelopeContent::Receipt {
            kind: ReceiptKind::Read,
        };
        let payload = serde_json::to_vec(&content).expect("serialize receipt");
        let ciphertext = bob_session.encrypt(payload).expect("encrypt receipt");
        let plaintext = alice_session.decrypt(&ciphertext).expect("decrypt receipt");

        let restored: EnvelopeContent = serde_json::from_slice(&plaintext).expect("parse");
        assert_eq!(
            restored,
            EnvelopeContent::Receipt {
                kind: ReceiptKind::Read
            }
        );
    }

    #[test]
    fn display_name_validation_rejects_control_characters_and_oversize_names() {
        let too_long = "x".repeat(MAX_DISPLAY_NAME_CHARS + 1);
        assert!(too_long.chars().count() > MAX_DISPLAY_NAME_CHARS);
        assert!("name\nwith\ttabs".chars().any(char::is_control));
    }
}
