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
//!
//! # Module layout
//!
//! This module is the relay core: the shared wire layer ([`ClientMessage`],
//! [`ServerMessage`], [`RelayEnvelope`]), the [`RelayClient`] connection
//! lifecycle, the inbound pump and its [`handle_text`](RelayClient::handle_text)
//! dispatch, the message send path, ack bookkeeping, dedup, and every public
//! type the UI consumes ([`ChatState`], [`Settings`], ...). Domain-specific
//! `impl RelayClient` blocks live in sibling modules declared below:
//!
//! - [`relay_groups`]: Megolm group sessions, roster operations and the group
//!   arms of the inbound dispatch.
//! - [`relay_profiles`]: username/profile registration, search, display names.
//! - [`relay_presence`]: presence queries, watches and the presence event stream.
//! - [`relay_settings`]: settings persistence, privacy toggles, receipts and typing.

#[path = "relay_contacts.rs"]
mod relay_contacts;
#[path = "relay_group_invites.rs"]
mod relay_group_invites;
#[path = "relay_groups.rs"]
mod relay_groups;
#[path = "relay_presence.rs"]
mod relay_presence;
#[path = "relay_profiles.rs"]
mod relay_profiles;
#[path = "relay_reactions.rs"]
mod relay_reactions;
#[path = "relay_settings.rs"]
mod relay_settings;
#[path = "relay_verify.rs"]
mod relay_verify;

use relay_groups::{GroupInfoState, GroupKeyPayload};

pub use relay_verify::SafetyNumberInfo;

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
    parse_plaintext, ChatPayload, ChatSession, Envelope, EnvelopeContent, Handshake, Identity,
    InboundGroup, Message, OutboundGroup, ParsedPayload, PreKeyBundle, Quote, ReactionPayload,
    ReceiptKind, TextPayload,
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

