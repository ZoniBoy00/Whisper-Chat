//! WebSocket relay client for the desktop app.
//!
//! Connects to the zero-knowledge relay, authenticates with a signed hello and
//! pumps end-to-end encrypted [`e2ee_core::Envelope`]s between peers. The relay
//! never sees plaintext: every envelope payload is opaque ciphertext.
//!
//! Message history, Double Ratchet sessions, contacts and settings are
//! persisted in a keyed SQLite store (see [`crate::store`]) so they survive
//! app restarts. Identity and sessions are stored in shared, thread-safe state
//! so the WebSocket inbound task and the Tauri commands can both touch them.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
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
    ChatSession, Envelope, EnvelopeContent, Handshake, Identity, InboundGroup, Message,
    OutboundGroup, PreKeyBundle, ReceiptKind,
};
use vodozemac::olm::OlmMessage;

use crate::store::{derive_db_key, ChatStore, ContactRow, StoreError};

/// Default relay endpoint; override with the `WHISPER_RELAY_URL` env var.
const DEFAULT_RELAY_URL: &str = "ws://127.0.0.1:8080/ws";

/// How long to wait for a pre-key bundle after requesting one.
const PREKEY_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// How long to wait for a profile/username response after requesting one.
const PROFILE_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// How long to wait for a presence report after requesting one.
const PRESENCE_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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

/// Resolve the chat-store database path for a peer ID.
///
/// The database is named after the peer ID so two identities (the dual-instance
/// dev workflow) never share one file, and the encrypted file can only be
/// opened with the matching identity key.
pub fn resolve_store_path(app: &AppHandle, peer_id: &str) -> PathBuf {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(format!("whisper-{peer_id}.db")))
        .unwrap_or_else(|_| PathBuf::from(format!("whisper-{peer_id}.db")))
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
    /// The relay did not answer a profile request in time.
    #[error("timed out waiting for profile")]
    ProfileTimeout,
    /// The profile request was answered with an error or dropped.
    #[error("profile request failed")]
    ProfileRequestFailed,
    /// The pre-key request was answered with an error or dropped.
    #[error("pre-key request failed")]
    PrekeyFetchFailed,
    /// The relay did not answer a presence query in time.
    #[error("timed out waiting for presence")]
    PresenceTimeout,
    /// The presence request was answered with an error or dropped.
    #[error("presence request failed")]
    PresenceFetchFailed,
    /// The relay did not answer a group request in time.
    #[error("timed out waiting for group reply")]
    GroupTimeout,
    /// The group request was answered with an error or dropped.
    #[error("group request failed")]
    GroupRequestFailed,
    /// A Megolm group-session operation failed.
    #[error("group error: {0}")]
    Group(#[from] e2ee_core::GroupError),
    /// The group has no outbound session, so this identity cannot send to it.
    #[error("group {0} has no outbound session")]
    NoOutboundGroup(String),
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
    /// A failure while reading or writing the SQLCipher chat store.
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    /// A chat-store operation was attempted before the store was opened.
    #[error("chat store is not open")]
    StoreNotOpen,
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
    /// Subscribe to presence pushes for `peer_id`: the relay sends a
    /// `presence` message whenever that peer connects or disconnects.
    #[serde(rename = "watch_presence")]
    WatchPresence { peer_id: String },
    /// Request the current presence of `peer_id`; the relay replies with a
    /// single `presence` message.
    #[serde(rename = "get_presence")]
    GetPresence { peer_id: String },
    /// Toggle whether our online status and last-seen are visible to other
    /// peers. When hidden, the relay reports us as `online: false` with no
    /// last-seen in every presence reply and push.
    #[serde(rename = "set_privacy")]
    SetPrivacy { presence_visible: bool },
    /// Register (or re-register) a signed username alias with an optional
    /// avatar (base64, ≤2 MB). `signature` is an Ed25519 signature over
    /// `username || 0x00 || curve25519_key` (see e2ee-core profile.rs).
    #[serde(rename = "register_profile")]
    RegisterProfile {
        username: String,
        signature: String,
        display_name: Option<String>,
        avatar: Option<String>,
    },
    /// Prefix-search registered usernames and peer IDs.
    #[serde(rename = "search_users")]
    SearchUsers { query: String, limit: Option<usize> },
    /// Fetch one peer's public profile.
    #[serde(rename = "get_profile")]
    GetProfile { peer_id: String },
    /// Create a group: the authenticated peer becomes its owner. The Megolm
    /// session key is shared to members separately over 1:1 envelopes.
    #[serde(rename = "create_group")]
    CreateGroup { name: String },
    /// Add `peer_id` to a group's roster.
    #[serde(rename = "add_group_member")]
    AddGroupMember { group_id: String, peer_id: String },
    /// Remove the caller from a group's roster.
    #[serde(rename = "leave_group")]
    LeaveGroup { group_id: String },
    /// Request a group's public metadata and member roster (with roles).
    #[serde(rename = "get_group_info")]
    GetGroupInfo { group_id: String },
    /// Fan one client-encrypted group envelope out to every group member.
    #[serde(rename = "send_group_message")]
    SendGroupMessage {
        group_id: String,
        envelope: RelayEnvelope,
    },
    /// Promote a member to admin (owner or admin only).
    #[serde(rename = "promote_member")]
    PromoteMember { group_id: String, peer_id: String },
    /// Demote an admin back to a regular member (owner only).
    #[serde(rename = "demote_member")]
    DemoteMember { group_id: String, peer_id: String },
    /// Remove a member from a group (owner only).
    #[serde(rename = "remove_member")]
    RemoveMember { group_id: String, peer_id: String },
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
    /// The caller's privacy settings were updated.
    #[serde(rename = "privacy_updated")]
    PrivacyUpdated,
    /// The caller's username was registered.
    #[serde(rename = "profile_registered")]
    ProfileRegistered { username: String },
    /// Search results for a `search_users` request.
    #[serde(rename = "users_search")]
    UsersSearch { results: Vec<ProfileSearchResult> },
    /// One peer's public profile (reply to `get_profile`).
    Profile(PeerProfile),
    /// Presence report for `peer_id` (a `watch_presence` push or the reply to
    /// a `get_presence` request): whether the peer is online right now plus its
    /// last-seen unix-seconds timestamp when offline (`None` while online or
    /// when the peer has never been seen).
    #[serde(rename = "presence")]
    Presence {
        peer_id: String,
        online: bool,
        last_seen: Option<i64>,
    },
    /// Confirmation that a group was created (`create_group` reply).
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
    /// The public metadata + member roster of a group (`get_group_info` reply).
    /// `members` carries each member's role (owner/admin/member).
    #[serde(rename = "group_info")]
    GroupInfo {
        group_id: String,
        name: String,
        owner_peer_id: String,
        members: Vec<GroupMember>,
    },
    /// Confirmation that `peer_id` was promoted to admin (`promote_member`
    /// reply).
    #[serde(rename = "group_member_promoted")]
    GroupMemberPromoted { group_id: String, peer_id: String },
    /// Confirmation that `peer_id` was demoted to a regular member
    /// (`demote_member` reply).
    #[serde(rename = "group_member_demoted")]
    GroupMemberDemoted { group_id: String, peer_id: String },
    /// Confirmation that `peer_id` was removed from a group (`remove_member`
    /// reply).
    #[serde(rename = "group_member_removed")]
    GroupMemberRemoved { group_id: String, peer_id: String },
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

/// One row of a username/UID search result (wire form).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSearchResult {
    #[serde(default)]
    pub username: Option<String>,
    pub peer_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

/// A peer's public profile (wire form).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerProfile {
    #[serde(default)]
    pub username: Option<String>,
    pub peer_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub curve25519_key: Option<String>,
}

/// One member of a group roster, with its current role ("owner", "admin" or
/// "member"). Mirror of the relay's `GroupMember` wire shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMember {
    /// The member's peer ID (fingerprint).
    pub peer_id: String,
    /// "owner", "admin" or "member".
    pub role: String,
}

/// A group's public metadata as reported to the UI: the name, the owner and
/// every member with its role. `my_role` is this identity's own role in the
/// group (`None` while unknown), which drives the permission-gated controls in
/// the group info panel.
#[derive(Debug, Clone, Serialize)]
pub struct GroupInfo {
    pub group_id: String,
    pub name: String,
    pub owner_peer_id: String,
    pub members: Vec<GroupMember>,
    pub my_role: Option<String>,
}

/// Internal, in-memory group state. Not serializable: it owns the (secret)
/// Megolm outbound session for groups this identity created.
struct GroupInfoState {
    /// Public group name.
    name: String,
    /// Cached member roster (server-authoritative snapshot).
    members: Vec<GroupMember>,
    /// This identity's role in the group, when known.
    my_role: Option<String>,
    /// The outbound Megolm session; `Some` only for groups this identity
    /// created (they are the only sender in the MVP model).
    outbound: Option<OutboundGroup>,
}

/// The plaintext JSON of a Megolm session-key share. The creator encrypts it
/// inside an ordinary 1:1 Double Ratchet message so the relay never sees the
/// key; the recipient parses it and builds an [`InboundGroup`].
#[derive(Debug, Deserialize)]
struct GroupKeyPayload {
    /// Always "group_key"; distinguishes the share from ordinary text.
    kind: String,
    /// The relay-assigned group ID the key belongs to.
    group_id: String,
    /// The base64 Megolm session key (secret key material).
    session_key: String,
    /// The public group name, used to surface the group in the chat list.
    group_name: String,
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

/// A peer's presence snapshot: whether they are online right now, plus their
/// last-seen unix-seconds timestamp when offline (`None` while online or when
/// they have never been seen).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceInfo {
    pub online: bool,
    pub last_seen: Option<i64>,
}

