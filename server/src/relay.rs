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
//!
//! # Module layout
//!
//! This module is the relay core: the shared wire layer ([`Envelope`],
//! [`ClientMessage`], [`ServerMessage`]), the [`Relay`]/[`RelayInner`] state,
//! the socket lifecycle ([`Relay::handle_socket`], hello authentication,
//! envelope routing, the offline queue) and the rate-limiter wiring. The
//! domain-specific `impl Relay` blocks live in sibling modules declared below:
//!
//! - [`prekeys`]: pre-key bundle publishing and fetching.
//! - [`profiles`]: username/profile registration, search, display names and
//!   avatars.
//! - [`groups`]: group rosters, owner/admin roles and group-message fan-out.
//! - [`presence`]: presence watches, queries, pushes, last-seen and privacy.
//! - [`ratelimit`]: per-IP token-bucket rate limiting.

#[path = "contacts.rs"]
pub(crate) mod contacts;
#[path = "groups.rs"]
pub(crate) mod groups;
#[path = "prekeys.rs"]
pub(crate) mod prekeys;
#[path = "presence.rs"]
pub(crate) mod presence;
#[path = "profiles.rs"]
pub(crate) mod profiles;
#[path = "ratelimit.rs"]
pub(crate) mod ratelimit;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use e2ee_core::prekey::PreKeyBundle;
use e2ee_core::{Identity, SignedHello};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};

use ratelimit::RateLimiter;

use crate::store::{unix_now, Store};

/// Upper bound for a single relayed envelope (ciphertext blob size cap).
/// Keeps the server DoS-resistant and the network light.
const MAX_ENVELOPE_BYTES: usize = 8 * 1024 * 1024; // 8 MiB

/// Seconds a client may take to send its `hello` before being dropped.
const HELLO_TIMEOUT_SECS: u64 = 10;

/// Maximum length of a public display name, in Unicode characters.
pub(crate) const MAX_DISPLAY_NAME_CHARS: usize = 64;

/// Maximum length of a group name, in Unicode characters.
pub(crate) const MAX_GROUP_NAME_CHARS: usize = 64;

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
    /// Toggle whether this peer's online status and last-seen are visible to
    /// other peers. When hidden, every `get_presence` reply and every
    /// `broadcast_presence` push for this peer reports `online: false` with
    /// `last_seen: null`, so no one can tell when the peer is online or when
    /// it was last seen. The relay replies with `privacy_updated`.
    #[serde(rename = "set_privacy")]
    SetPrivacy { presence_visible: bool },
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
    /// Promote `peer_id` to a group admin. Only the owner or an existing
    /// admin may promote; promoting a member makes them an admin and
    /// promoting an admin is a no-op.
    #[serde(rename = "promote_member")]
    PromoteMember { group_id: String, peer_id: String },
    /// Demote `peer_id` from admin back to a regular member. Only the group
    /// owner may demote, and the owner can never demote themselves.
    #[serde(rename = "demote_member")]
    DemoteMember { group_id: String, peer_id: String },
    /// Remove `peer_id` from a group's roster. Only the owner may remove a
    /// member, and the owner cannot remove themselves.
    #[serde(rename = "remove_member")]
    RemoveMember { group_id: String, peer_id: String },
    /// Transfer group ownership to `new_owner_peer_id`. Only the current owner
    /// may call this; on success the old owner becomes an admin and the new
    /// owner takes over the owner role.
    #[serde(rename = "transfer_ownership")]
    TransferOwnership {
        group_id: String,
        new_owner_peer_id: String,
    },
    /// Set a group's avatar image. `avatar` is a base64 image blob of at most
    /// 2 MiB; the relay stores it content-addressed in the media directory and
    /// exposes it through `get_group_info` as `avatar_url`. Only the group
    /// owner or an admin may change the avatar.
    #[serde(rename = "set_group_avatar")]
    SetGroupAvatar { group_id: String, avatar: String },
    /// Send a friend request to `peer_id`. The recipient receives a
    /// `friend_request_received` push when online; offline recipients find it
    /// via `get_friend_requests` on their next connect.
    #[serde(rename = "send_friend_request")]
    SendFriendRequest { peer_id: String },
    /// Accept a pending friend request from `peer_id`: the two peers become
    /// accepted contacts and both receive a `friend_request_accepted` push.
    #[serde(rename = "accept_friend_request")]
    AcceptFriendRequest { peer_id: String },
    /// Decline a pending friend request from `peer_id`. The requester is
    /// pushed a `friend_request_declined` notification.
    #[serde(rename = "decline_friend_request")]
    DeclineFriendRequest { peer_id: String },
    /// List the caller's pending incoming and outgoing friend requests.
    #[serde(rename = "get_friend_requests")]
    GetFriendRequests,
    /// Remove `peer_id` from the caller's contacts. Both peers receive a
    /// `contact_removed` push.
    #[serde(rename = "remove_contact")]
    RemoveContact { peer_id: String },
    /// Invite `peer_id` to join `group_id`. The invitee accepts or declines;
    /// they are NOT added to the roster until they accept.
    #[serde(rename = "group_invite")]
    GroupInvite { group_id: String, peer_id: String },
    /// Accept a pending invite to `group_id`: the caller joins the roster and
    /// every member is pushed a `group_member_added`.
    #[serde(rename = "group_invite_accept")]
    GroupInviteAccept { group_id: String },
    /// Decline a pending invite to `group_id`.
    #[serde(rename = "group_invite_decline")]
    GroupInviteDecline { group_id: String },
    /// List the caller's pending group invites as (group_id, group_name, inviter).
    #[serde(rename = "get_group_invites")]
    GetGroupInvites,
}