/// How long to wait before each auto-reconnect attempt, in seconds, starting
/// with the first retry: 2s → 5s → 10s → 20s, capped at 30s for every later
/// attempt.
const RECONNECT_BACKOFF_SECS: [u64; 5] = [2, 5, 10, 20, 30];

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
    /// The relay did not answer a friend-request command in time.
    #[error("timed out waiting for friend request reply")]
    ContactTimeout,
    /// The friend-request command was answered with an error or dropped.
    #[error("friend request failed")]
    ContactRequestFailed,
    /// A Megolm group-session operation failed.
    #[error("group error: {0}")]
    Group(#[from] e2ee_core::GroupError),
    /// The group has no outbound Megolm session. In the multi-sender model
    /// every member owns one (created automatically when they first receive
    /// the group key), so this only fires in the brief window before that
    /// join-time setup completes.
    #[error("group {0} has no outbound session yet")]
    NoOutboundGroup(String),
    /// The local identity is not (or no longer) in the group's roster.
    #[error("you are not a member of group {0}")]
    NotInGroup(String),
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
    /// The peer's public identity key is not known yet (needed for safety
    /// numbers). Learned from a pre-key bundle, handshake or profile.
    #[error("peer identity key unknown; start a chat with them first")]
    PeerKeyUnknown,
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
    /// Transfer group ownership to `peer_id` (owner only). The old owner
    /// becomes an admin; `peer_id` takes over the owner role.
    #[serde(rename = "transfer_ownership")]
    TransferOwnership {
        group_id: String,
        new_owner_peer_id: String,
    },
    /// Set a group's avatar image (base64, ≤2 MB). The relay stores the blob
    /// content-addressed and exposes it as `avatar_url` in group metadata.
    /// Only the owner or an admin may change the avatar.
    #[serde(rename = "set_group_avatar")]
    SetGroupAvatar { group_id: String, avatar: String },
    /// Send a friend request to `peer_id`. The relay stores the pending request
    /// and pushes `friend_request_received` to the target; the sender is
    /// answered with a `friend_requests` snapshot (or an `error` code such as
    /// `already_pending`, `already_contacts` or `cannot_add_self`).
    #[serde(rename = "send_friend_request")]
    SendFriendRequest { peer_id: String },
    /// Accept a pending incoming friend request from `peer_id`. Both sides
    /// become accepted contacts and the requester receives a
    /// `friend_request_accepted` push. The caller is answered with a
    /// `friend_requests` snapshot.
    #[serde(rename = "accept_friend_request")]
    AcceptFriendRequest { peer_id: String },
    /// Decline a pending incoming friend request from `peer_id`. The requester
    /// receives a `friend_request_declined` push. The caller is answered with a
    /// `friend_requests` snapshot.
    #[serde(rename = "decline_friend_request")]
    DeclineFriendRequest { peer_id: String },
    /// Fetch the full friend-request snapshot (incoming + outgoing). The relay
    /// replies with a `friend_requests` message.
    #[serde(rename = "get_friend_requests")]
    GetFriendRequests,
    /// Remove the accepted contact relationship with `peer_id` on both sides.
    /// The relay pushes `contact_removed` to both peers.
    #[serde(rename = "remove_contact")]
    RemoveContact { peer_id: String },
    #[serde(rename = "group_invite")]
    GroupInvite { group_id: String, peer_id: String },
    #[serde(rename = "group_invite_accept")]
    GroupInviteAccept { group_id: String },
    #[serde(rename = "group_invite_decline")]
    GroupInviteDecline { group_id: String },
    #[serde(rename = "get_group_invites")]
    GetGroupInvites,
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
    /// `members` carries each member's role (owner/admin/member). `avatar_url`
    /// is the public path of the group avatar blob (`/media/{hash}`), `null`
    /// when the group has none; it defaults to `None` so replies from older
    /// relays still parse.
    #[serde(rename = "group_info")]
    GroupInfo {
        group_id: String,
        name: String,
        owner_peer_id: String,
        #[serde(default)]
        avatar_url: Option<String>,
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
    /// Confirmation that group ownership was transferred to `new_owner_peer_id`
    /// (`transfer_ownership` reply). The old owner is now an admin.
    #[serde(rename = "ownership_transferred")]
    OwnershipTransferred {
        group_id: String,
        new_owner_peer_id: String,
    },
    /// Confirmation that a group's avatar was updated (`set_group_avatar`
    /// reply).
    #[serde(rename = "group_avatar_set")]
    GroupAvatarSet { group_id: String },
    /// A new incoming friend request from `peer_id`, carrying the requester's
    /// public display name (`null` when they have not set one). Pushed to the
    /// request target. `display_name` defaults to `None` so older pushes
    /// without the field still parse.
    #[serde(rename = "friend_request_received")]
    FriendRequestReceived {
        peer_id: String,
        #[serde(default)]
        display_name: Option<String>,
    },
    /// Confirmation that a friend request was sent (`send_friend_request`
    /// reply to the requester).
    #[serde(rename = "friend_request_sent")]
    FriendRequestSent,
    /// A pending OUTGOING request was accepted: `peer_id` is now a contact.
    /// Pushed to BOTH the requester and the acceptor, so each adds the peer
    /// to its contact list.
    #[serde(rename = "friend_request_accepted")]
    FriendRequestAccepted { peer_id: String },
    /// Confirmation that a pending request was accepted (`accept_friend_request`
    /// reply to the acceptor).
    #[serde(rename = "friend_request_accepted_ok")]
    FriendRequestAcceptedOk,
    /// A pending OUTGOING request was declined. Pushed to the requester.
    #[serde(rename = "friend_request_declined")]
    FriendRequestDeclined { peer_id: String },
    /// Confirmation that a pending request was declined (`decline_friend_request`
    /// reply to the decliner).
    #[serde(rename = "friend_request_declined_ok")]
    FriendRequestDeclinedOk,
    /// The accepted contact relationship with `peer_id` ended (either side
    /// removed it). Pushed to BOTH peers, so both drop the contact locally.
    #[serde(rename = "contact_removed")]
    ContactRemoved { peer_id: String },
    /// Confirmation that a contact was removed (`remove_contact` reply to the
    /// caller).
    #[serde(rename = "contact_removed_ok")]
    ContactRemovedOk,
    /// The full friend-request snapshot: reply to `get_friend_requests`.
    #[serde(rename = "friend_requests")]
    FriendRequests {
        incoming: Vec<FriendRequestIncoming>,
        #[serde(default)]
        outgoing: Vec<String>,
    },
    #[serde(rename = "group_invite_sent")]
    GroupInviteSent,
    #[serde(rename = "group_invite_received")]
    GroupInviteReceived {
        group_id: String,
        group_name: String,
        inviter_peer_id: String,
    },
    #[serde(rename = "group_invite_accepted_ok")]
    GroupInviteAcceptedOk,
    #[serde(rename = "group_invite_accepted")]
    GroupInviteAccepted { group_id: String, peer_id: String },
    #[serde(rename = "group_invite_declined_ok")]
    GroupInviteDeclinedOk,
    #[serde(rename = "group_invite_declined")]
    GroupInviteDeclined { group_id: String, peer_id: String },
    #[serde(rename = "group_invites")]
    GroupInvites { invites: Vec<GroupInviteInfo> },
    /// A protocol error code.
    Error { code: String },
}

/// One pending group invite as reported to the invitee.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupInviteInfo {
    pub group_id: String,
    pub group_name: String,
    pub inviter_peer_id: String,
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
/// the group info panel. `avatar_url` is the public path of the group avatar
/// blob (`/media/{hash}`), `None` when the group has none.
#[derive(Debug, Clone, Serialize)]
pub struct GroupInfo {
    pub group_id: String,
    pub name: String,
    pub owner_peer_id: String,
    pub avatar_url: Option<String>,
    pub members: Vec<GroupMember>,
    pub my_role: Option<String>,
}

/// Payload of the `group-removed` event emitted when the relay pushes a
/// `group_member_removed` for our own peer ID (the owner removed us from a
/// group). The UI drops the group and shows a toast.
#[derive(Debug, Clone, Serialize)]
pub struct GroupRemovedEvent {
    pub group_id: String,
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
    /// The quoted reply context, when this message replies to another one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<Quote>,
    /// Emoji reactions attached to this message, oldest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<ReactionView>,
}

/// One emoji reaction attached to a message by a peer.
#[derive(Debug, Clone, Serialize)]
pub struct ReactionView {
    /// Peer ID of the reacting peer.
    pub sender: String,
    /// The reaction emoji.
    pub emoji: String,
}

/// Payload of the `message-reaction` event emitted when a reaction is applied.
/// The UI uses it to toggle the pill under the affected bubble.
#[derive(Debug, Clone, Serialize)]
pub struct ReactionEvent {
    /// Conversation key: peer ID for 1:1 chats, group ID for groups.
    pub peer_id: String,
    /// The id of the reacted-to message.
    pub message_id: String,
    /// Peer ID of the reacting peer.
    pub sender: String,
    /// The reaction emoji.
    pub emoji: String,
    /// Whether the reaction was added (`true`) or removed (`false`).
    pub active: bool,
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

/// Payload of the `reconnecting` event emitted while the client retries a
/// dropped connection with exponential backoff. `active: false` ends the
/// reconnecting state — either the connection was re-established or a manual
/// disconnect/reset cancelled the retries.
#[derive(Debug, Clone, Serialize)]
pub struct ReconnectingEvent {
    /// Whether the auto-reconnect loop is still running.
    pub active: bool,
    /// One-based retry attempt currently scheduled (or in flight).
    pub attempt: u32,
    /// Milliseconds until the next `connect()` attempt.
    pub next_in_ms: u64,
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
/// `is_typing: false`. `sender` is the composing member for GROUP chats (the
/// `peer_id` is then the group id); it is `None` for 1:1 chats where the peer
/// id already identifies the writer.
#[derive(Debug, Clone, Serialize)]
pub struct TypingEvent {
    pub peer_id: String,
    pub is_typing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
}

/// Payload of the `contact-updated` event emitted when a contact's display
/// name or avatar is learned or refreshed (from a pre-key or profile lookup),
/// so the UI can update the contact list without waiting for a full state
/// refresh.
#[derive(Debug, Clone, Serialize)]
pub struct ContactUpdatedEvent {
    pub peer_id: String,
    pub display_name: Option<String>,
    /// Server avatar path ("/media/{hash}"); `None` when unknown/unchanged.
    #[serde(default)]
    pub avatar_url: Option<String>,
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
    /// Our own registered public username; `None` when unset. Persisted so the
    /// UI can show the registered state even when the relay is unreachable.
    #[serde(default)]
    pub my_username: Option<String>,
    /// Our own avatar path ("/media/{hash}"); `None` when unset. Persisted so
    /// the avatar renders across restarts.
    #[serde(default)]
    pub my_avatar_url: Option<String>,
    /// Peer ID -> the display name that peer advertises in pre-key lookups.
    #[serde(default)]
    pub contacts: HashMap<String, String>,
    /// Peer ID -> the avatar path that peer advertises ("/media/{hash}").
    /// Kept alongside `contacts` so `get_chat_state` can render contact
    /// avatars without a per-peer profile round-trip.
    #[serde(default)]
    pub contact_avatars: HashMap<String, String>,
}

/// A known conversation peer plus the display name and avatar they advertise,
/// if any.
#[derive(Debug, Clone, Serialize)]
pub struct ContactInfo {
    pub peer_id: String,
    /// `None` (or a peer with no name) falls back to the peer ID in the UI.
    pub display_name: Option<String>,
    /// Server avatar path ("/media/{hash}"); `None` when not known.
    #[serde(default)]
    pub avatar_url: Option<String>,
    /// Relationship status with this 1:1 peer: "accepted" (friends, chatable)
    /// or "pending" (a friend request is outstanding). Groups always report
    /// "accepted". `None` while unknown (e.g. a peer added by a live event
    /// before any snapshot).
    #[serde(default)]
    pub status: Option<String>,
}

/// One incoming friend request as reported by the relay: the requester's peer
/// ID plus the public display name they advertise (`None` when unset).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendRequestIncoming {
    pub peer_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

/// The full friend-request snapshot (the `get_friend_requests` reply). Incoming
/// requests carry the requester's display name; outgoing is a list of peer IDs
/// whose acceptance is still pending.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FriendRequests {
    #[serde(default)]
    pub incoming: Vec<FriendRequestIncoming>,
    #[serde(default)]
    pub outgoing: Vec<String>,
}

/// Payload of the `friend-request` event emitted when a new incoming friend
/// request arrives, so the UI can toast and refresh its Requests section.
#[derive(Debug, Clone, Serialize)]
pub struct FriendRequestEvent {
    pub peer_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

/// Payload of the `contact-added` event emitted when a peer becomes an
/// accepted contact: my outgoing request was accepted, or I accepted
/// someone's incoming request.
#[derive(Debug, Clone, Serialize)]
pub struct ContactAddedEvent {
    pub peer_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

/// Payload of the `friend-request-declined` event emitted when my outgoing
/// request was declined, so the UI can drop it from the Requests section.
#[derive(Debug, Clone, Serialize)]
pub struct FriendRequestDeclinedEvent {
    pub peer_id: String,
}

/// Payload of the `contact-removed` event emitted when a contact relationship
/// ends (either side removed it). The UI drops the peer locally and toasts.
#[derive(Debug, Clone, Serialize)]
pub struct ContactRemovedEvent {
    pub peer_id: String,
}

/// Snapshot of everything the UI needs to render the chat surface.
#[derive(Debug, Clone, Serialize)]
pub struct ChatState {
    pub my_peer_id: String,
    /// Our own public display name; `None` when unset.
    pub my_display_name: Option<String>,
    /// Our own registered public username; `None` when unset.
    pub my_username: Option<String>,
    /// Our own avatar path ("/media/{hash}"); `None` when unset.
    pub my_avatar_url: Option<String>,
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
    /// Incoming friend requests (requester + display name), in arrival order.
    /// Drives the Sidebar's Requests section.
    pub friend_requests_incoming: Vec<FriendRequestIncoming>,
    /// Outgoing (pending) friend requests: peer IDs we asked who have not
    /// answered yet.
    pub friend_requests_outgoing: Vec<String>,
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
    /// Whether the UI plays a short chime for incoming messages.
    #[serde(default = "default_true")]
    pub notification_sound: bool,
    /// UI language ("en", "fi", ...); the UI owns the valid values. Falls back
    /// to English when unset.
    #[serde(default)]
    pub language: Option<String>,
    /// Whether closing the window hides it to the system tray instead of
    /// quitting the app (WhatsApp-style background chat).
    #[serde(default)]
    pub minimize_to_tray: bool,
    /// Whether Enter sends a message in the composer. When off, Enter inserts a
    /// new line and Ctrl+Enter sends.
    #[serde(default = "default_true")]
    pub enter_to_send: bool,
    /// Message bubble font scale ("small", "normal", "large"); the UI owns the
    /// valid values. `None`/empty means the default "normal" scale.
    #[serde(default)]
    pub message_font_scale: Option<String>,
    /// Whether the app registers itself to launch at system startup (mirrors
    /// the OS-level autostart registration so the toggle restores on restart).
    #[serde(default)]
    pub autostart: bool,
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
            notification_sound: true,
            language: None,
            minimize_to_tray: false,
            enter_to_send: true,
            message_font_scale: None,
            autostart: false,
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
    #[serde(default)]
    pub notification_sound: Option<bool>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub minimize_to_tray: Option<bool>,
    #[serde(default)]
    pub enter_to_send: Option<bool>,
    #[serde(default)]
    pub message_font_scale: Option<String>,
    #[serde(default)]
    pub autostart: Option<bool>,
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
    /// Whether an unexpected connection drop should be followed by automatic
    /// reconnection (exponential backoff). Cleared by user-initiated
    /// `disconnect()`/`reset()` calls so those never loop back; re-armed by
    /// every successful `connect`.
    auto_reconnect: AtomicBool,
    /// Whether the auto-reconnect loop is currently running. Guards against
    /// stacking a second loop when a `connect` fails while one is already
    /// retrying.
    reconnect_loop_active: AtomicBool,
    /// Whether our persisted public profile has been re-asserted against the
    /// relay for this process run. The relay rate-limits profile mutations
    /// per source IP, so the startup sync must run once — not on every
    /// auto-reconnect.
    profile_synced: AtomicBool,
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
    /// (group_id, sender_peer_id). Each sender shares its own outbound
    /// session key over a 1:1 Double Ratchet envelope, so a recipient keeps
    /// one inbound session per sender and can decrypt every member's stream.
    inbound_groups: Mutex<HashMap<String, HashMap<String, InboundGroup>>>,
    /// In-flight `create_group` requests (replies are ordered, so FIFO works).
    pending_group_created: Mutex<VecDeque<oneshot::Sender<GroupCreatedResponse>>>,
    /// In-flight `add_group_member` requests, resolved in FIFO order.
    pending_group_member_added: Mutex<VecDeque<oneshot::Sender<GroupMemberAddedResponse>>>,
    /// In-flight `get_group_info` requests keyed by group ID. The relay may
    /// answer concurrent requests out of order, so a plain FIFO queue would
    /// misroute replies (one request timing out while another receives the
    /// group_info) — resolve by matching the group ID instead.
    pending_group_info: Mutex<VecDeque<(String, oneshot::Sender<GroupInfoResponse>)>>,
    /// In-flight promote/demote/remove/leave confirmations, resolved in FIFO
    /// order (the relay answers each request in turn).
    pending_group_op: Mutex<VecDeque<oneshot::Sender<GroupOpResponse>>>,
    /// In-flight group invite commands (invite/accept/decline), FIFO order.
    pending_group_invite_op: Mutex<VecDeque<oneshot::Sender<GroupOpResponse>>>,
    /// In-flight `get_group_invites` snapshots, FIFO order.
    pending_group_invites_list: Mutex<VecDeque<oneshot::Sender<GroupInvitesResponse>>>,
    /// Pending group invites for this identity, in arrival order. Fed by
    /// `group_invite_received` pushes and `group_invites` snapshots.
    group_invites_incoming: RwLock<Vec<GroupInviteInfo>>,
    /// Incoming friend requests, in arrival order, with the requester's public
    /// display name when known. Fed by `friend_request_received` pushes and
    /// `friend_requests` snapshots.
    friend_requests_incoming: RwLock<Vec<FriendRequestIncoming>>,
    /// Outgoing pending friend requests: peer IDs we asked who have not
    /// answered yet.
    friend_requests_outgoing: RwLock<Vec<String>>,
    /// In-flight friend-request commands (send/accept/decline/get), resolved
    /// in FIFO order. The relay answers each one with a `friend_requests`
    /// snapshot (or an `error` code), so FIFO alignment holds.
    pending_contact_ops: Mutex<VecDeque<oneshot::Sender<ContactOpResponse>>>,
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

/// Result channel type for a `get_group_invites` snapshot.
type GroupInvitesResponse = Result<Vec<GroupInviteInfo>, RelayError>;

/// Result channel type for a friend-request command: the `friend_requests`
/// snapshot the relay answers with.
type ContactOpResponse = Result<FriendRequests, RelayError>;

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
                auto_reconnect: AtomicBool::new(true),
                reconnect_loop_active: AtomicBool::new(false),
                profile_synced: AtomicBool::new(false),
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
                pending_group_invite_op: Mutex::new(VecDeque::new()),
                pending_group_invites_list: Mutex::new(VecDeque::new()),
                group_invites_incoming: RwLock::new(Vec::new()),
                friend_requests_incoming: RwLock::new(Vec::new()),
                friend_requests_outgoing: RwLock::new(Vec::new()),
                pending_contact_ops: Mutex::new(VecDeque::new()),
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
        // Persist the *effective* endpoint (settings -> env var -> default) so
        // the UI can resolve `/media/{hash}` avatar URLs against the relay the
        // client actually connected to, even when the user never set one.
        {
            let mut settings = read_guard(&self.inner.settings)?.clone();
            if settings.relay_url.as_deref() != Some(url.as_str()) {
                settings.relay_url = Some(url.clone());
                self.save_settings(&settings)?;
            }
        }
        let (ws_stream, _) = match tokio_tungstenite::connect_async(&url).await {
            Ok(stream) => stream,
            Err(err) => {
                // A failed connection attempt (the relay is unreachable) also
                // enters the auto-reconnect loop, so the app keeps retrying by
                // itself instead of waiting for a manual "Reconnect" click. The
                // loop-active guard keeps this from stacking a second loop when
                // a retry inside the loop fails too.
                if self.inner.auto_reconnect.load(Ordering::SeqCst) {
                    self.spawn_reconnect_loop();
                }
                return Err(RelayError::Connection(err.to_string()));
            }
        };

        let (mut write, read) = ws_stream.split();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();

        {
            let mut outbox = write_guard(&self.inner.outbox)?;
            *outbox = Some(out_tx.clone());
        }
        self.inner.connected.store(true, Ordering::SeqCst);
        // A successful connect (initial, manual or automatic) re-arms
        // auto-reconnect so a later drop is retried again. A user-initiated
        // `disconnect()`/`reset()` clears the flag first, so they always win.
        self.inner.auto_reconnect.store(true, Ordering::SeqCst);
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
            .send(WsMessage::Text(hello_json.into()))
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
                            tracing::warn!(error = %err, "inbound relay message error");
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

        // After a startup hydration or a reconnect, groups restored from the
        // store have an empty member roster. Kick off background `get_group_info`
        // fetches so the chat list shows a real member count (not "0") shortly
        // after connecting, without the user opening the info panel.
        self.refresh_group_rosters();

        // Contacts learned before the avatar pipeline existed (or restored from
        // an older store) may lack an avatar_url. Kick off background
        // `get_profile` fetches for those so the chat list and headers render
        // images instead of letter tiles — without waiting for the user to open
        // a profile. `get_profile` persists whatever it learns and emits
        // `contact-updated`, so the UI updates live.
        {
            let contacts = read_guard(&self.inner.contacts)
                .map(|contacts| contacts.clone())
                .unwrap_or_default();
            let avatars = read_guard(&self.inner.profiles)
                .map(|profiles| profiles.contact_avatars.clone())
                .unwrap_or_default();
            let missing: Vec<String> = contacts
                .into_iter()
                .filter(|peer| !avatars.contains_key(peer))
                .collect();
            if !missing.is_empty() {
                let client = self.clone();
                tauri::async_runtime::spawn(async move {
                    for peer in missing {
                        if let Err(err) = client.get_profile(&peer).await {
                            tracing::trace!(peer = %peer, error = %err, "avatar sync lookup failed");
                        }
                    }
                });
            }
        }

        // Re-assert our persisted public profile against the relay's users
        // table so the server keeps (peer_id, username, display_name,
        // avatar_hash) across app restarts. Runs once per process — the relay
        // rate-limits profile mutations per source IP — and in the background
        // so a slow relay can never delay the connect handoff.
        if self
            .inner
            .profile_synced
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let client = self.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = client.sync_own_profile().await {
                    tracing::warn!(error = %err, "failed to re-assert own profile on the relay");
                }
            });
        }

        Ok(())
    }

    /// Tear down the current connection. Other commands will report
    /// [`RelayError::NotConnected`] until [`RelayClient::connect`] is called.
    ///
    /// A manual disconnect cancels any pending auto-reconnect loop first, so
    /// the connection stays down until the UI reconnects on its own.
    pub fn disconnect(&self) -> Result<(), RelayError> {
        self.inner.auto_reconnect.store(false, Ordering::SeqCst);
        self.mark_disconnected();
        Ok(())
    }

    /// Disconnect and wipe all in-memory chat state. Called when the identity
    /// is reset so stale contacts, sessions and messages never leak into a
    /// freshly generated identity.
    pub fn reset(&self) -> Result<(), RelayError> {
        // Cancel any pending auto-reconnect loop before the state wipe, so the
        // reset cannot reconnect (and resurrect state) behind the UI's back.
        self.inner.auto_reconnect.store(false, Ordering::SeqCst);
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
        // A reset may be followed by a fresh identity: allow its profile to
        // be re-asserted on the next connect.
        self.inner.profile_synced.store(false, Ordering::SeqCst);
        write_guard(&self.inner.messages)?.clear();
        write_guard(&self.inner.contacts)?.clear();
        write_guard(&self.inner.presence)?.clear();
        write_guard(&self.inner.friend_requests_incoming)?.clear();
        write_guard(&self.inner.friend_requests_outgoing)?.clear();
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

    /// Establish an outbound X3DH session with `peer_id` and send the first,
    /// session-creating message.
    pub async fn start_chat(&self, peer_id: &str) -> Result<(), RelayError> {
        if peer_id == self.my_peer_id()? {
            return Err(RelayError::InvalidPeer(peer_id.to_string()));
        }

        let (bundle, display_name) = self.fetch_prekeys(peer_id).await?;
        let my_peer_id = self.my_peer_id()?;

        // Learn the peer's public display name so the UI can show it in the
        // contact list and chat header. The identity key from the bundle is
        // remembered too, so safety numbers work offline.
        if let Some(name) = display_name {
            self.remember_contact_name(peer_id, &name)?;
        }
        self.remember_peer_key(peer_id, bundle.identity_key)?;

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
        let msg = self.record_outgoing(peer_id, FIRST_MESSAGE_TEXT, "", None)?;
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
    /// `send_group_message`. A non-empty `quote` turns the message into a
    /// quoted reply: the text and quote snapshot travel as an encrypted
    /// [`ChatPayload::Text`] envelope so older clients keep working (their
    /// plain messages stay raw text).
    pub async fn send_message(
        &self,
        peer_id: &str,
        text: &str,
        client_id: &str,
        quote: Option<Quote>,
    ) -> Result<(), RelayError> {
        // Group messages are encrypted with the group's Megolm session and
        // routed through the relay's group fan-out rather than a 1:1 ratchet.
        // A group that predates the multi-sender model (or whose join-time
        // setup never finished) has no outbound session yet; establish one
        // (fetch the roster, build our own Megolm session, share its key to
        // the other members) before the first send so old groups become
        // sendable without any user action.
        if read_guard(&self.inner.groups)?.contains_key(peer_id) {
            self.ensure_outbound_session(peer_id).await?;
            // The message id is decided up front and embedded in the encrypted
            // payload; every recipient stores the message under this same id,
            // which is what lets reactions/replies resolve across devices.
            let message_id = self.next_message_id(client_id);
            let payload = ChatPayload::Text(TextPayload {
                text: text.to_string(),
                quote,
                message_id: Some(message_id.clone()),
            });
            let bytes = serde_json::to_vec(&payload)?;
            return self.send_group_payload(
                peer_id,
                &bytes,
                relay_groups::GroupSend {
                    record: true,
                    client_id: client_id.to_string(),
                    quote: None,
                    message_id: Some(message_id),
                    display_text: Some(text.to_string()),
                },
            );
        }

        let my_peer_id = self.my_peer_id()?;
        // Same id-sharing scheme as the group path above.
        let message_id = self.next_message_id(client_id);
        let (olm, session_id) = {
            let mut sessions = mutex_guard(&self.inner.sessions)?;
            let session = sessions
                .get_mut(peer_id)
                .ok_or_else(|| RelayError::NoSession(peer_id.to_string()))?;
            let session_id = session.session_id();
            let payload = ChatPayload::Text(TextPayload {
                text: text.to_string(),
                quote,
                message_id: Some(message_id.clone()),
            });
            let olm = session.encrypt(&serde_json::to_vec(&payload)?)?;
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
        let msg = self.record_outgoing_with_id(peer_id, message_id, text, None)?;
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

    /// Snapshot the state the UI needs: identity, connection, contacts (with
    /// their display names and relationship status), message history, group
    /// metadata and the pending friend-request lists.
    pub fn get_chat_state(&self) -> Result<ChatState, RelayError> {
        let my_peer_id = self.my_peer_id()?;
        self.ensure_store_open()?;
        let profiles = read_guard(&self.inner.profiles)?.clone();
        let contacts = read_guard(&self.inner.contacts)?.clone();
        let messages = read_guard(&self.inner.messages)?.clone();
        let presence = read_guard(&self.inner.presence)?.clone();
        let connected = self.inner.connected.load(Ordering::SeqCst);
        let friend_requests_incoming = read_guard(&self.inner.friend_requests_incoming)?.clone();
        let friend_requests_outgoing = read_guard(&self.inner.friend_requests_outgoing)?.clone();
        // A peer with an outstanding request (either direction) is not
        // chatable yet, so its status reads "pending" instead of "accepted".
        let pending: HashSet<String> = friend_requests_incoming
            .iter()
            .map(|request| request.peer_id.clone())
            .chain(friend_requests_outgoing.iter().cloned())
            .collect();
        let contacts = contacts
            .into_iter()
            .map(|peer_id| ContactInfo {
                peer_id: peer_id.clone(),
                display_name: profiles.contacts.get(&peer_id).cloned(),
                avatar_url: profiles.contact_avatars.get(&peer_id).cloned(),
                status: Some(if pending.contains(&peer_id) {
                    "pending".to_string()
                } else {
                    "accepted".to_string()
                }),
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
                avatar_url: group.avatar_url.clone(),
                members: group.members.clone(),
                my_role: group.my_role.clone(),
            })
            .collect();
        Ok(ChatState {
            my_peer_id,
            my_display_name: profiles.my_display_name,
            my_username: profiles.my_username,
            my_avatar_url: profiles.my_avatar_url,
            connected,
            contacts,
            messages,
            presence,
            groups,
            friend_requests_incoming,
            friend_requests_outgoing,
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
            } => self.handle_group_created(group_id, name, members),
            ServerMessage::GroupMemberAdded { group_id, peer_id } => {
                self.handle_group_member_added(&group_id, &peer_id)
            }
            ServerMessage::GroupMemberLeft { group_id, peer_id } => {
                self.handle_group_member_left(&group_id, &peer_id)
            }
            ServerMessage::GroupInfo {
                group_id,
                name,
                owner_peer_id,
                avatar_url,
                members,
            } => self.handle_group_info(group_id, name, owner_peer_id, avatar_url, members),
            ServerMessage::GroupMemberPromoted { group_id, peer_id } => {
                self.handle_group_member_promoted(&group_id, &peer_id)
            }
            ServerMessage::GroupMemberDemoted { group_id, peer_id } => {
                self.handle_group_member_demoted(&group_id, &peer_id)
            }
            ServerMessage::GroupMemberRemoved { group_id, peer_id } => {
                self.handle_group_member_removed(&group_id, &peer_id)
            }
            ServerMessage::OwnershipTransferred {
                group_id,
                new_owner_peer_id,
            } => self.handle_ownership_transferred(&group_id, &new_owner_peer_id),
            ServerMessage::GroupAvatarSet { group_id } => self.handle_group_avatar_set(&group_id),
            ServerMessage::FriendRequestReceived {
                peer_id,
                display_name,
            } => self.handle_friend_request_received(&peer_id, display_name),
            ServerMessage::FriendRequestSent => self.handle_friend_request_ack(),
            ServerMessage::FriendRequestAccepted { peer_id } => {
                self.handle_friend_request_accepted(&peer_id)
            }
            ServerMessage::FriendRequestAcceptedOk => self.handle_friend_request_ack(),
            ServerMessage::FriendRequestDeclined { peer_id } => {
                self.handle_friend_request_declined(&peer_id)
            }
            ServerMessage::FriendRequestDeclinedOk => self.handle_friend_request_ack(),
            ServerMessage::ContactRemoved { peer_id } => self.handle_contact_removed(&peer_id),
            ServerMessage::ContactRemovedOk => self.handle_friend_request_ack(),
            ServerMessage::FriendRequests { incoming, outgoing } => {
                self.handle_friend_requests(incoming, outgoing)
            }
            ServerMessage::GroupInviteSent => self.handle_group_op_ack(),
            ServerMessage::GroupInviteReceived {
                group_id,
                group_name,
                inviter_peer_id,
            } => self.handle_group_invite_received(&group_id, &group_name, &inviter_peer_id),
            ServerMessage::GroupInviteAcceptedOk => self.handle_group_op_ack(),
            ServerMessage::GroupInviteAccepted { group_id, peer_id } => {
                self.handle_group_invite_accepted(&group_id, &peer_id)
            }
            ServerMessage::GroupInviteDeclinedOk => self.handle_group_op_ack(),
            ServerMessage::GroupInviteDeclined { group_id, peer_id } => {
                self.handle_group_invite_declined(&group_id, &peer_id)
            }
            ServerMessage::GroupInvites { invites } => self.handle_group_invites(invites),
            ServerMessage::Error { code } => {
                // Route the error to the queue that actually owns the failed
                // request. A blind "oldest waiter across every queue" fallback
                // lets an unrelated stale waiter (e.g. a leftover pre-key
                // fetch) swallow errors meant for group/profile requests,
                // which then time out — and, for example, a stale group can
                // never learn that it is `not_a_member` anymore.
                // Generic helper: resolve any pending request with a relay
                // error. Each queue carries `Sender<Result<T, RelayError>>`
                // with a different T, hence the generic function instead of a
                // closure.
                fn send_error<T>(tx: oneshot::Sender<Result<T, RelayError>>, code: &str) {
                    let _ = tx.send(Err(RelayError::Relay(code.to_string())));
                }
                match code.as_str() {
                    // Group operations: membership/ownership errors.
                    "not_a_member" | "group_not_found" | "not_admin" | "not_owner"
                    | "invalid_group_name" => {
                        if let Some((_, tx)) =
                            mutex_guard(&self.inner.pending_group_info)?.pop_front()
                        {
                            send_error(tx, &code);
                        } else if let Some(tx) =
                            mutex_guard(&self.inner.pending_group_op)?.pop_front()
                        {
                            send_error(tx, &code);
                        } else if let Some(tx) =
                            mutex_guard(&self.inner.pending_group_created)?.pop_front()
                        {
                            send_error(tx, &code);
                        } else if let Some(tx) =
                            mutex_guard(&self.inner.pending_group_member_added)?.pop_front()
                        {
                            send_error(tx, &code);
                        }
                    }
                    // Contact/friend + pre-key operations.
                    "not_contacts" | "already_pending" | "already_contacts" | "cannot_add_self"
                    | "not_found" => {
                        if let Some(tx) = mutex_guard(&self.inner.pending_prekeys)?.pop_front() {
                            send_error(tx, &code);
                        } else if let Some(tx) =
                            mutex_guard(&self.inner.pending_contact_ops)?.pop_front()
                        {
                            send_error(tx, &code);
                        } else if let Some(tx) =
                            mutex_guard(&self.inner.pending_group_invite_op)?.pop_front()
                        {
                            send_error(tx, &code);
                        }
                    }
                    // Group invite errors.
                    "not_invited" | "already_invited" | "already_member" => {
                        if let Some(tx) =
                            mutex_guard(&self.inner.pending_group_invite_op)?.pop_front()
                        {
                            send_error(tx, &code);
                        }
                    }
                    // Profile/username/avatar operations.
                    "invalid_username"
                    | "username_taken"
                    | "bad_signature"
                    | "invalid_display_name"
                    | "invalid_avatar"
                    | "no_profile"
                    | "media_error" => {
                        if let Some(tx) = mutex_guard(&self.inner.pending_register)?.pop_front() {
                            send_error(tx, &code);
                        } else if let Some(tx) =
                            mutex_guard(&self.inner.pending_search)?.pop_front()
                        {
                            send_error(tx, &code);
                        } else if let Some(tx) =
                            mutex_guard(&self.inner.pending_profile)?.pop_front()
                        {
                            send_error(tx, &code);
                        }
                    }
                    // Unknown code: fall back to the oldest waiter anywhere.
                    _ => {
                        if let Some(tx) = mutex_guard(&self.inner.pending_prekeys)?.pop_front() {
                            send_error(tx, &code);
                        } else if let Some(tx) =
                            mutex_guard(&self.inner.pending_register)?.pop_front()
                        {
                            send_error(tx, &code);
                        } else if let Some(tx) =
                            mutex_guard(&self.inner.pending_search)?.pop_front()
                        {
                            send_error(tx, &code);
                        } else if let Some(tx) =
                            mutex_guard(&self.inner.pending_profile)?.pop_front()
                        {
                            send_error(tx, &code);
                        } else if let Some((_, tx)) =
                            mutex_guard(&self.inner.pending_group_info)?.pop_front()
                        {
                            send_error(tx, &code);
                        } else if let Some(tx) =
                            mutex_guard(&self.inner.pending_group_op)?.pop_front()
                        {
                            send_error(tx, &code);
                        } else if let Some(tx) =
                            mutex_guard(&self.inner.pending_contact_ops)?.pop_front()
                        {
                            send_error(tx, &code);
                        }
                    }
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
                self.save_sessions()?;
                match parse_plaintext(&inbound.plaintext) {
                    ParsedPayload::Text(text) => Ok(Some((
                        sender.clone(),
                        self.record_incoming(&sender, text.text, text.quote, text.message_id)?,
                    ))),
                    ParsedPayload::Reaction(reaction) => {
                        self.handle_reaction(
                            &sender,
                            &reaction.message_id,
                            &sender,
                            &reaction.emoji,
                            reaction.active,
                        )?;
                        Ok(None)
                    }
                    ParsedPayload::Typing(_) => Ok(None),
                }
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
                // an inbound Megolm session instead of a chat message. The key
                // is attributed to the sender of the 1:1 envelope that carried
                // it (backfilling the payload's `sender` field when an older
                // share omitted it).
                if let Ok(mut group_key) = serde_json::from_slice::<GroupKeyPayload>(&plaintext) {
                    if group_key.kind == "group_key" {
                        if group_key.sender.is_empty() {
                            group_key.sender = sender.clone();
                        }
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
                // Anything else is an ordinary chat payload: a text message
                // (possibly quoting an earlier one) or an emoji reaction. Both
                // travel as encrypted JSON so the relay only ever sees
                // ciphertext; legacy raw text parses as plain text too.
                self.save_sessions()?;
                match parse_plaintext(&plaintext) {
                    ParsedPayload::Text(text) => {
                        // Acknowledging the message end-to-end: encrypt a read
                        // receipt with the same (now-advanced) session.
                        // Best-effort so a transient send failure never drops
                        // the plaintext message. When read receipts are
                        // disabled we do NOT emit one — but receipts the peer
                        // sends us are still shown (like WhatsApp: the toggle
                        // only stops us from sending).
                        if read_guard(&self.inner.settings)?.read_receipts {
                            let _ = self.send_receipt(&sender, ReceiptKind::Read);
                        }
                        Ok(Some((
                            sender.clone(),
                            self.record_incoming(&sender, text.text, text.quote, text.message_id)?,
                        )))
                    }
                    ParsedPayload::Reaction(reaction) => {
                        self.handle_reaction(
                            &sender,
                            &reaction.message_id,
                            &sender,
                            &reaction.emoji,
                            reaction.active,
                        )?;
                        Ok(None)
                    }
                    // A group-style typing payload in a 1:1 session is
                    // unexpected (1:1 typing uses the receipt channel); ignore
                    // it defensively.
                    ParsedPayload::Typing(_) => Ok(None),
                }
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
            my_username,
            my_avatar_url,
            next_msg_id,
            presence_visible,
            read_receipts,
            typing_indicator,
            notifications_enabled,
            notification_preview,
            notification_sound,
            language,
            minimize_to_tray,
            enter_to_send,
            message_font_scale,
            autostart,
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
                store.get_setting("my_username")?,
                store.get_setting("my_avatar_url")?,
                store.get_setting("next_msg_id")?,
                store.get_setting("presence_visible")?,
                store.get_setting("read_receipts")?,
                store.get_setting("typing_indicator")?,
                store.get_setting("notifications_enabled")?,
                store.get_setting("notification_preview")?,
                store.get_setting("notification_sound")?,
                store.get_setting("language")?,
                store.get_setting("minimize_to_tray")?,
                store.get_setting("enter_to_send")?,
                store.get_setting("message_font_scale")?,
                store.get_setting("autostart")?,
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
                            avatar_url: None,
                            outbound: Some(outbound),
                        },
                    );
                }
            }
        }
        {
            let mut inbound = mutex_guard(&self.inner.inbound_groups)?;
            let mut groups = write_guard(&self.inner.groups)?;
            for ((group_id, sender), (name, pickle)) in stored_group_inbound {
                if let Ok(session) = InboundGroup::from_json(&pickle) {
                    inbound
                        .entry(group_id.clone())
                        .or_default()
                        .insert(sender, session);
                    groups.entry(group_id).or_insert(GroupInfoState {
                        name,
                        members: Vec::new(),
                        my_role: None,
                        avatar_url: None,
                        outbound: None,
                    });
                }
            }
        }

        // Contacts come back as rows: the ordered contact list, their learned
        // display names, their avatar paths and the last-seen timestamps that
        // seed the presence cache before any live push arrives.
        let mut contact_names = HashMap::new();
        let mut contact_avatars = HashMap::new();
        let mut contacts = Vec::new();
        let mut presence = HashMap::new();
        for contact in stored_contacts {
            if let Some(name) = contact.display_name.clone() {
                contact_names.insert(contact.peer_id.clone(), name);
            }
            if let Some(avatar) = contact.avatar_url.clone() {
                contact_avatars.insert(contact.peer_id.clone(), avatar);
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
        settings.notification_sound = setting_bool(notification_sound, true);
        settings.language = language.filter(|value| !value.is_empty());
        settings.minimize_to_tray = setting_bool(minimize_to_tray, false);
        settings.enter_to_send = setting_bool(enter_to_send, true);
        settings.message_font_scale = message_font_scale.filter(|value| !value.is_empty());
        settings.autostart = setting_bool(autostart, false);
        *write_guard(&self.inner.settings)? = settings;

        let mut profiles = read_guard(&self.inner.profiles)?.clone();
        profiles.my_display_name = my_display_name.filter(|name| !name.is_empty());
        profiles.my_username = my_username.filter(|name| !name.is_empty());
        profiles.my_avatar_url = my_avatar_url.filter(|url| !url.is_empty());
        profiles.contacts = contact_names;
        profiles.contact_avatars = contact_avatars;
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
            for (group_id, senders) in inbound_sessions.iter() {
                let name = groups
                    .get(group_id)
                    .map(|g| g.name.clone())
                    .unwrap_or_default();
                for (sender, session) in senders.iter() {
                    if let Ok(json) = session.to_json() {
                        inbound.insert((group_id.clone(), sender.clone()), (name.clone(), json));
                    }
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
    // Local data operations
    // ---------------------------------------------------------------------

    /// Wipe the entire message history on THIS device: every decrypted message
    /// in memory and every row in the encrypted store. Contacts, sessions,
    /// groups and settings are deliberately kept — only the conversation
    /// history (and thus nothing else) disappears.
    pub fn clear_chat_history(&self) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        write_guard(&self.inner.messages)?.clear();
        let store_guard = self.store_guard()?;
        let store = store_guard.as_ref().ok_or(RelayError::StoreNotOpen)?;
        store.clear_messages()?;
        Ok(())
    }

    /// Drop the cached identity and every piece of state derived from it so the
    /// NEXT `connect` reloads the identity file from disk. Used after an
    /// `import_identity` so a freshly restored identity takes effect without a
    /// full process restart — the webview reloads afterwards and re-runs the
    /// whole startup. In-memory settings/profile data are not wiped here; the
    /// next `connect` re-hydrates them from the restored identity's store,
    /// which is keyed to the new peer ID (like `reset`, but without deleting
    /// the old database file — the old history simply becomes unreachable).
    pub fn reload_identity(&self) -> Result<(), RelayError> {
        self.inner.auto_reconnect.store(false, Ordering::SeqCst);
        self.mark_disconnected();
        mutex_guard(&self.inner.identity)?.take();
        mutex_guard(&self.inner.sessions)?.clear();
        mutex_guard(&self.inner.inbound_groups)?.clear();
        write_guard(&self.inner.groups)?.clear();
        write_guard(&self.inner.messages)?.clear();
        write_guard(&self.inner.contacts)?.clear();
        write_guard(&self.inner.presence)?.clear();
        write_guard(&self.inner.friend_requests_incoming)?.clear();
        write_guard(&self.inner.friend_requests_outgoing)?.clear();
        *write_guard(&self.inner.store)? = None;
        if let Ok(mut seen) = self.inner.seen_envelopes.lock() {
            seen.clear();
        }
        self.inner.profile_synced.store(false, Ordering::SeqCst);
        Ok(())
    }

    // ---------------------------------------------------------------------
    // State recording
    // ---------------------------------------------------------------------

    /// Record an inbound plaintext message and add the sender as a contact.
    ///
    /// When the sender embedded their own `message_id` in the encrypted
    /// payload, the message is stored under that SAME id — the shared id is
    /// what lets reactions and replies (which reference the sender's id)
    /// resolve on both ends. Legacy messages without one get a local `in-N`.
    fn record_incoming(
        &self,
        peer_id: &str,
        text: String,
        quote: Option<Quote>,
        message_id: Option<String>,
    ) -> Result<UIMessage, RelayError> {
        let message = UIMessage {
            id: match message_id {
                Some(id) => id,
                None => format!(
                    "in-{}",
                    self.inner.next_msg_id.fetch_add(1, Ordering::SeqCst)
                ),
            },
            text,
            outgoing: false,
            timestamp: now_millis(),
            status: "delivered".to_string(),
            quote,
            reactions: Vec::new(),
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
        quote: Option<Quote>,
    ) -> Result<UIMessage, RelayError> {
        let id = self.next_message_id(client_id);
        self.record_outgoing_with_id(peer_id, id, text, quote)
    }

    /// Decide the id for the next outgoing message: the UI-provided client id
    /// (a UUID in practice), or a generated `out-N` when none was supplied.
    fn next_message_id(&self, client_id: &str) -> String {
        if client_id.is_empty() {
            format!(
                "out-{}",
                self.inner.next_msg_id.fetch_add(1, Ordering::SeqCst)
            )
        } else {
            client_id.to_string()
        }
    }

    /// Record an outbound message under an already-decided id. `send_message`
    /// uses this so the id that travels inside the encrypted payload (where
    /// the recipient picks it up) is exactly the id the sender stores the
    /// message under — that shared id is what makes reactions and replies
    /// resolve on both ends.
    fn record_outgoing_with_id(
        &self,
        peer_id: &str,
        id: String,
        text: &str,
        quote: Option<Quote>,
    ) -> Result<UIMessage, RelayError> {
        let message = UIMessage {
            id,
            text: text.to_string(),
            outgoing: true,
            timestamp: now_millis(),
            status: "sent".to_string(),
            quote,
            reactions: Vec::new(),
        };
        write_guard(&self.inner.messages)?
            .entry(peer_id.to_string())
            .or_default()
            .push(message.clone());
        self.persist_message(peer_id, &message, None)?;
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
                    curve25519_key: None,
                    verified: false,
                })?;
            let client = self.clone();
            let peer = peer_id.to_string();
            tauri::async_runtime::spawn(async move {
                // Best-effort: a peer that never published pre-keys simply has
                // no name to learn, and the fetch fails silently.
                if let Ok((_, Some(name))) = client.fetch_prekeys(&peer).await {
                    let _ = client.remember_contact_name(&peer, &name);
                }
                // Learn the peer's public avatar too (when they have one), so
                // the contact list and chat header render the image instead of
                // a letter tile. `get_profile` persists what it learns.
                let _ = client.get_profile(&peer).await;
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
        self.send_raw(WsMessage::Text(text.into()))
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
    ///
    /// An unexpected drop (the inbound loop ending) reconnects automatically
    /// with exponential backoff. A user-initiated `disconnect()`/`reset()` sets
    /// `auto_reconnect` to `false` BEFORE calling this, so those never enter
    /// the retry loop.
    fn mark_disconnected(&self) {
        self.inner.connected.store(false, Ordering::SeqCst);
        if let Ok(mut outbox) = self.inner.outbox.write() {
            *outbox = None;
        }
        let _ = self
            .inner
            .app
            .emit("relay-status", RelayStatusEvent { connected: false });
        if self.inner.auto_reconnect.load(Ordering::SeqCst) {
            self.spawn_reconnect_loop();
        } else {
            let _ = self.inner.app.emit(
                "reconnecting",
                ReconnectingEvent {
                    active: false,
                    attempt: 0,
                    next_in_ms: 0,
                },
            );
        }
    }

    /// Spawn the auto-reconnect loop for a dropped connection.
    ///
    /// The loop sleeps for the exponential backoff (2s → 5s → 10s → 20s →
    /// 30s cap), retries `connect()` and keeps retrying until it succeeds or
    /// `auto_reconnect` flips to `false` (a user `disconnect()`/`reset()`).
    /// Progress is announced through the `reconnecting` event so the UI can
    /// render a "Reconnecting…" state instead of a dead "Disconnected".
    /// At most one loop runs at a time — a second entry while one is already
    /// running is a no-op.
    fn spawn_reconnect_loop(&self) {
        if self
            .inner
            .reconnect_loop_active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let client = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut retry_index: u32 = 0;
            while client.inner.auto_reconnect.load(Ordering::SeqCst) {
                let backoff = std::time::Duration::from_secs(reconnect_backoff_secs(retry_index));
                let attempt = retry_index + 1;
                let _ = client.inner.app.emit(
                    "reconnecting",
                    ReconnectingEvent {
                        active: true,
                        attempt,
                        next_in_ms: backoff.as_millis() as u64,
                    },
                );
                tokio::time::sleep(backoff).await;
                if !client.inner.auto_reconnect.load(Ordering::SeqCst) {
                    break;
                }
                match client.connect().await {
                    Ok(()) => {
                        // A user-initiated disconnect may have raced with the
                        // successful reconnect; if so, tear the fresh
                        // connection down so the user's intent wins.
                        if !client.inner.auto_reconnect.load(Ordering::SeqCst) {
                            client.mark_disconnected();
                        }
                        client
                            .inner
                            .reconnect_loop_active
                            .store(false, Ordering::SeqCst);
                        let _ = client.inner.app.emit(
                            "reconnecting",
                            ReconnectingEvent {
                                active: false,
                                attempt,
                                next_in_ms: 0,
                            },
                        );
                        return;
                    }
                    Err(_) => {
                        retry_index += 1;
                    }
                }
            }
            client
                .inner
                .reconnect_loop_active
                .store(false, Ordering::SeqCst);
            let _ = client.inner.app.emit(
                "reconnecting",
                ReconnectingEvent {
                    active: false,
                    attempt: 0,
                    next_in_ms: 0,
                },
            );
        });
    }

    /// Delete one message locally ("delete for me"): the decrypted history in
    /// memory and its row in the encrypted store. The peer's copy and any
    /// relay-queued envelopes are untouched. An unknown thread or message id
    /// is an idempotent no-op.
    pub fn delete_message(&self, peer_id: &str, message_id: &str) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        let removed = {
            let mut messages = write_guard(&self.inner.messages)?;
            let msgs = match messages.get_mut(peer_id) {
                Some(msgs) => msgs,
                None => return Ok(()),
            };
            let before = msgs.len();
            msgs.retain(|message| message.id != message_id);
            before != msgs.len()
        };
        if !removed {
            return Ok(());
        }
        self.store_guard()?
            .as_ref()
            .ok_or(RelayError::StoreNotOpen)?
            .delete_message(message_id)?;
        // Drop any pending ack for the deleted message so a late relay ack can
        // neither flip its status nor resurrect the row.
        if let Ok(mut pending_acks) = self.inner.pending_acks.lock() {
            pending_acks.retain(|_, id| id != message_id);
        }
        Ok(())
    }
}

/// Current time as epoch milliseconds.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Backoff delay in seconds for the zero-based retry index. The schedule is
/// 2, 5, 10, 20 seconds, then capped at 30 seconds for every later retry.
fn reconnect_backoff_secs(retry_index: u32) -> u64 {
    RECONNECT_BACKOFF_SECS
        .get(retry_index as usize)
        .copied()
        .unwrap_or(
            *RECONNECT_BACKOFF_SECS
                .last()
                .expect("schedule is non-empty"),
        )
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
                quote: None,
                reactions: Vec::new(),
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
                quote: None,
                reactions: Vec::new(),
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
                quote: None,
                reactions: Vec::new(),
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
                quote: None,
                reactions: Vec::new(),
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
    // Auto-reconnect backoff
    // ---------------------------------------------------------------------

    #[test]
    fn reconnect_backoff_follows_the_schedule_and_caps_at_30_seconds() {
        // 2s → 5s → 10s → 20s, then capped at 30s for every later retry.
        assert_eq!(reconnect_backoff_secs(0), 2);
        assert_eq!(reconnect_backoff_secs(1), 5);
        assert_eq!(reconnect_backoff_secs(2), 10);
        assert_eq!(reconnect_backoff_secs(3), 20);
        assert_eq!(reconnect_backoff_secs(4), 30);
        assert_eq!(reconnect_backoff_secs(5), 30);
        assert_eq!(reconnect_backoff_secs(100), 30);
    }

    #[test]
    fn reconnecting_event_serializes_for_the_ui() {
        let event = ReconnectingEvent {
            active: true,
            attempt: 2,
            next_in_ms: 5_000,
        };
        let json = serde_json::to_value(&event).expect("serialize");
        assert_eq!(json["active"], true);
        assert_eq!(json["attempt"], 2);
        assert_eq!(json["next_in_ms"], 5000);

        let ended = ReconnectingEvent {
            active: false,
            attempt: 0,
            next_in_ms: 0,
        };
        let json = serde_json::to_value(&ended).expect("serialize");
        assert_eq!(json["active"], false);
    }

    // ---------------------------------------------------------------------
    // Reactions & quoted replies
    // ---------------------------------------------------------------------

    #[test]
    fn reaction_event_serializes_for_the_ui() {
        let event = ReactionEvent {
            peer_id: "group-1".into(),
            message_id: "m-7".into(),
            sender: "alice".into(),
            emoji: "👍".into(),
            active: true,
        };
        let json = serde_json::to_value(&event).expect("serialize");
        assert_eq!(json["peer_id"], "group-1");
        assert_eq!(json["message_id"], "m-7");
        assert_eq!(json["sender"], "alice");
        assert_eq!(json["emoji"], "👍");
        assert_eq!(json["active"], true);
    }

    #[test]
    fn ui_message_serializes_quote_and_reactions_for_the_ui() {
        let message = UIMessage {
            id: "m-1".into(),
            text: "my reply".into(),
            outgoing: true,
            timestamp: 0,
            status: "sent".into(),
            quote: Some(Quote::new(
                "m-0",
                "original",
                "bob",
                Some("Bob".to_string()),
            )),
            reactions: vec![ReactionView {
                sender: "bob".into(),
                emoji: "🔥".into(),
            }],
        };
        let json = serde_json::to_value(&message).expect("serialize");
        assert_eq!(json["quote"]["message_id"], "m-0");
        assert_eq!(json["quote"]["sender_name"], "Bob");
        assert_eq!(json["reactions"][0]["emoji"], "🔥");
    }

    #[test]
    fn ui_message_without_quote_or_reactions_omits_the_keys() {
        let message = UIMessage {
            id: "m-1".into(),
            text: "plain".into(),
            outgoing: true,
            timestamp: 0,
            status: "sent".into(),
            quote: None,
            reactions: Vec::new(),
        };
        let json = serde_json::to_value(&message).expect("serialize");
        assert!(json.get("quote").is_none(), "absent quote must be skipped");
        assert!(
            json.get("reactions").is_none(),
            "empty reactions must be skipped"
        );
    }
}