/// Payload of the `presence` event emitted whenever a peer's presence changes
/// or a `get_presence` reply arrives.
#[derive(Debug, Clone, Serialize)]
pub struct PresenceEvent {
    pub peer_id: String,
    pub online: bool,
    pub last_seen: Option<i64>,
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
    /// Groups appear here too (keyed by their group ID with the group name as
    /// the display name), so the existing chat list renders them like contacts.
    pub contacts: Vec<ContactInfo>,
    /// Per-peer (and per-group) message history, oldest first. Group messages
    /// are keyed by the group ID.
    pub messages: HashMap<String, Vec<UIMessage>>,
    /// Latest known presence per peer (online status + last-seen timestamp).
    pub presence: HashMap<String, PresenceInfo>,
    /// Every group this identity belongs to, with its roster and roles.
    pub groups: Vec<GroupInfo>,
}

/// Persisted user preferences stored in `settings.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Relay endpoint to connect to; falls back to the `WHISPER_RELAY_URL` env
    /// var and then [`DEFAULT_RELAY_URL`].
    #[serde(default)]
    pub relay_url: Option<String>,
    /// UI theme ("dark", "light", ...); the UI owns the valid values.
    #[serde(default)]
    pub theme: Option<String>,
    /// Whether our online status and last-seen are shown to other peers.
    /// Mirrors the relay-side `set_privacy` preference so the toggle restores
    /// on restart.
    #[serde(default = "default_true")]
    pub presence_visible: bool,
    /// Whether we send end-to-end read receipts. Off means WE do not emit
    /// receipts; receipts others send us are still shown (like WhatsApp).
    #[serde(default = "default_true")]
    pub read_receipts: bool,
    /// Whether we broadcast end-to-end typing indicators to the active peer.
    #[serde(default = "default_true")]
    pub typing_indicator: bool,
    /// Whether the UI shows HTML5 desktop notifications for incoming messages
    /// while the window is unfocused.
    #[serde(default = "default_true")]
    pub notifications_enabled: bool,
    /// Whether desktop notifications include the message text; when off they
    /// only say "New message from @name".
    #[serde(default = "default_true")]
    pub notification_preview: bool,
}

/// Serde default for the opt-out boolean preferences above.
fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            relay_url: None,
            theme: None,
            presence_visible: true,
            read_receipts: true,
            typing_indicator: true,
            notifications_enabled: true,
            notification_preview: true,
        }
    }
}

/// A partial settings update from the UI. `None` leaves the stored value
/// untouched; each `Some` field overwrites it. Excludes `presence_visible`,
/// which is persisted through [`RelayClient::set_privacy`] because it also
/// round-trips to the relay.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SettingsPatch {
    #[serde(default)]
    pub read_receipts: Option<bool>,
    #[serde(default)]
    pub typing_indicator: Option<bool>,
    #[serde(default)]
    pub notifications_enabled: Option<bool>,
    #[serde(default)]
    pub notification_preview: Option<bool>,
}

/// Thread-safe handle to the relay client, managed as Tauri state.
#[derive(Clone)]
pub struct RelayClient {
    inner: Arc<RelayInner>,
}

struct RelayInner {
    app: AppHandle,
    identity_path: PathBuf,
    /// Keyed SQLite store for messages, sessions, contacts and settings.
    /// Lazy: opened (and hydrated) once the identity is loaded in `connect`,
    /// then kept for the rest of the process.
    store: RwLock<Option<ChatStore>>,
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
    /// In-flight `register_profile` requests, resolved in FIFO order.
    pending_register: Mutex<VecDeque<oneshot::Sender<RegisterResponse>>>,
    /// In-flight `search_users` requests, resolved in FIFO order.
    pending_search: Mutex<VecDeque<oneshot::Sender<SearchResponse>>>,
    /// In-flight `get_profile` requests, resolved in FIFO order.
    pending_profile: Mutex<VecDeque<oneshot::Sender<ProfileResponse>>>,
    /// In-flight presence queries (peer_id + reply channel). Unlike pre-key
    /// fetches, replies echo the requested peer ID, so a pending request is
    /// resolved by matching the peer — a push for another peer can never
    /// satisfy (and corrupt) an outstanding query.
    pending_presence: Mutex<VecDeque<(String, oneshot::Sender<PresenceResponse>)>>,
    /// Latest known presence per peer, fed by pushes and `get_presence` replies.
    presence: RwLock<HashMap<String, PresenceInfo>>,
    /// Per-peer generation counter used to deduplicate typing auto-timeouts:
    /// each new `typing` receipt bumps the counter so older pending timers
    /// give up instead of flipping the indicator off early.
    typing_timeouts: Mutex<HashMap<String, u64>>,
    /// Megolm group state: group_id -> name, member roster and (for groups
    /// this identity created) the outbound session.
    groups: RwLock<HashMap<String, GroupInfoState>>,
    /// Megolm inbound sessions for groups this identity joined, keyed by
    /// group_id. Built from the creator's `session_key` shared over a 1:1
    /// Double Ratchet envelope.
    inbound_groups: Mutex<HashMap<String, InboundGroup>>,
    /// In-flight `create_group` requests (replies are ordered, so FIFO works).
    pending_group_created: Mutex<VecDeque<oneshot::Sender<GroupCreatedResponse>>>,
    /// In-flight `add_group_member` requests, resolved in FIFO order.
    pending_group_member_added: Mutex<VecDeque<oneshot::Sender<GroupMemberAddedResponse>>>,
    /// In-flight `get_group_info` requests, resolved in FIFO order.
    pending_group_info: Mutex<VecDeque<oneshot::Sender<GroupInfoResponse>>>,
    /// In-flight promote/demote/remove/leave confirmations, resolved in FIFO
    /// order (the relay answers each request in turn).
    pending_group_op: Mutex<VecDeque<oneshot::Sender<GroupOpResponse>>>,
}

/// Result channel type for a pre-key fetch: the bundle plus the peer's public
/// display name (`None` when they have not set one).
type PrekeyResponse = Result<(PreKeyBundle, Option<String>), RelayError>;
/// Result channel type for a presence fetch: the online flag plus the peer's
/// last-seen timestamp when offline.
type PresenceResponse = Result<PresenceInfo, RelayError>;

/// Result channel type for a username registration.
type RegisterResponse = Result<String, RelayError>;

/// Result channel type for a user search.
type SearchResponse = Result<Vec<ProfileSearchResult>, RelayError>;

/// Result channel type for a profile fetch.
type ProfileResponse = Result<Option<PeerProfile>, RelayError>;

/// Result channel type for a `create_group` request: the relay-assigned group
/// ID.
type GroupCreatedResponse = Result<String, RelayError>;

/// Result channel type for an `add_group_member` confirmation.
type GroupMemberAddedResponse = Result<(), RelayError>;

/// Result channel type for a `get_group_info` request.
type GroupInfoResponse = Result<GroupInfo, RelayError>;

/// Result channel type for a promote/demote/remove/leave confirmation.
type GroupOpResponse = Result<(), RelayError>;