/// Messages the SERVER sends to the client.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ServerMessage {
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
    /// online or when it has never been seen. Peers that hide their presence
    /// (`set_privacy`) are always reported as `online: false` with a `null`
    /// `last_seen`, even while connected.
    #[serde(rename = "presence")]
    Presence {
        peer_id: String,
        online: bool,
        last_seen: Option<i64>,
    },
    /// Confirmation that the caller's privacy settings were updated.
    #[serde(rename = "privacy_updated")]
    PrivacyUpdated,
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
    /// reply). `members` carries each member's current role (owner/admin/
    /// member) so clients can render role badges and permission-gated
    /// controls. `avatar_url` is the public path of the group avatar blob
    /// (`/media/{hash}`), `null` when the group has none.
    #[serde(rename = "group_info")]
    GroupInfo {
        group_id: String,
        name: String,
        owner_peer_id: String,
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
    /// Reply to `send_friend_request`: the request was persisted.
    #[serde(rename = "friend_request_sent")]
    FriendRequestSent,
    /// Push to the recipient of a friend request, naming the requester and
    /// its public display name. Only sent when the recipient is online;
    /// offline recipients retrieve pending requests via `get_friend_requests`.
    #[serde(rename = "friend_request_received")]
    FriendRequestReceived {
        peer_id: String,
        display_name: Option<String>,
    },
    /// Reply to `accept_friend_request`: the caller is now contacts with the
    /// other peer.
    #[serde(rename = "friend_request_accepted_ok")]
    FriendRequestAcceptedOk,
    /// Push naming the peer the caller just became contacts with. Sent to BOTH
    /// sides of the relationship, so the accepting client treats it as its
    /// confirmation too.
    #[serde(rename = "friend_request_accepted")]
    FriendRequestAccepted { peer_id: String },
    /// Reply to `decline_friend_request`.
    #[serde(rename = "friend_request_declined_ok")]
    FriendRequestDeclinedOk,
    /// Push to the requester, naming the peer that declined their request.
    #[serde(rename = "friend_request_declined")]
    FriendRequestDeclined { peer_id: String },
    /// Reply to `remove_contact`.
    #[serde(rename = "contact_removed_ok")]
    ContactRemovedOk,
    /// Push naming the peer the caller is no longer contacts with. Sent to BOTH
    /// sides of the relationship.
    #[serde(rename = "contact_removed")]
    ContactRemoved { peer_id: String },
    /// Reply to `get_friend_requests`: pending incoming requests (requester +
    /// display name) and outgoing requests (target peer IDs).
    #[serde(rename = "friend_requests")]
    FriendRequests {
        incoming: Vec<FriendRequestIncoming>,
        outgoing: Vec<String>,
    },
    /// Reply to `group_invite` for the inviter.
    #[serde(rename = "group_invite_sent")]
    GroupInviteSent,
    /// Push to the invitee: `inviter_peer_id` invites them to `group_name`.
    #[serde(rename = "group_invite_received")]
    GroupInviteReceived {
        group_id: String,
        group_name: String,
        inviter_peer_id: String,
    },
    /// Reply to `group_invite_accept` for the accepter.
    #[serde(rename = "group_invite_accepted_ok")]
    GroupInviteAcceptedOk,
    /// Push to the inviter when the invitee accepted the invite.
    #[serde(rename = "group_invite_accepted")]
    GroupInviteAccepted { group_id: String, peer_id: String },
    /// Reply to `group_invite_decline` for the decliner.
    #[serde(rename = "group_invite_declined_ok")]
    GroupInviteDeclinedOk,
    /// Push to the inviter when the invitee declined the invite.
    #[serde(rename = "group_invite_declined")]
    GroupInviteDeclined { group_id: String, peer_id: String },
    /// Reply to `get_group_invites`: the caller's pending group invites.
    #[serde(rename = "group_invites")]
    GroupInvites { invites: Vec<GroupInviteInfo> },
    /// Protocol error.
    Error { code: String },
}

/// One pending group invite as reported to the invitee.
#[derive(Debug, Clone, Serialize)]
pub struct GroupInviteInfo {
    pub group_id: String,
    pub group_name: String,
    pub inviter_peer_id: String,
}

/// One member of a group's roster, with its current role.
///
/// The role is relay-managed group metadata (owner/admin/member) and is NOT
/// secret — it never carries key material or plaintext.
#[derive(Debug, Clone, Serialize)]
pub struct GroupMember {
    /// The member's peer ID (fingerprint).
    pub peer_id: String,
    /// "owner", "admin" or "member".
    pub role: String,
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

/// One pending incoming friend request (the `get_friend_requests` reply).
#[derive(Debug, Serialize)]
pub struct FriendRequestIncoming {
    /// Peer ID of the requester.
    pub peer_id: String,
    /// The requester's public display name, if one was set.
    pub display_name: Option<String>,
}

// ---------------------------------------------------------------------------
// Relay state
// ---------------------------------------------------------------------------

/// Peer ID (fingerprint) type: hash of the peer's X25519 identity key.
pub(crate) type PeerId = String;
/// Outbound channel: WS messages queued for a connected peer.
pub(crate) type Outbound = mpsc::UnboundedSender<WsMessage>;

/// One presence subscription. The peer ID lets the relay de-duplicate
/// re-watches (one channel per watching peer) and clean up its own
/// registrations when the watcher disconnects — an `UnboundedSender` alone
/// carries no identity, so it cannot serve either purpose.
#[derive(Clone)]
pub(crate) struct PresenceWatcher {
    /// Peer ID of the subscribing socket.
    pub(crate) peer_id: PeerId,
    /// The watcher's outbound WS channel (its `online` entry).
    pub(crate) tx: Outbound,
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
    pub(crate) inner: Arc<RelayInner>,
}

/// Shared relay state, reachable through the [`Relay`] handle.
pub(crate) struct RelayInner {
    /// Online peers: peer_id -> outbound channel.
    pub(crate) online: RwLock<HashMap<PeerId, Outbound>>,
    /// Presence subscriptions: watched peer_id -> its watchers' channels.
    pub(crate) presence_watchers: RwLock<HashMap<PeerId, Vec<PresenceWatcher>>>,
    /// SQLite-backed offline queue of opaque ciphertext blobs.
    pub(crate) store: Store,
    /// Per-IP envelope throughput guard.
    pub(crate) limiter: RateLimiter,
    /// Per-IP guard for profile mutations and directory lookups.
    pub(crate) profile_limiter: RateLimiter,
    /// Per-IP guard for friend-request/contact mutations.
    pub(crate) contacts_limiter: RateLimiter,
    /// Directory holding uploaded avatar blobs (`<sha256>.bin`).
    pub(crate) media_dir: PathBuf,
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
            RateLimiter::from_contacts_env(),
        )
    }

    /// Build a relay over a pre-opened store (tests use in-memory stores).
    #[cfg(test)]
    pub(crate) fn with_store(store: Store) -> Self {
        Self::with_parts(
            store,
            Self::default_media_dir(),
            RateLimiter::from_env(),
            RateLimiter::from_profile_env(),
            RateLimiter::from_contacts_env(),
        )
    }

    /// Build a relay over a pre-opened store with a deterministic rate
    /// limiter (unit tests need exact bucket sizes).
    #[cfg(test)]
    pub(crate) fn with_limiter(store: Store, burst: f64, refill: f64) -> Self {
        Self::with_parts(
            store,
            Self::default_media_dir(),
            RateLimiter::new(burst, refill),
            RateLimiter::new(burst, refill),
            RateLimiter::new(burst, refill),
        )
    }

    /// Build a relay over a pre-opened store with a scratch media directory
    /// and a generous profile bucket (unit tests only).
    pub(crate) fn with_parts(
        store: Store,
        media_dir: PathBuf,
        limiter: RateLimiter,
        profile_limiter: RateLimiter,
        contacts_limiter: RateLimiter,
    ) -> Self {
        Self {
            inner: Arc::new(RelayInner {
                online: RwLock::new(HashMap::new()),
                presence_watchers: RwLock::new(HashMap::new()),
                store,
                limiter,
                profile_limiter,
                contacts_limiter,
                media_dir,
            }),
        }
    }

    /// The media directory used when no database path is known: `data/media`
    /// (i.e. `server/data/media` when the relay runs from the server dir).
    #[cfg(test)]
    pub(crate) fn default_media_dir() -> PathBuf {
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
        let online_count = self.inner.online.read().await.len();
        tracing::info!(peer = %peer_id, ip = %ip, online = online_count, "peer online");

        // 2b) Announce the peer is online to everyone watching them. Any peer
        //     that reconnects mid-watch sees a fresh `online: true` push.
        self.broadcast_presence(&peer_id, true).await;

        // 3) Push any ciphertext blobs persisted while the peer was offline.
        //    Rows are left in the DB until a fetch_since drains them, so the
        //    client can re-pull its offline history.
        let blobs = self.inner.store.list_for(&peer_id, unix_now());
        for env in blobs {
            if let Ok(text) = serde_json::to_string(&ServerMessage::Envelope { envelope: env }) {
                if tx.send(WsMessage::Text(text.into())).await.is_err() {
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
                        Ok(ClientMessage::SetPrivacy { presence_visible }) => {
                            self.set_privacy(&peer_id, &ip, presence_visible).await;
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
                        Ok(ClientMessage::PromoteMember {
                            group_id,
                            peer_id: target,
                        }) => {
                            self.promote_member(&peer_id, &ip, &group_id, &target).await;
                        }
                        Ok(ClientMessage::DemoteMember {
                            group_id,
                            peer_id: target,
                        }) => {
                            self.demote_member(&peer_id, &ip, &group_id, &target).await;
                        }
                        Ok(ClientMessage::RemoveMember {
                            group_id,
                            peer_id: target,
                        }) => {
                            self.remove_member(&peer_id, &ip, &group_id, &target).await;
                        }
                        Ok(ClientMessage::TransferOwnership {
                            group_id,
                            new_owner_peer_id,
                        }) => {
                            self.transfer_ownership(&peer_id, &ip, &group_id, &new_owner_peer_id)
                                .await;
                        }
                        Ok(ClientMessage::SetGroupAvatar { group_id, avatar }) => {
                            self.set_group_avatar(&peer_id, &ip, &group_id, &avatar)
                                .await;
                        }
                        Ok(ClientMessage::GroupInvite {
                            group_id,
                            peer_id: target,
                        }) => {
                            self.group_invite(&peer_id, &ip, &group_id, &target).await;
                        }
                        Ok(ClientMessage::GroupInviteAccept { group_id }) => {
                            self.group_invite_accept(&peer_id, &ip, &group_id).await;
                        }
                        Ok(ClientMessage::GroupInviteDecline { group_id }) => {
                            self.group_invite_decline(&peer_id, &ip, &group_id).await;
                        }
                        Ok(ClientMessage::GetGroupInvites) => {
                            self.get_group_invites(&peer_id, &ip).await;
                        }
                        Ok(ClientMessage::SendFriendRequest { peer_id: target }) => {
                            self.send_friend_request(&peer_id, &ip, &target).await;
                        }
                        Ok(ClientMessage::AcceptFriendRequest { peer_id: target }) => {
                            self.accept_friend_request(&peer_id, &ip, &target).await;
                        }
                        Ok(ClientMessage::DeclineFriendRequest { peer_id: target }) => {
                            self.decline_friend_request(&peer_id, &ip, &target).await;
                        }
                        Ok(ClientMessage::GetFriendRequests) => {
                            self.get_friend_requests(&peer_id, &ip).await;
                        }
                        Ok(ClientMessage::RemoveContact { peer_id: target }) => {
                            self.remove_contact(&peer_id, &ip, &target).await;
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
        let online_count = self.inner.online.read().await.len();
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
        tracing::info!(peer = %peer_id, online = online_count, "peer disconnected");
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
                        serde_json::to_string(&ServerMessage::Error { code })
                            .unwrap()
                            .into(),
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
                        .unwrap()
                        .into(),
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
    pub(crate) fn is_valid_display_name(name: &str) -> bool {
        !name.is_empty()
            && name.chars().count() <= MAX_DISPLAY_NAME_CHARS
            && !name.chars().any(char::is_control)
    }

    /// Whether `name` is acceptable as a group name: 1-64 Unicode characters
    /// and free of control characters.
    pub(crate) fn is_valid_group_name(name: &str) -> bool {
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

        // Contact gate: 1:1 envelopes may only flow between ACCEPTED contacts.
        // This is the server-level anti-spam boundary — a stranger's ciphertext
        // is never routed, queued or acked. Group sends use a separate path
        // (`send_group_message`) where group membership is the only requirement.
        if !self
            .inner
            .store
            .are_contacts(&envelope.sender, &envelope.recipient)
        {
            tracing::warn!(
                sender = %envelope.sender,
                recipient = %envelope.recipient,
                "envelope between non-contacts dropped"
            );
            let _ = self
                .send(
                    sender_peer,
                    ServerMessage::Error {
                        code: "not_contacts".into(),
                    },
                )
                .await;
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
        tracing::debug!(
            sender = %sender_peer,
            recipient = %envelope.recipient,
            seq,
            "envelope routed"
        );
    }

    /// Deliver a single envelope to its recipient: live to an online socket,
    /// otherwise into the SQLite offline queue. Never rate limits and never
    /// acks — shared by 1:1 routing and group fan-out.
    pub(crate) async fn deliver_one(&self, envelope: &Envelope) {
        let delivered = {
            let online = self.inner.online.read().await;
            match online.get(&envelope.recipient) {
                Some(tx) => {
                    let msg = serde_json::to_string(&ServerMessage::Envelope {
                        envelope: envelope.clone(),
                    });
                    match msg {
                        Ok(text) => {
                            let _ = tx.send(WsMessage::Text(text.into()));
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
    pub(crate) async fn take_group_slot(&self, peer_id: &str, ip: &str) -> bool {
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

    /// Resolve the on-disk path of a media blob. The `/media/{hash}` endpoint
    /// in main.rs uses this; the caller is responsible for validating `hash`.
    pub fn media_path(&self, hash: &str) -> PathBuf {
        self.inner.media_dir.join(format!("{hash}.bin"))
    }

    /// Send a server message to a specific peer if they are online.
    pub(crate) async fn send(&self, peer_id: &str, msg: ServerMessage) -> bool {
        let online = self.inner.online.read().await;
        match online.get(peer_id) {
            Some(tx) => match serde_json::to_string(&msg) {
                Ok(text) => {
                    let _ = tx.send(WsMessage::Text(text.into()));
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

/// Shared helpers for the relay unit tests. Kept in one place so the sibling
/// feature modules (prekeys, profiles, groups, presence) can reuse them.
#[cfg(test)]
pub(crate) mod test_utils {
    use super::*;

    /// Build a minimal routing envelope.
    pub(crate) fn env(sender: &str, recipient: &str, seq: u64) -> Envelope {
        Envelope {
            sender: sender.into(),
            recipient: recipient.into(),
            payload: format!("blob-{seq}"),
            seq,
        }
    }

    /// Read the single text reply queued for a peer and parse it as JSON.
    pub(crate) fn read_reply(rx: &mut mpsc::UnboundedReceiver<WsMessage>) -> serde_json::Value {
        let msg = rx.try_recv().expect("a reply must be queued");
        let text = match msg {
            WsMessage::Text(t) => t,
            _ => panic!("expected a text reply"),
        };
        serde_json::from_str(&text).expect("reply must be valid JSON")
    }

    /// Register an identity's keys in the store and wire an outbound channel
    /// so the peer can receive relay replies.
    pub(crate) async fn online_peer(
        relay: &Relay,
        identity: &Identity,
    ) -> mpsc::UnboundedReceiver<WsMessage> {
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

    /// Sign a username binding with an identity's Ed25519 key (base64).
    pub(crate) fn sign_username(identity: &Identity, username: &str) -> String {
        e2ee_core::sign_username(identity, username).to_base64()
    }

    /// Establish an accepted contact relationship between `a` and `b` directly
    /// in the store (as if `a` had requested and `b` had accepted). Tests that
    /// drive `add_group_member`, `fetch_prekeys` or 1:1 routing need the two
    /// peers to be accepted contacts first.
    pub(crate) fn make_contacts(relay: &Relay, a: &str, b: &str) {
        relay.inner.store.upsert_friend_request(a, b).unwrap();
        relay.inner.store.accept_friend(a, b).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::test_utils::env;
    use crate::store::{ENVELOPE_TTL_SECS, MAX_OFFLINE_BLOBS};

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
}