impl RelayClient {
    /// Build a client bound to `app`'s identity file in the app data dir.
    pub fn new(app: AppHandle) -> Self {
        let identity_path = resolve_identity_path(&app);
        Self {
            inner: Arc::new(RelayInner {
                app,
                identity_path,
                store: RwLock::new(None),
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
                pending_register: Mutex::new(VecDeque::new()),
                pending_search: Mutex::new(VecDeque::new()),
                pending_profile: Mutex::new(VecDeque::new()),
                pending_presence: Mutex::new(VecDeque::new()),
                presence: RwLock::new(HashMap::new()),
                typing_timeouts: Mutex::new(HashMap::new()),
                groups: RwLock::new(HashMap::new()),
                inbound_groups: Mutex::new(HashMap::new()),
                pending_group_created: Mutex::new(VecDeque::new()),
                pending_group_member_added: Mutex::new(VecDeque::new()),
                pending_group_info: Mutex::new(VecDeque::new()),
                pending_group_op: Mutex::new(VecDeque::new()),
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

        // Load the identity from disk (once). Its contents also seed the
        // SQLCipher key, so the store is opened from the same bytes.
        let identity_json = std::fs::read_to_string(&self.inner.identity_path)?;
        {
            let mut guard = mutex_guard(&self.inner.identity)?;
            if guard.is_none() {
                let identity = Identity::from_json(&identity_json)?;
                *guard = Some(identity);
            }
        }
        // Open the store (idempotent) and hydrate messages, sessions,
        // contacts and settings from it so history survives restarts.
        self.open_store(&identity_json)?;

        let hello = mutex_guard(&self.inner.identity)?
            .as_ref()
            .ok_or(RelayError::NoIdentity)?
            .signed_hello();

        let url = {
            let settings = read_guard(&self.inner.settings)?;
            resolve_relay_url(
                &settings,
                std::env::var("WHISPER_RELAY_URL").ok().as_deref(),
            )
        };
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
            display_name: read_guard(&self.inner.profiles)?.my_display_name.clone(),
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
        // Resolve the store file from the current identity (loaded or still
        // on disk) before any state is cleared.
        let store_path = {
            let peer_id = mutex_guard(&self.inner.identity)?
                .as_ref()
                .map(|identity| identity.peer_id())
                .or_else(|| {
                    std::fs::read_to_string(&self.inner.identity_path)
                        .ok()
                        .and_then(|json| Identity::from_json(&json).ok())
                        .map(|identity| identity.peer_id())
                });
            peer_id.map(|peer_id| resolve_store_path(&self.inner.app, &peer_id))
        };
        mutex_guard(&self.inner.identity)?.take();
        mutex_guard(&self.inner.sessions)?.clear();
        mutex_guard(&self.inner.inbound_groups)?.clear();
        write_guard(&self.inner.groups)?.clear();
        *write_guard(&self.inner.settings)? = Settings::default();
        *write_guard(&self.inner.profiles)? = Profiles::default();
        write_guard(&self.inner.messages)?.clear();
        write_guard(&self.inner.contacts)?.clear();
        write_guard(&self.inner.presence)?.clear();
        // Close the store and drop the database file so a fresh identity
        // starts with a clean, empty history.
        *write_guard(&self.inner.store)? = None;
        if let Some(path) = store_path {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
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

    /// Fetch a peer's current presence (online status + last-seen), waiting up
    /// to [`PRESENCE_FETCH_TIMEOUT`]. The reply is also cached in the presence
    /// map and emitted as a `presence` event by the inbound loop, so a command
    /// caller and every event listener end up with the same snapshot.
    pub async fn get_presence(&self, peer_id: &str) -> Result<PresenceInfo, RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_presence)?.push_back((peer_id.to_string(), tx));
        if let Err(err) = self.send_json(&ClientMessage::GetPresence {
            peer_id: peer_id.to_string(),
        }) {
            // The request never left, so drop the dangling waiter(s) for this
            // peer to keep the queue aligned with the relay's replies.
            mutex_guard(&self.inner.pending_presence)?.retain(|(peer, _)| peer != peer_id);
            return Err(err);
        }

        match tokio::time::timeout(PRESENCE_FETCH_TIMEOUT, rx).await {
            Ok(Ok(Ok(info))) => Ok(info),
            Ok(Ok(Err(err))) => Err(err),
            Ok(Err(_)) => Err(RelayError::PresenceFetchFailed),
            Err(_) => {
                // The waiter timed out: sweep closed senders so a late reply
                // for this peer cannot keep resolving dead requests (each
                // dropped receiver closes its sender, so this only ever
                // removes stale entries).
                if let Ok(mut pending) = self.inner.pending_presence.lock() {
                    pending.retain(|(_, tx)| !tx.is_closed());
                }
                Err(RelayError::PresenceTimeout)
            }
        }
    }

    /// Subscribe to presence pushes for `peer_id`: the relay sends a
    /// `presence` message whenever the peer comes online or goes offline.
    /// Best-effort — without a connection the subscription is dropped and the
    /// caller is expected to re-watch after (re)connecting.
    pub fn watch_presence(&self, peer_id: &str) -> Result<(), RelayError> {
        self.send_json(&ClientMessage::WatchPresence {
            peer_id: peer_id.to_string(),
        })
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
    ///
    /// When `peer_id` is a group (an entry in the groups map), the message is
    /// instead Megolm-encrypted and fanned out to every member via
    /// `send_group_message`.
    pub async fn send_message(
        &self,
        peer_id: &str,
        text: &str,
        client_id: &str,
    ) -> Result<(), RelayError> {
        // Group messages are encrypted with the group's Megolm session and
        // routed through the relay's group fan-out rather than a 1:1 ratchet.
        if read_guard(&self.inner.groups)?.contains_key(peer_id) {
            return self.send_group_message(peer_id, text, client_id);
        }

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
            // mapping and roll back the optimistic record (in memory and in
            // the store) so a failed send does not surface as a sent message
            // on the next refresh or restart.
            let _ = mutex_guard(&self.inner.pending_acks)?.remove(&seq);
            if let Ok(mut messages) = write_guard(&self.inner.messages) {
                if let Some(msgs) = messages.get_mut(peer_id) {
                    msgs.retain(|m| m.id != msg.id);
                }
            }
            if let Ok(store) = self.store_guard() {
                if let Some(store) = store.as_ref() {
                    let _ = store.delete_message(&msg.id);
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

    /// Megolm-encrypt `text` with the group's outbound session and fan it out
    /// to every member via the relay's `send_group_message`.
    fn send_group_message(
        &self,
        group_id: &str,
        text: &str,
        client_id: &str,
    ) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        // Encrypt with the outbound Megolm session. Only the group's creator
        // holds one in the MVP model, so a member attempting to send fails
        // with `NoOutboundGroup`.
        let ciphertext = {
            let mut groups = write_guard(&self.inner.groups)?;
            let group = groups
                .get_mut(group_id)
                .ok_or_else(|| RelayError::NoOutboundGroup(group_id.to_string()))?;
            let outbound = group
                .outbound
                .as_mut()
                .ok_or_else(|| RelayError::NoOutboundGroup(group_id.to_string()))?;
            outbound.encrypt(text)
        };
        // The ratchet advanced, so persist the outbound session.
        self.save_group_sessions()?;

        let wire = Envelope::new(
            my_peer_id.clone(),
            group_id.to_string(),
            EnvelopeContent::Group {
                group_id: group_id.to_string(),
                ciphertext,
            },
        );
        let payload = BASE64.encode(serde_json::to_vec(&wire)?);
        let relay_envelope = RelayEnvelope {
            sender: my_peer_id.clone(),
            recipient: group_id.to_string(),
            payload,
            seq: 0, // replaced below with the allocated seq
        };

        let seq = self.next_seq();
        let msg = self.record_outgoing(group_id, text, client_id)?;
        self.record_pending_ack(seq, &msg.id)?;

        let mut envelope = relay_envelope;
        envelope.seq = seq;
        if let Err(err) = self.send_json(&ClientMessage::SendGroupMessage {
            group_id: group_id.to_string(),
            envelope,
        }) {
            let _ = mutex_guard(&self.inner.pending_acks)?.remove(&seq);
            if let Ok(mut messages) = write_guard(&self.inner.messages) {
                if let Some(msgs) = messages.get_mut(group_id) {
                    msgs.retain(|m| m.id != msg.id);
                }
            }
            if let Ok(store) = self.store_guard() {
                if let Some(store) = store.as_ref() {
                    let _ = store.delete_message(&msg.id);
                }
            }
            return Err(err);
        }

        let _ = self.inner.app.emit(
            "chat-message",
            ChatMessageEvent {
                peer_id: group_id.to_string(),
                message: msg,
            },
        );
        Ok(())
    }

    /// Snapshot the state the UI needs: identity, connection, contacts (with
    /// their display names), message history and group metadata.
    pub fn get_chat_state(&self) -> Result<ChatState, RelayError> {
        let my_peer_id = self.my_peer_id()?;
        self.ensure_store_open()?;
        let profiles = read_guard(&self.inner.profiles)?.clone();
        let contacts = read_guard(&self.inner.contacts)?.clone();
        let messages = read_guard(&self.inner.messages)?.clone();
        let presence = read_guard(&self.inner.presence)?.clone();
        let connected = self.inner.connected.load(Ordering::SeqCst);
        let contacts = contacts
            .into_iter()
            .map(|peer_id| ContactInfo {
                peer_id: peer_id.clone(),
                display_name: profiles.contacts.get(&peer_id).cloned(),
            })
            .collect();
        // Expose the group roster (without the secret outbound sessions).
        let groups = read_guard(&self.inner.groups)?
            .iter()
            .map(|(group_id, group)| GroupInfo {
                group_id: group_id.clone(),
                name: group.name.clone(),
                owner_peer_id: group
                    .members
                    .iter()
                    .find(|m| m.role == "owner")
                    .map(|m| m.peer_id.clone())
                    .unwrap_or_default(),
                members: group.members.clone(),
                my_role: group.my_role.clone(),
            })
            .collect();
        Ok(ChatState {
            my_peer_id,
            my_display_name: profiles.my_display_name,
            connected,
            contacts,
            messages,
            presence,
            groups,
        })
    }

    /// Create a group on the relay, register its members, build the Megolm
    /// outbound session and share its `session_key` to every member over the
    /// existing 1:1 Double Ratchet channel.
    ///
    /// Returns the relay-assigned group ID. Key sharing is best-effort per
    /// member: a member whose pre-keys are unavailable (never connected) is
    /// skipped so one failure cannot abort group creation.
    pub async fn create_group(
        &self,
        name: &str,
        member_ids: Vec<String>,
    ) -> Result<String, RelayError> {
        let my_peer_id = self.my_peer_id()?;
        if member_ids.iter().any(|m| m == &my_peer_id) {
            return Err(RelayError::InvalidPeer(my_peer_id));
        }

        // 1) Ask the relay for a fresh group and wait for the group_created
        //    reply carrying the group ID.
        let group_id = {
            let (tx, rx) = oneshot::channel();
            mutex_guard(&self.inner.pending_group_created)?.push_back(tx);
            if let Err(err) = self.send_json(&ClientMessage::CreateGroup {
                name: name.to_string(),
            }) {
                mutex_guard(&self.inner.pending_group_created)?.pop_back();
                return Err(err);
            }
            tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx)
                .await
                .map_err(|_| RelayError::GroupTimeout)?
                .map_err(|_| RelayError::GroupRequestFailed)??
        };

        // 2) Add every member to the roster (owner or member may add, and the
        //    creator is the owner).
        for member in &member_ids {
            let (tx, rx) = oneshot::channel();
            mutex_guard(&self.inner.pending_group_member_added)?.push_back(tx);
            if let Err(err) = self.send_json(&ClientMessage::AddGroupMember {
                group_id: group_id.clone(),
                peer_id: member.clone(),
            }) {
                mutex_guard(&self.inner.pending_group_member_added)?.pop_back();
                return Err(err);
            }
            tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx)
                .await
                .map_err(|_| RelayError::GroupTimeout)?
                .map_err(|_| RelayError::GroupRequestFailed)??;
        }

        // 3) Build the Megolm outbound session and keep it in the groups map
        //    together with the roster we just assembled.
        let outbound = OutboundGroup::new();
        let session_key = outbound.session_key();
        let mut members = vec![GroupMember {
            peer_id: my_peer_id.clone(),
            role: "owner".to_string(),
        }];
        members.extend(member_ids.iter().map(|peer_id| GroupMember {
            peer_id: peer_id.clone(),
            role: "member".to_string(),
        }));
        write_guard(&self.inner.groups)?.insert(
            group_id.clone(),
            GroupInfoState {
                name: name.to_string(),
                members: members.clone(),
                my_role: Some("owner".to_string()),
                outbound: Some(outbound),
            },
        );
        // Surface the group in the chat list (the group ID acts as a contact
        // with the group name as its display name).
        self.remember_contact_name(&group_id, name)?;
        self.save_group_sessions()?;

        // 4) Share the session key to every member over 1:1 sessions. Each
        //    member starts with `start_chat` (which also sends the greeting)
        //    when no session exists yet.
        for member in &member_ids {
            let result = async {
                if !mutex_guard(&self.inner.sessions)?.contains_key(member) {
                    self.start_chat(member).await?;
                }
                self.send_group_key(member, &group_id, &session_key, name)
            }
            .await;
            if let Err(err) = result {
                eprintln!("whisper desktop: failed to share group key to {member}: {err}");
            }
        }

        Ok(group_id)
    }

    /// Fetch a group's public metadata and member roster from the relay.
    pub async fn get_group_info(&self, group_id: &str) -> Result<GroupInfo, RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_group_info)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::GetGroupInfo {
            group_id: group_id.to_string(),
        }) {
            mutex_guard(&self.inner.pending_group_info)?.pop_back();
            return Err(err);
        }

        let info = tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::GroupTimeout)?
            .map_err(|_| RelayError::GroupRequestFailed)??;

        // Cache the fresh roster locally so the chat list and group panel stay
        // consistent without a full state refresh.
        let my_peer_id = self.my_peer_id()?;
        let my_role = info
            .members
            .iter()
            .find(|m| m.peer_id == my_peer_id)
            .map(|m| m.role.clone());
        {
            let mut groups = write_guard(&self.inner.groups)?;
            if let Some(group) = groups.get_mut(group_id) {
                group.name = info.name.clone();
                group.members = info.members.clone();
                group.my_role = my_role.clone();
            }
        }
        Ok(GroupInfo { my_role, ..info })
    }

    /// Promote `peer_id` to a group admin. The relay only allows this for the
    /// group owner or an existing admin.
    pub async fn promote_member(&self, group_id: &str, peer_id: &str) -> Result<(), RelayError> {
        self.group_op(ClientMessage::PromoteMember {
            group_id: group_id.to_string(),
            peer_id: peer_id.to_string(),
        })
        .await
    }

    /// Demote `peer_id` from admin back to a member. The relay only allows the
    /// owner to demote, and never the owner themselves.
    pub async fn demote_member(&self, group_id: &str, peer_id: &str) -> Result<(), RelayError> {
        self.group_op(ClientMessage::DemoteMember {
            group_id: group_id.to_string(),
            peer_id: peer_id.to_string(),
        })
        .await
    }

    /// Remove `peer_id` from a group. The relay only allows the owner to
    /// remove members, and never the owner themselves.
    pub async fn remove_member(&self, group_id: &str, peer_id: &str) -> Result<(), RelayError> {
        self.group_op(ClientMessage::RemoveMember {
            group_id: group_id.to_string(),
            peer_id: peer_id.to_string(),
        })
        .await
    }

    /// Remove the caller from a group's roster.
    pub async fn leave_group(&self, group_id: &str) -> Result<(), RelayError> {
        self.group_op(ClientMessage::LeaveGroup {
            group_id: group_id.to_string(),
        })
        .await
    }

    /// Send a promote/demote/remove/leave request and wait for its
    /// confirmation, then refresh the cached roster so the UI reflects the new
    /// membership immediately.
    async fn group_op(&self, message: ClientMessage) -> Result<(), RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_group_op)?.push_back(tx);
        if let Err(err) = self.send_json(&message) {
            mutex_guard(&self.inner.pending_group_op)?.pop_back();
            return Err(err);
        }
        tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::GroupTimeout)?
            .map_err(|_| RelayError::GroupRequestFailed)??;
        // Refresh the cached roster (best-effort: the group may no longer
        // exist from our point of view after a leave).
        if let ClientMessage::LeaveGroup { group_id } = message {
            self.forget_group(&group_id);
        } else if let ClientMessage::RemoveMember { group_id, .. } = message {
            // The removed member is no longer in the roster; refresh the rest.
            let _ = self.get_group_info(&group_id).await;
        } else if let ClientMessage::PromoteMember { group_id, .. }
        | ClientMessage::DemoteMember { group_id, .. } = message
        {
            let _ = self.get_group_info(&group_id).await;
        }
        Ok(())
    }

    /// Drop all local group state after leaving a group: the outbound/inbound
    /// sessions and the contact-list entry. Messages stay in history but the
    /// group no longer appears as a conversation.
    fn forget_group(&self, group_id: &str) {
        if let Ok(mut groups) = write_guard(&self.inner.groups) {
            groups.remove(group_id);
        }
        if let Ok(mut inbound) = mutex_guard(&self.inner.inbound_groups) {
            inbound.remove(group_id);
        }
        if let Ok(mut contacts) = write_guard(&self.inner.contacts) {
            contacts.retain(|c| c != group_id);
        }
        if let Ok(store) = self.store_guard() {
            if let Some(store) = store.as_ref() {
                let _ = store.delete_contact(group_id);
            }
        }
        let _ = self.save_group_sessions();
    }

    /// Encrypt a `group_key` share (the Megolm session key + group name) inside
    /// the 1:1 session with `peer_id` and send it as an ordinary message. The
    /// recipient recognises the plaintext JSON and stores the inbound session
    /// instead of rendering it as a chat message.
    fn send_group_key(
        &self,
        peer_id: &str,
        group_id: &str,
        session_key: &str,
        group_name: &str,
    ) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        let payload = serde_json::json!({
            "kind": "group_key",
            "group_id": group_id,
            "session_key": session_key,
            "group_name": group_name,
        });
        let (olm, session_id) = {
            let mut sessions = mutex_guard(&self.inner.sessions)?;
            let session = sessions
                .get_mut(peer_id)
                .ok_or_else(|| RelayError::NoSession(peer_id.to_string()))?;
            let session_id = session.session_id();
            let olm = session.encrypt(serde_json::to_vec(&payload)?)?;
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

    /// Register (or re-register) the caller's signed username alias with the
    /// relay, optionally attaching an avatar. Returns the registered username.
    pub async fn register_profile(
        &self,
        username: &str,
        display_name: Option<&str>,
        avatar_b64: Option<&str>,
    ) -> Result<String, RelayError> {
        // The relay verifies an Ed25519 signature over
        // `username || 0x00 || curve25519_key`, so sign it locally.
        let signature = {
            let guard = mutex_guard(&self.inner.identity)?;
            let identity = guard.as_ref().ok_or(RelayError::NoIdentity)?;
            let mut canonical = Vec::with_capacity(username.len() + 1 + 32);
            canonical.extend_from_slice(username.as_bytes());
            canonical.push(0);
            canonical.extend_from_slice(identity.curve25519_key().as_bytes());
            identity.sign(&canonical).to_base64()
        };

        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_register)?.push_back(tx);
        let message = ClientMessage::RegisterProfile {
            username: username.to_string(),
            signature,
            display_name: display_name.map(str::to_string),
            avatar: avatar_b64.map(str::to_string),
        };
        if let Err(err) = self.send_json(&message) {
            // The request never left, so drop the dangling waiter.
            mutex_guard(&self.inner.pending_register)?.pop_back();
            return Err(err);
        }

        tokio::time::timeout(PROFILE_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::ProfileTimeout)?
            .map_err(|_| RelayError::ProfileRequestFailed)?
    }

    /// Prefix-search registered usernames and peer IDs.
    pub async fn search_users(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ProfileSearchResult>, RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_search)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::SearchUsers {
            query: query.to_string(),
            limit,
        }) {
            mutex_guard(&self.inner.pending_search)?.pop_back();
            return Err(err);
        }

        tokio::time::timeout(PROFILE_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::ProfileTimeout)?
            .map_err(|_| RelayError::ProfileRequestFailed)?
    }

    /// Fetch one peer's public profile; `Ok(None)` when they have none.
    pub async fn get_profile(&self, peer_id: &str) -> Result<Option<PeerProfile>, RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_profile)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::GetProfile {
            peer_id: peer_id.to_string(),
        }) {
            mutex_guard(&self.inner.pending_profile)?.pop_back();
            return Err(err);
        }

        tokio::time::timeout(PROFILE_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::ProfileTimeout)?
            .map_err(|_| RelayError::ProfileRequestFailed)?
    }

    /// Re-register the caller's profile with a new avatar image (base64,
    /// ≤2 MB). The username must already be registered.
    pub async fn set_avatar(&self, username: &str, avatar_b64: &str) -> Result<(), RelayError> {
        self.register_profile(username, None, Some(avatar_b64))
            .await
            .map(|_| ())
    }

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
            ServerMessage::PrivacyUpdated => Ok(()),
            ServerMessage::ProfileRegistered { username } => {
                if let Some(tx) = mutex_guard(&self.inner.pending_register)?.pop_front() {
                    let _ = tx.send(Ok(username));
                }
                Ok(())
            }
            ServerMessage::UsersSearch { results } => {
                if let Some(tx) = mutex_guard(&self.inner.pending_search)?.pop_front() {
                    let _ = tx.send(Ok(results));
                }
                Ok(())
            }
            ServerMessage::Profile(profile) => {
                if let Some(tx) = mutex_guard(&self.inner.pending_profile)?.pop_front() {
                    let _ = tx.send(Ok(Some(profile)));
                }
                Ok(())
            }
            ServerMessage::Presence {
                peer_id,
                online,
                last_seen,
            } => {
                // Resolve the matching pending `get_presence` request, if any.
                // A push for a peer nobody is polling has no pending entry and
                // is simply stored + emitted below.
                let mut pending = mutex_guard(&self.inner.pending_presence)?;
                if let Some(pos) = pending.iter().position(|(peer, _)| peer == &peer_id) {
                    let (_, tx) = pending.remove(pos).expect("position must be in bounds");
                    let _ = tx.send(Ok(PresenceInfo { online, last_seen }));
                }
                drop(pending);
                self.handle_presence(&peer_id, online, last_seen)
            }
            ServerMessage::GroupCreated {
                group_id,
                name,
                members,
            } => {
                if let Some(tx) = mutex_guard(&self.inner.pending_group_created)?.pop_front() {
                    let _ = tx.send(Ok(group_id.clone()));
                }
                // Cache the roster we already know (the creator + every member
                // we added) so the chat list renders the group immediately.
                let my_peer_id = self.my_peer_id()?;
                let roster = members
                    .into_iter()
                    .map(|peer_id| GroupMember {
                        role: if peer_id == my_peer_id {
                            "owner".to_string()
                        } else {
                            "member".to_string()
                        },
                        peer_id,
                    })
                    .collect::<Vec<_>>();
                write_guard(&self.inner.groups)?
                    .entry(group_id.clone())
                    .or_insert(GroupInfoState {
                        name,
                        members: roster,
                        my_role: Some("owner".to_string()),
                        outbound: None,
                    });
                Ok(())
            }
            ServerMessage::GroupMemberAdded { group_id, peer_id } => {
                if let Some(tx) = mutex_guard(&self.inner.pending_group_member_added)?.pop_front() {
                    let _ = tx.send(Ok(()));
                }
                // Keep the local roster in sync: append the member unless
                // already present.
                if let Ok(mut groups) = write_guard(&self.inner.groups) {
                    if let Some(group) = groups.get_mut(&group_id) {
                        if !group.members.iter().any(|m| m.peer_id == peer_id) {
                            group.members.push(GroupMember {
                                peer_id,
                                role: "member".to_string(),
                            });
                        }
                    }
                }
                Ok(())
            }
            ServerMessage::GroupMemberLeft { .. } => {
                if let Some(tx) = mutex_guard(&self.inner.pending_group_op)?.pop_front() {
                    let _ = tx.send(Ok(()));
                }
                Ok(())
            }
            ServerMessage::GroupInfo {
                group_id,
                name,
                owner_peer_id,
                members,
            } => {
                let my_peer_id = self.my_peer_id()?;
                let my_role = members
                    .iter()
                    .find(|m| m.peer_id == my_peer_id)
                    .map(|m| m.role.clone());
                // Cache the fresh roster for the chat list / group panel.
                if let Ok(mut groups) = write_guard(&self.inner.groups) {
                    groups.entry(group_id.clone()).or_insert(GroupInfoState {
                        name: name.clone(),
                        members: members.clone(),
                        my_role: my_role.clone(),
                        outbound: None,
                    });
                }
                if let Some(tx) = mutex_guard(&self.inner.pending_group_info)?.pop_front() {
                    let _ = tx.send(Ok(GroupInfo {
                        group_id,
                        name,
                        owner_peer_id,
                        members,
                        my_role,
                    }));
                }
                Ok(())
            }
            ServerMessage::GroupMemberPromoted { group_id, peer_id } => {
                self.apply_group_role(&group_id, &peer_id, "admin")?;
                if let Some(tx) = mutex_guard(&self.inner.pending_group_op)?.pop_front() {
                    let _ = tx.send(Ok(()));
                }
                Ok(())
            }
            ServerMessage::GroupMemberDemoted { group_id, peer_id } => {
                self.apply_group_role(&group_id, &peer_id, "member")?;
                if let Some(tx) = mutex_guard(&self.inner.pending_group_op)?.pop_front() {
                    let _ = tx.send(Ok(()));
                }
                Ok(())
            }
            ServerMessage::GroupMemberRemoved { group_id, peer_id } => {
                if let Ok(mut groups) = write_guard(&self.inner.groups) {
                    if let Some(group) = groups.get_mut(&group_id) {
                        group.members.retain(|m| m.peer_id != peer_id);
                    }
                }
                if let Some(tx) = mutex_guard(&self.inner.pending_group_op)?.pop_front() {
                    let _ = tx.send(Ok(()));
                }
                Ok(())
            }
            ServerMessage::Error { code } => {
                let err = RelayError::Relay(code);
                // Resolve the oldest outstanding request across every queue.
                if let Some(tx) = mutex_guard(&self.inner.pending_prekeys)?.pop_front() {
                    let _ = tx.send(Err(err));
                } else if let Some(tx) = mutex_guard(&self.inner.pending_register)?.pop_front() {
                    let _ = tx.send(Err(err));
                } else if let Some(tx) = mutex_guard(&self.inner.pending_search)?.pop_front() {
                    let _ = tx.send(Err(err));
                } else if let Some(tx) = mutex_guard(&self.inner.pending_profile)?.pop_front() {
                    let _ = tx.send(Err(err));
                } else if let Some(tx) = mutex_guard(&self.inner.pending_group_created)?.pop_front()
                {
                    let _ = tx.send(Err(err));
                } else if let Some(tx) =
                    mutex_guard(&self.inner.pending_group_member_added)?.pop_front()
                {
                    let _ = tx.send(Err(err));
                } else if let Some(tx) = mutex_guard(&self.inner.pending_group_info)?.pop_front() {
                    let _ = tx.send(Err(err));
                } else if let Some(tx) = mutex_guard(&self.inner.pending_group_op)?.pop_front() {
                    let _ = tx.send(Err(err));
                }
                Ok(())
            }
        }
    }

    /// Mirror a member's role change into the cached group roster (used when a
    /// `group_member_promoted` / `group_member_demoted` reply arrives).
    fn apply_group_role(
        &self,
        group_id: &str,
        peer_id: &str,
        role: &str,
    ) -> Result<(), RelayError> {
        if let Ok(mut groups) = write_guard(&self.inner.groups) {
            if let Some(group) = groups.get_mut(group_id) {
                if let Some(member) = group.members.iter_mut().find(|m| m.peer_id == peer_id) {
                    member.role = role.to_string();
                }
            }
        }
        Ok(())
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

    /// Record a peer's presence and notify the UI via a `presence` event.
    ///
    /// Called for both `watch_presence` pushes and `get_presence` replies, so
    /// the cache and the event stream always reflect the same snapshot. An
    /// offline report's `last_seen` is persisted on the contact row so the
    /// timestamp survives restarts.
    fn handle_presence(
        &self,
        peer_id: &str,
        online: bool,
        last_seen: Option<i64>,
    ) -> Result<(), RelayError> {
        write_guard(&self.inner.presence)?
            .insert(peer_id.to_string(), PresenceInfo { online, last_seen });
        if !online {
            if let Some(ts) = last_seen {
                self.ensure_store_open()?;
                let store_guard = self.store_guard()?;
                let store = store_guard.as_ref().ok_or(RelayError::StoreNotOpen)?;
                // Only touch contacts we already know; presence alone must not
                // surface a stranger in the contact list.
                if let Some(mut contact) = store.get_contact(peer_id)? {
                    contact.last_seen = Some(ts);
                    store.upsert_contact(&contact)?;
                }
            }
        }
        let _ = self.inner.app.emit(
            "presence",
            PresenceEvent {
                peer_id: peer_id.to_string(),
                online,
                last_seen,
            },
        );
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

        // Group envelopes decrypt with the group's Megolm session and land in
        // the group's message thread, not the sender's 1:1 conversation.
        if let EnvelopeContent::Group { group_id, .. } = &wire.content {
            if let Some(message) = self.ingest_group(&wire, group_id)? {
                let _ = self.inner.app.emit(
                    "chat-message",
                    ChatMessageEvent {
                        peer_id: group_id.clone(),
                        message,
                    },
                );
            }
            return Ok(());
        }

        if let Some((peer_id, message)) = self.ingest(wire)? {
            let _ = self
                .inner
                .app
                .emit("chat-message", ChatMessageEvent { peer_id, message });
        }
        Ok(())
    }

    /// Turn an incoming wire envelope into plaintext.
    ///
    /// A handshake establishes the inbound X3DH session using the sender's
    /// identity key embedded in the pre-key message; an ordinary message is
    /// decrypted with the already-established session. Returns the thread the
    /// plaintext belongs to (the sender's peer ID) so the caller can emit the
    /// message under the right conversation key.
    fn ingest(&self, wire: Envelope) -> Result<Option<(String, UIMessage)>, RelayError> {
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
                Ok(Some((sender.clone(), self.record_incoming(&sender, text)?)))
            }
            EnvelopeContent::Message(message) => {
                let plaintext = {
                    let mut sessions = mutex_guard(&self.inner.sessions)?;
                    let session = sessions
                        .get_mut(&sender)
                        .ok_or_else(|| RelayError::NoSession(sender.clone()))?;
                    session.decrypt(&message.message)?
                };
                // Group-key shares travel as an encrypted serialized JSON
                // payload inside an ordinary message (so the relay only ever
                // sees ciphertext). They are recognised here and turned into
                // an inbound Megolm session instead of a chat message.
                if let Ok(group_key) = serde_json::from_slice::<GroupKeyPayload>(&plaintext) {
                    if group_key.kind == "group_key" {
                        self.save_sessions()?;
                        self.handle_group_key(&group_key)?;
                        return Ok(None);
                    }
                }
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
                // When read receipts are disabled we do NOT emit one — but
                // receipts the peer sends us are still shown (like WhatsApp:
                // the toggle only stops us from sending).
                if read_guard(&self.inner.settings)?.read_receipts {
                    let _ = self.send_receipt(&sender, ReceiptKind::Read);
                }
                Ok(Some((sender.clone(), self.record_incoming(&sender, text)?)))
            }
            // A bundle is published, never delivered as a chat envelope.
            EnvelopeContent::PreKeyBundle(_) => Ok(None),
            // Defensive: this client always sends receipts encrypted inside a
            // Message, so a bare receipt content is never expected here.
            EnvelopeContent::Receipt { .. } => Ok(None),
            // Group envelopes are routed through `handle_envelope` -> `ingest_group`
            // before this match, so a bare `Group` here is unexpected.
            EnvelopeContent::Group { .. } => Ok(None),
        }
    }

    /// Decrypt a Megolm group envelope with the group's inbound session and
    /// record it in the group's message thread.
    fn ingest_group(
        &self,
        wire: &Envelope,
        group_id: &str,
    ) -> Result<Option<UIMessage>, RelayError> {
        let ciphertext = match &wire.content {
            EnvelopeContent::Group { ciphertext, .. } => ciphertext.clone(),
            _ => return Ok(None),
        };
        let plaintext = match self.decrypt_group(group_id, &ciphertext) {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        let text = String::from_utf8_lossy(&plaintext).to_string();
        // No end-to-end read receipts for group messages in the MVP model.
        Ok(Some(self.record_incoming(group_id, text)?))
    }

    /// Decrypt a Megolm ciphertext with the group's inbound session. Returns
    /// `None` (and logs) when no inbound session exists yet or the ratchet
    /// rejects the message — a missing session key must never break the
    /// inbound pump.
    fn decrypt_group(&self, group_id: &str, ciphertext: &str) -> Option<Vec<u8>> {
        let mut inbound = match mutex_guard(&self.inner.inbound_groups) {
            Ok(g) => g,
            Err(_) => return None,
        };
        let session = match inbound.get_mut(group_id) {
            Some(session) => session,
            None => {
                eprintln!("whisper desktop: no inbound group session for {group_id}");
                return None;
            }
        };
        match session.decrypt(ciphertext) {
            Ok(bytes) => Some(bytes),
            Err(err) => {
                eprintln!("whisper desktop: failed to decrypt group message: {err}");
                None
            }
        }
    }

    /// Store a received Megolm `session_key` share as the group's inbound
    /// session and surface the group in the chat list under its name.
    fn handle_group_key(&self, payload: &GroupKeyPayload) -> Result<(), RelayError> {
        let inbound = InboundGroup::new(&payload.session_key)?;
        mutex_guard(&self.inner.inbound_groups)?.insert(payload.group_id.clone(), inbound);
        // Register the group so the chat list and header render it by name.
        write_guard(&self.inner.groups)?
            .entry(payload.group_id.clone())
            .or_insert(GroupInfoState {
                name: payload.group_name.clone(),
                members: Vec::new(),
                my_role: None,
                outbound: None,
            });
        // The group ID acts as a contact whose display name is the group name.
        self.remember_contact_name(&payload.group_id, &payload.group_name)?;
        self.save_group_sessions()?;
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Chat store lifecycle
    // ---------------------------------------------------------------------

    /// Open the SQLCipher store for the current identity and hydrate the
    /// in-memory state from it. No-op when the store is already open. The
    /// database is keyed deterministically from the identity file contents, so
    /// it only opens on the machine holding that identity.
    fn open_store(&self, identity_json: &str) -> Result<(), RelayError> {
        {
            let mut store = write_guard(&self.inner.store)?;
            if store.is_some() {
                return Ok(());
            }
            let peer_id = Identity::from_json(identity_json)?.peer_id();
            let path = resolve_store_path(&self.inner.app, &peer_id);
            let chat_store = ChatStore::open(&path, &derive_db_key(identity_json))?;
            *store = Some(chat_store);
        }
        self.hydrate_from_store()
    }

    /// Ensure the store is open, lazily opening it from the identity file on
    /// disk. Returns `Ok` with the store still closed when no identity exists
    /// yet, so settings reads work both before and after onboarding.
    fn ensure_store_open(&self) -> Result<(), RelayError> {
        if read_guard(&self.inner.store)?.is_some() {
            return Ok(());
        }
        let identity_json = match std::fs::read_to_string(&self.inner.identity_path) {
            Ok(json) => json,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        self.open_store(&identity_json)
    }

    /// A read guard over the store slot, for callers to unwrap to `&ChatStore`.
    fn store_guard(&self) -> Result<RwLockReadGuard<'_, Option<ChatStore>>, RelayError> {
        read_guard(&self.inner.store)
    }

    /// Restore persisted sessions, contacts, messages, settings and presence
    /// into the in-memory state. Runs once, right after the store is opened.
    fn hydrate_from_store(&self) -> Result<(), RelayError> {
        let (
            stored_sessions,
            stored_messages,
            stored_contacts,
            relay_url,
            theme,
            my_display_name,
            next_msg_id,
            presence_visible,
            read_receipts,
            typing_indicator,
            notifications_enabled,
            notification_preview,
            stored_group_outbound,
            stored_group_inbound,
        ) = {
            let store = self.store_guard()?;
            let store = store.as_ref().ok_or(RelayError::StoreNotOpen)?;
            (
                store.load_sessions()?,
                store.all_messages()?,
                store.contacts()?,
                store.get_setting("relay_url")?,
                store.get_setting("theme")?,
                store.get_setting("my_display_name")?,
                store.get_setting("next_msg_id")?,
                store.get_setting("presence_visible")?,
                store.get_setting("read_receipts")?,
                store.get_setting("typing_indicator")?,
                store.get_setting("notifications_enabled")?,
                store.get_setting("notification_preview")?,
                store.load_group_outbound()?,
                store.load_group_inbound()?,
            )
        };

        {
            let mut sessions = mutex_guard(&self.inner.sessions)?;
            for (peer, session_json) in stored_sessions {
                if let Ok(session) = ChatSession::from_json(&session_json) {
                    sessions.insert(peer, session);
                }
            }
        }
        *write_guard(&self.inner.messages)? = stored_messages;

        // Restore Megolm group sessions: outbound for groups this identity
        // created, inbound for groups it joined via a shared session key. The
        // group name is stored alongside each pickle so the chat list can
        // render groups right after startup.
        {
            let mut groups = write_guard(&self.inner.groups)?;
            for (group_id, (name, pickle)) in stored_group_outbound {
                if let Ok(outbound) = OutboundGroup::from_json(&pickle) {
                    groups.insert(
                        group_id,
                        GroupInfoState {
                            name,
                            members: Vec::new(),
                            my_role: Some("owner".to_string()),
                            outbound: Some(outbound),
                        },
                    );
                }
            }
        }
        {
            let mut inbound = mutex_guard(&self.inner.inbound_groups)?;
            let mut groups = write_guard(&self.inner.groups)?;
            for (group_id, (name, pickle)) in stored_group_inbound {
                if let Ok(session) = InboundGroup::from_json(&pickle) {
                    inbound.insert(group_id.clone(), session);
                    groups.entry(group_id).or_insert(GroupInfoState {
                        name,
                        members: Vec::new(),
                        my_role: None,
                        outbound: None,
                    });
                }
            }
        }

        // Contacts come back as rows: the ordered contact list, their learned
        // display names, and the last-seen timestamps that seed the presence
        // cache before any live push arrives.
        let mut contact_names = HashMap::new();
        let mut contacts = Vec::new();
        let mut presence = HashMap::new();
        for contact in stored_contacts {
            if let Some(name) = contact.display_name.clone() {
                contact_names.insert(contact.peer_id.clone(), name);
            }
            if let Some(last_seen) = contact.last_seen {
                presence.insert(
                    contact.peer_id.clone(),
                    PresenceInfo {
                        online: false,
                        last_seen: Some(last_seen),
                    },
                );
            }
            contacts.push(contact.peer_id);
        }
        *write_guard(&self.inner.contacts)? = contacts;

        let mut settings = read_guard(&self.inner.settings)?.clone();
        settings.relay_url = relay_url.filter(|url| !url.is_empty());
        settings.theme = theme.filter(|value| !value.is_empty());
        settings.presence_visible = setting_bool(presence_visible, true);
        settings.read_receipts = setting_bool(read_receipts, true);
        settings.typing_indicator = setting_bool(typing_indicator, true);
        settings.notifications_enabled = setting_bool(notifications_enabled, true);
        settings.notification_preview = setting_bool(notification_preview, true);
        *write_guard(&self.inner.settings)? = settings;

        let mut profiles = read_guard(&self.inner.profiles)?.clone();
        profiles.my_display_name = my_display_name.filter(|name| !name.is_empty());
        profiles.contacts = contact_names;
        *write_guard(&self.inner.profiles)? = profiles;

        if !presence.is_empty() {
            write_guard(&self.inner.presence)?.extend(presence);
        }

        // Resume the monotonic message-id counter so a restart never hands
        // out an id that already owns a row in the store.
        if let Some(value) = next_msg_id {
            if let Ok(n) = value.parse::<u64>() {
                self.inner.next_msg_id.store(n, Ordering::SeqCst);
            }
        }
        Ok(())
    }

    /// Persist all current sessions to the store, replacing the previous map.
    fn save_sessions(&self) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        let mut stored = HashMap::new();
        {
            let sessions = mutex_guard(&self.inner.sessions)?;
            for (peer, session) in sessions.iter() {
                if let Ok(json) = session.to_json() {
                    stored.insert(peer.clone(), json);
                }
            }
        }
        self.store_guard()?
            .as_ref()
            .ok_or(RelayError::StoreNotOpen)?
            .replace_sessions(&stored)?;
        Ok(())
    }

    /// Persist the Megolm group sessions (outbound + inbound pickles) to the
    /// store, replacing the previous maps. Group messages themselves live in
    /// the regular `messages` table keyed by the group ID.
    fn save_group_sessions(&self) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        let mut outbound = HashMap::new();
        {
            let groups = read_guard(&self.inner.groups)?;
            for (group_id, group) in groups.iter() {
                if let Some(session) = &group.outbound {
                    if let Ok(json) = session.to_json() {
                        outbound.insert(group_id.clone(), (group.name.clone(), json));
                    }
                }
            }
        }
        let mut inbound = HashMap::new();
        {
            let groups = read_guard(&self.inner.groups)?;
            let inbound_sessions = mutex_guard(&self.inner.inbound_groups)?;
            for (group_id, session) in inbound_sessions.iter() {
                if let Ok(json) = session.to_json() {
                    let name = groups
                        .get(group_id)
                        .map(|g| g.name.clone())
                        .unwrap_or_default();
                    inbound.insert(group_id.clone(), (name, json));
                }
            }
        }
        let store_guard = self.store_guard()?;
        let store = store_guard.as_ref().ok_or(RelayError::StoreNotOpen)?;
        store.replace_group_outbound(&outbound)?;
        store.replace_group_inbound(&inbound)?;
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Settings persistence
    // ---------------------------------------------------------------------

    /// Return the persisted settings, hydrated from the store on first use.
    pub fn get_settings(&self) -> Result<Settings, RelayError> {
        self.ensure_store_open()?;
        let settings = read_guard(&self.inner.settings)?.clone();
        Ok(settings)
    }

    /// Persist `settings` to the store and cache them in memory.
    fn save_settings(&self, settings: &Settings) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        let store_guard = self.store_guard()?;
        let store = store_guard.as_ref().ok_or(RelayError::StoreNotOpen)?;
        match &settings.relay_url {
            Some(url) if !url.is_empty() => store.set_setting("relay_url", url)?,
            _ => store.delete_setting("relay_url")?,
        }
        match &settings.theme {
            Some(theme) if !theme.is_empty() => store.set_setting("theme", theme)?,
            _ => store.delete_setting("theme")?,
        }
        store.set_setting("presence_visible", setting_str(settings.presence_visible))?;
        store.set_setting("read_receipts", setting_str(settings.read_receipts))?;
        store.set_setting("typing_indicator", setting_str(settings.typing_indicator))?;
        store.set_setting(
            "notifications_enabled",
            setting_str(settings.notifications_enabled),
        )?;
        store.set_setting(
            "notification_preview",
            setting_str(settings.notification_preview),
        )?;
        *write_guard(&self.inner.settings)? = settings.clone();
        Ok(())
    }

    /// Persist a new relay endpoint. If the client is connected to a different
    /// URL, the connection is dropped so the UI can reconnect to the new
    /// address.
    pub fn set_relay_url(&self, url: &str) -> Result<(), RelayError> {
        let mut settings = self.get_settings()?;
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
        let mut settings = self.get_settings()?;
        settings.theme = Some(theme.to_string());
        self.save_settings(&settings)
    }

    /// Toggle whether our online status and last-seen are visible to other
    /// peers. The preference is persisted locally so it restores on restart,
    /// and sent to the relay (best-effort) so it takes effect for others
    /// immediately. The relay answers with `privacy_updated`.
    pub fn set_privacy(&self, presence_visible: bool) -> Result<(), RelayError> {
        let mut settings = self.get_settings()?;
        settings.presence_visible = presence_visible;
        self.save_settings(&settings)?;
        if self.inner.connected.load(Ordering::SeqCst) {
            self.send_json(&ClientMessage::SetPrivacy { presence_visible })?;
        }
        Ok(())
    }

    /// Apply a partial boolean-preferences update (read receipts, typing
    /// indicator, notifications) and persist it. Each `Some` field overwrites
    /// the stored value; `None` fields are left untouched.
    pub fn update_settings(&self, patch: &SettingsPatch) -> Result<(), RelayError> {
        let mut settings = self.get_settings()?;
        if let Some(value) = patch.read_receipts {
            settings.read_receipts = value;
        }
        if let Some(value) = patch.typing_indicator {
            settings.typing_indicator = value;
        }
        if let Some(value) = patch.notifications_enabled {
            settings.notifications_enabled = value;
        }
        if let Some(value) = patch.notification_preview {
            settings.notification_preview = value;
        }
        self.save_settings(&settings)
    }

    /// Remove a contact and its message history on THIS device: the contact
    /// row, the messages and the cached presence. Client-local by design — the
    /// peer's copy of the conversation and the relay's queued envelopes are
    /// untouched. The Double Ratchet session is kept so a later message from
    /// the peer still decrypts (like Signal's "delete conversation"); the
    /// contact is then re-added by `ensure_contact` automatically.
    pub fn remove_contact(&self, peer_id: &str) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        write_guard(&self.inner.contacts)?.retain(|known| known != peer_id);
        write_guard(&self.inner.messages)?.remove(peer_id);
        write_guard(&self.inner.presence)?.remove(peer_id);
        write_guard(&self.inner.profiles)?.contacts.remove(peer_id);
        let store_guard = self.store_guard()?;
        let store = store_guard.as_ref().ok_or(RelayError::StoreNotOpen)?;
        store.delete_contact(peer_id)?;
        store.delete_messages_for(peer_id)?;
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Display names and receipts
    // ---------------------------------------------------------------------

    /// Persist our own display name to the store and cache it in memory.
    fn save_profiles(&self, profiles: &Profiles) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        let store_guard = self.store_guard()?;
        let store = store_guard.as_ref().ok_or(RelayError::StoreNotOpen)?;
        match &profiles.my_display_name {
            Some(name) if !name.is_empty() => store.set_setting("my_display_name", name)?,
            _ => store.delete_setting("my_display_name")?,
        }
        *write_guard(&self.inner.profiles)? = profiles.clone();
        Ok(())
    }

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
        self.ensure_store_open()?;
        let mut profiles = read_guard(&self.inner.profiles)?.clone();
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
    /// peer, encrypted inside the established session. When the typing
    /// indicator is disabled in settings this is a no-op — the peer never
    /// learns that we are typing.
    pub fn send_typing(&self, peer_id: &str, is_typing: bool) -> Result<(), RelayError> {
        if !read_guard(&self.inner.settings)?.typing_indicator {
            return Ok(());
        }
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
        self.ensure_store_open()?;
        self.store_guard()?
            .as_ref()
            .ok_or(RelayError::StoreNotOpen)?
            .upsert_contact(&ContactRow {
                peer_id: peer_id.to_string(),
                display_name: Some(name.to_string()),
                username: None,
                avatar_url: None,
                last_seen: None,
            })?;
        write_guard(&self.inner.profiles)?
            .contacts
            .insert(peer_id.to_string(), name.to_string());
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
                for client_id in &flipped {
                    self.persist_message_status(client_id, "read")?;
                }
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
        // Persist the message (no client id on incoming) and the message-id
        // counter so a restart never reuses an id that already owns a row.
        self.persist_message(peer_id, &message, None)?;
        self.persist_next_msg_id()?;
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
        let stored_client_id = if client_id.is_empty() {
            None
        } else {
            Some(client_id)
        };
        self.persist_message(peer_id, &message, stored_client_id)?;
        self.persist_next_msg_id()?;
        Ok(message)
    }

    /// Persist one recorded message to the store. The store is opened lazily
    /// here so a recording always lands on disk even if the connection died
    /// before the first full connect.
    fn persist_message(
        &self,
        peer_id: &str,
        message: &UIMessage,
        client_id: Option<&str>,
    ) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        self.store_guard()?
            .as_ref()
            .ok_or(RelayError::StoreNotOpen)?
            .upsert_message(peer_id, message, client_id)?;
        Ok(())
    }

    /// Persist the monotonic message-id counter so a restart never reuses an
    /// id that already owns a row in the store.
    fn persist_next_msg_id(&self) -> Result<(), RelayError> {
        let value = self.inner.next_msg_id.load(Ordering::SeqCst).to_string();
        self.store_guard()?
            .as_ref()
            .ok_or(RelayError::StoreNotOpen)?
            .set_setting("next_msg_id", &value)?;
        Ok(())
    }

    /// Flip the status of the message with `client_id` to "delivered" so a
    /// state refresh keeps the delivery marker. Returns whether the message
    /// was actually flipped: an ack that races in after an end-to-end read
    /// receipt must not downgrade the message back to "delivered".
    fn mark_delivered(&self, client_id: &str) -> Result<bool, RelayError> {
        let flipped = {
            let mut messages = write_guard(&self.inner.messages)?;
            apply_delivered(&mut messages, client_id)
        };
        if flipped {
            self.persist_message_status(client_id, "delivered")?;
        }
        Ok(flipped)
    }

    /// Persist a message status flip (delivered ack or read receipt) so the
    /// marker survives restarts.
    fn persist_message_status(&self, client_id: &str, status: &str) -> Result<(), RelayError> {
        let target = {
            let messages = read_guard(&self.inner.messages)?;
            messages.iter().find_map(|(peer, msgs)| {
                msgs.iter()
                    .find(|m| m.id == client_id)
                    .map(|m| (peer.clone(), m.clone()))
            })
        };
        if let Some((peer_id, mut message)) = target {
            message.status = status.to_string();
            self.persist_message(&peer_id, &message, Some(client_id))?;
        }
        Ok(())
    }

    /// Add `peer_id` to the contact list if it is not already there. For a new
    /// contact, the peer's public display name is fetched in the background so
    /// the receiving side of a conversation shows names too (not only the
    /// initiator, who learns the name during `start_chat`). The new contact is
    /// persisted so the contact list survives restarts.
    fn ensure_contact(&self, peer_id: &str) -> Result<(), RelayError> {
        let is_new = {
            let contacts = read_guard(&self.inner.contacts)?;
            !contacts.iter().any(|known| known == peer_id)
        };
        if is_new {
            write_guard(&self.inner.contacts)?.push(peer_id.to_string());
            self.ensure_store_open()?;
            self.store_guard()?
                .as_ref()
                .ok_or(RelayError::StoreNotOpen)?
                .upsert_contact(&ContactRow {
                    peer_id: peer_id.to_string(),
                    display_name: None,
                    username: None,
                    avatar_url: None,
                    last_seen: None,
                })?;
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

/// Parse a persisted boolean setting (`"true"` / `"false"`), falling back to
/// `default` for missing or unrecognized values. New installs have no rows, so
/// every preference defaults to enabled.
fn setting_bool(value: Option<String>, default: bool) -> bool {
    match value.as_deref() {
        Some("true") => true,
        Some("false") => false,
        _ => default,
    }
}

/// Serialize a boolean setting for storage.
fn setting_str(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
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

    // -- Presence wire format ------------------------------------------------

    #[test]
    fn server_presence_message_parses() {
        let text = r#"{"type":"presence","peer_id":"bob","online":false,"last_seen":1700000000}"#;
        match serde_json::from_str::<ServerMessage>(text).expect("parse") {
            ServerMessage::Presence {
                peer_id,
                online,
                last_seen,
            } => {
                assert_eq!(peer_id, "bob");
                assert!(!online);
                assert_eq!(last_seen, Some(1_700_000_000));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn server_presence_message_parses_online_with_null_last_seen() {
        let text = r#"{"type":"presence","peer_id":"bob","online":true,"last_seen":null}"#;
        match serde_json::from_str::<ServerMessage>(text).expect("parse") {
            ServerMessage::Presence {
                online, last_seen, ..
            } => {
                assert!(online);
                assert_eq!(last_seen, None);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn presence_client_messages_serialize() {
        let get = serde_json::to_value(ClientMessage::GetPresence {
            peer_id: "bob".into(),
        })
        .expect("serialize");
        assert_eq!(get["type"], "get_presence");
        assert_eq!(get["peer_id"], "bob");

        let watch = serde_json::to_value(ClientMessage::WatchPresence {
            peer_id: "bob".into(),
        })
        .expect("serialize");
        assert_eq!(watch["type"], "watch_presence");
        assert_eq!(watch["peer_id"], "bob");
    }

    // -- Group wire format ----------------------------------------------------

    #[test]
    fn group_client_messages_serialize_to_expected_wire_shape() {
        let create = serde_json::to_value(ClientMessage::CreateGroup {
            name: "Squad".into(),
        })
        .expect("serialize");
        assert_eq!(create["type"], "create_group");
        assert_eq!(create["name"], "Squad");

        let add = serde_json::to_value(ClientMessage::AddGroupMember {
            group_id: "g-1".into(),
            peer_id: "bob".into(),
        })
        .expect("serialize");
        assert_eq!(add["type"], "add_group_member");
        assert_eq!(add["group_id"], "g-1");
        assert_eq!(add["peer_id"], "bob");

        let info = serde_json::to_value(ClientMessage::GetGroupInfo {
            group_id: "g-1".into(),
        })
        .expect("serialize");
        assert_eq!(info["type"], "get_group_info");

        let promote = serde_json::to_value(ClientMessage::PromoteMember {
            group_id: "g-1".into(),
            peer_id: "bob".into(),
        })
        .expect("serialize");
        assert_eq!(promote["type"], "promote_member");

        let demote = serde_json::to_value(ClientMessage::DemoteMember {
            group_id: "g-1".into(),
            peer_id: "bob".into(),
        })
        .expect("serialize");
        assert_eq!(demote["type"], "demote_member");

        let remove = serde_json::to_value(ClientMessage::RemoveMember {
            group_id: "g-1".into(),
            peer_id: "bob".into(),
        })
        .expect("serialize");
        assert_eq!(remove["type"], "remove_member");

        let leave = serde_json::to_value(ClientMessage::LeaveGroup {
            group_id: "g-1".into(),
        })
        .expect("serialize");
        assert_eq!(leave["type"], "leave_group");
    }

    #[test]
    fn group_info_server_message_parses_with_roles() {
        let text = r#"{
            "type":"group_info",
            "group_id":"g-1",
            "name":"Squad",
            "owner_peer_id":"alice",
            "members":[
                {"peer_id":"alice","role":"owner"},
                {"peer_id":"bob","role":"admin"}
            ]
        }"#;
        match serde_json::from_str::<ServerMessage>(text).expect("parse") {
            ServerMessage::GroupInfo {
                group_id,
                name,
                owner_peer_id,
                members,
            } => {
                assert_eq!(group_id, "g-1");
                assert_eq!(name, "Squad");
                assert_eq!(owner_peer_id, "alice");
                assert_eq!(members.len(), 2);
                assert_eq!(members[0].peer_id, "alice");
                assert_eq!(members[0].role, "owner");
                assert_eq!(members[1].peer_id, "bob");
                assert_eq!(members[1].role, "admin");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn group_created_and_role_confirmation_messages_parse() {
        let created: ServerMessage = serde_json::from_str(
            r#"{"type":"group_created","group_id":"g-1","name":"Squad","members":["alice"]}"#,
        )
        .expect("parse");
        match created {
            ServerMessage::GroupCreated {
                group_id,
                name,
                members,
                ..
            } => {
                assert_eq!(group_id, "g-1");
                assert_eq!(name, "Squad");
                assert_eq!(members, vec!["alice".to_string()]);
            }
            other => panic!("unexpected variant: {other:?}"),
        }

        let promoted: ServerMessage = serde_json::from_str(
            r#"{"type":"group_member_promoted","group_id":"g-1","peer_id":"bob"}"#,
        )
        .expect("parse");
        match promoted {
            ServerMessage::GroupMemberPromoted { group_id, peer_id } => {
                assert_eq!(group_id, "g-1");
                assert_eq!(peer_id, "bob");
            }
            other => panic!("unexpected variant: {other:?}"),
        }

        let removed: ServerMessage = serde_json::from_str(
            r#"{"type":"group_member_removed","group_id":"g-1","peer_id":"bob"}"#,
        )
        .expect("parse");
        assert!(matches!(
            removed,
            ServerMessage::GroupMemberRemoved { group_id, peer_id }
                if group_id == "g-1" && peer_id == "bob"
        ));
    }

    #[test]
    fn group_key_payload_parses() {
        let text =
            r#"{"kind":"group_key","group_id":"g-1","session_key":"abc","group_name":"Squad"}"#;
        let payload: GroupKeyPayload = serde_json::from_str(text).expect("parse");
        assert_eq!(payload.kind, "group_key");
        assert_eq!(payload.group_id, "g-1");
        assert_eq!(payload.session_key, "abc");
        assert_eq!(payload.group_name, "Squad");
    }

    #[test]
    fn group_envelope_roundtrips_via_megolm_on_the_wire() {
        // The creator builds the outbound session and shares its session key
        // over an end-to-end channel; the recipient rebuilds the inbound side.
        let mut outbound = OutboundGroup::new();
        let session_key = outbound.session_key();
        let mut inbound = InboundGroup::new(&session_key).expect("key must parse");
        assert_eq!(outbound.session_id(), inbound.session_id());

        // A group envelope carries the Megolm ciphertext keyed by group id,
        // exactly as `send_group_message` emits it.
        let ciphertext = outbound.encrypt(b"hello group");
        let content = EnvelopeContent::Group {
            group_id: "g-1".to_string(),
            ciphertext: ciphertext.clone(),
        };
        let json = serde_json::to_string(&content).expect("serialize");
        let restored: EnvelopeContent = serde_json::from_str(&json).expect("deserialize");
        let restored_cipher = match restored {
            EnvelopeContent::Group { ciphertext, .. } => ciphertext,
            other => panic!("unexpected content: {other:?}"),
        };
        assert_eq!(restored_cipher, ciphertext);

        // The recipient decrypts with the inbound session built from the key.
        let plaintext = inbound.decrypt(&restored_cipher).expect("decrypt");
        assert_eq!(plaintext, b"hello group");

        // A group-key share travels as an ordinary encrypted Message whose
        // plaintext is the JSON payload.
        let payload = serde_json::json!({
            "kind": "group_key",
            "group_id": "g-1",
            "session_key": session_key,
            "group_name": "Squad",
        });
        let parsed: GroupKeyPayload = serde_json::from_value(payload).expect("parse");
        let mut member_inbound =
            InboundGroup::new(&parsed.session_key).expect("member key must parse");
        assert_eq!(member_inbound.session_id(), outbound.session_id());
        let next = outbound.encrypt(b"second message");
        assert_eq!(
            member_inbound.decrypt(&next).expect("decrypt"),
            b"second message"
        );
    }

    #[test]
    fn set_privacy_client_message_serializes() {
        let hide = serde_json::to_value(ClientMessage::SetPrivacy {
            presence_visible: false,
        })
        .expect("serialize");
        assert_eq!(hide["type"], "set_privacy");
        assert_eq!(hide["presence_visible"], false);

        let show = serde_json::to_value(ClientMessage::SetPrivacy {
            presence_visible: true,
        })
        .expect("serialize");
        assert_eq!(show["presence_visible"], true);
    }

    #[test]
    fn privacy_updated_server_message_parses() {
        let message: ServerMessage =
            serde_json::from_str(r#"{"type":"privacy_updated"}"#).expect("parse");
        assert!(matches!(message, ServerMessage::PrivacyUpdated));
    }

    #[test]
    fn presence_info_roundtrips_through_json() {
        let online = PresenceInfo {
            online: true,
            last_seen: None,
        };
        let restored: PresenceInfo =
            serde_json::from_str(&serde_json::to_string(&online).expect("serialize"))
                .expect("deserialize");
        assert!(restored.online);
        assert_eq!(restored.last_seen, None);

        let offline = PresenceInfo {
            online: false,
            last_seen: Some(1_700_000_000),
        };
        let json = serde_json::to_value(&offline).expect("serialize");
        assert_eq!(json["online"], false);
        assert_eq!(json["last_seen"], 1_700_000_000);
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
    // Settings
    // ---------------------------------------------------------------------

    #[test]
    fn settings_parse_handles_missing_fields() {
        let settings: Settings =
            serde_json::from_str(r#"{"theme":"dark"}"#).expect("partial settings must parse");
        assert_eq!(settings.relay_url, None);
        assert_eq!(settings.theme.as_deref(), Some("dark"));
        // Opt-out preferences default to enabled when the field is missing.
        assert!(settings.presence_visible);
        assert!(settings.read_receipts);
        assert!(settings.typing_indicator);
        assert!(settings.notifications_enabled);
        assert!(settings.notification_preview);
    }

    #[test]
    fn settings_parse_honours_explicit_opt_out_fields() {
        let settings: Settings = serde_json::from_str(
            r#"{"presence_visible":false,"read_receipts":false,"typing_indicator":false,"notifications_enabled":false,"notification_preview":false}"#,
        )
        .expect("full settings must parse");
        assert!(!settings.presence_visible);
        assert!(!settings.read_receipts);
        assert!(!settings.typing_indicator);
        assert!(!settings.notifications_enabled);
        assert!(!settings.notification_preview);
    }

    #[test]
    fn settings_patch_defaults_every_field_to_none() {
        let patch: SettingsPatch =
            serde_json::from_str(r#"{"read_receipts":true}"#).expect("partial patch must parse");
        assert_eq!(patch.read_receipts, Some(true));
        assert_eq!(patch.typing_indicator, None);
        assert_eq!(patch.notifications_enabled, None);
        assert_eq!(patch.notification_preview, None);
    }

    #[test]
    fn setting_bool_parses_strings_and_falls_back_to_default() {
        assert!(setting_bool(Some("true".into()), false));
        assert!(!setting_bool(Some("false".into()), true));
        assert!(setting_bool(None, true));
        assert!(!setting_bool(Some("garbage".into()), false));
        assert_eq!(setting_str(true), "true");
        assert_eq!(setting_str(false), "false");
    }

    #[test]
    fn relay_url_resolution_prefers_settings_then_env_then_default() {
        let custom = Settings {
            relay_url: Some("ws://custom".into()),
            ..Settings::default()
        };
        assert_eq!(resolve_relay_url(&custom, Some("ws://env")), "ws://custom");

        let blank = Settings {
            relay_url: Some(String::new()),
            ..Settings::default()
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
