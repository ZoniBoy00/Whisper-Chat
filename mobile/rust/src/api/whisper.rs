//! Whisper mobile core — E2EE chat over the same zero-knowledge relay the
//! desktop client uses. Full wire protocol: profiles, contacts, groups,
//! presence, reactions, edits, disappearing messages.
//!
//! Everything crypto lives in `e2ee-core` (identity, X3DH + Double Ratchet,
//! Megolm groups); this crate adds the wire protocol, the WebSocket
//! connection and an event queue the Flutter UI polls. The relay address is
//! hardcoded on purpose (mirroring the desktop client).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use e2ee_core::{
    parse_plaintext, ChatPayload, ChatSession, Envelope, EnvelopeContent, Handshake, Identity,
    Message, ParsedPayload, PreKeyBundle, Quote, TextPayload,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

/// Hardcoded relay endpoint — mirror of the desktop `DEFAULT_RELAY_URL`.
pub const DEFAULT_RELAY_URL: &str = "wss://whisper-test.homelab.cfd/ws";

// ---------------------------------------------------------------------------
// Wire protocol (client -> relay)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Hello {
        peer_id: String,
        curve25519_key: String,
        ed25519_key: String,
        signature: String,
        display_name: Option<String>,
    },
    Envelope {
        envelope: RelayEnvelope,
    },
    FetchSince {
        since: u64,
    },
    PublishPrekeys {
        bundle: Box<PreKeyBundle>,
    },
    FetchPrekeys {
        peer_id: String,
    },
    SendFriendRequest {
        peer_id: String,
    },
    AcceptFriendRequest {
        peer_id: String,
    },
    DeclineFriendRequest {
        peer_id: String,
    },
    RemoveContact {
        peer_id: String,
    },
    GetFriendRequests,
    ListContacts,
    WatchPresence {
        peer_id: String,
    },
    GetPresence {
        peer_id: String,
    },
    SetPrivacy {
        presence_visible: bool,
    },
    UpdateProfile {
        display_name: String,
    },
    RegisterProfile {
        username: String,
        signature: String,
        display_name: Option<String>,
        avatar: Option<String>,
    },
    SearchUsers {
        query: String,
        limit: Option<usize>,
    },
    GetProfile {
        peer_id: String,
    },
    CreateGroup {
        name: String,
    },
    AddGroupMember {
        group_id: String,
        peer_id: String,
    },
    LeaveGroup {
        group_id: String,
    },
    GetGroupInfo {
        group_id: String,
    },
    SendGroupMessage {
        group_id: String,
        envelope: RelayEnvelope,
    },
    PromoteMember {
        group_id: String,
        peer_id: String,
    },
    DemoteMember {
        group_id: String,
        peer_id: String,
    },
    RemoveMember {
        group_id: String,
        peer_id: String,
    },
    RenameGroup {
        group_id: String,
        name: String,
    },
    GroupInvite {
        group_id: String,
        peer_id: String,
    },
    GroupInviteAccept {
        group_id: String,
    },
    GroupInviteDecline {
        group_id: String,
    },
    GetGroupInvites,
    GetGroupJoinLink {
        group_id: String,
    },
    JoinGroup {
        group_id: String,
        token: String,
    },
}

/// The routing envelope the relay understands (payload = base64 JSON).
#[derive(Debug, Serialize, Deserialize, Clone)]
struct RelayEnvelope {
    sender: String,
    recipient: String,
    payload: String,
    #[allow(dead_code)] // wire field; the relay acks by seq
    seq: u64,
}

// ---------------------------------------------------------------------------
// Wire protocol (relay -> client)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Hello,
    Envelope {
        envelope: RelayEnvelope,
    },
    Acknowledged {
        #[allow(dead_code)] // wire field; acks are matched by seq on send
        seq: u64,
    },
    Prekeys {
        bundle: Box<PreKeyBundle>,
        #[allow(dead_code)] // serde field; display name is optional metadata
        display_name: Option<String>,
    },
    Error {
        code: String,
    },
    Contacts {
        peers: Vec<String>,
    },
    FriendRequests {
        incoming: Vec<FriendRequestIncoming>,
        outgoing: Vec<String>,
    },
    FriendRequestSent,
    FriendRequestReceived {
        peer_id: String,
        display_name: Option<String>,
    },
    FriendRequestAccepted {
        peer_id: String,
    },
    FriendRequestDeclined {
        peer_id: String,
    },
    ContactRemoved {
        peer_id: String,
    },
    Presence {
        peer_id: String,
        online: bool,
        last_seen: Option<i64>,
    },
    ProfileUpdated,
    ProfileRegistered {
        username: String,
    },
    UsersSearch {
        results: Vec<SearchResult>,
    },
    Profile {
        username: Option<String>,
        peer_id: String,
        display_name: Option<String>,
        avatar_url: Option<String>,
        curve25519_key: Option<String>,
    },
    GroupCreated {
        group_id: String,
        name: String,
        #[allow(dead_code)] // serde field; roster arrives via group_info
        members: Vec<String>,
    },
    GroupMemberAdded {
        group_id: String,
        peer_id: String,
    },
    GroupMemberOnline {
        group_id: String,
        peer_id: String,
    },
    GroupMemberLeft {
        group_id: String,
        peer_id: String,
    },
    GroupInfo {
        group_id: String,
        name: String,
        owner_peer_id: String,
        #[allow(dead_code)] // serde field; avatar rendering is a follow-up
        avatar_url: Option<String>,
        members: Vec<GroupMember>,
    },
    GroupMemberPromoted {
        group_id: String,
        peer_id: String,
    },
    GroupMemberDemoted {
        group_id: String,
        peer_id: String,
    },
    GroupMemberRemoved {
        group_id: String,
        peer_id: String,
    },
    GroupRenamed {
        group_id: String,
        name: String,
    },
    GroupInviteSent,
    GroupInviteAcceptedOk,
    GroupInviteDeclinedOk,
    GroupInviteReceived {
        group_id: String,
        group_name: String,
        inviter_peer_id: String,
    },
    GroupInviteAccepted {
        group_id: String,
        peer_id: String,
    },
    GroupInviteDeclined {
        group_id: String,
        peer_id: String,
    },
    GroupInvites {
        invites: Vec<GroupInviteInfo>,
    },
    GroupJoinLink {
        link: String,
    },
    GroupJoinOk {
        group_id: String,
        group_name: String,
    },
}

#[derive(Debug, Deserialize)]
struct FriendRequestIncoming {
    peer_id: String,
    #[allow(dead_code)] // serde field; the UI shows the peer ID for now
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    username: String,
    peer_id: String,
    display_name: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GroupMember {
    peer_id: String,
    role: String,
}

#[derive(Debug, Deserialize)]
struct GroupInviteInfo {
    group_id: String,
    group_name: String,
    inviter_peer_id: String,
}

// ---------------------------------------------------------------------------
// FFI-facing types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct IdentityInfo {
    pub peer_id: String,
    pub json: String,
}

/// One event emitted by the relay loop, drained by the UI via `take_events`.
/// `kind` is one of: connected, disconnected, message, message_sent, error,
/// contacts, friend_requests, friend_request_received, presence, profile,
/// search_results, group_created, group_info, group_member_added,
/// group_member_left, group_invite_received, group_invites, group_join_ok,
/// group_renamed, session_established.
#[derive(Debug, Clone)]
pub struct ChatEvent {
    pub kind: String,
    pub peer_id: String,
    pub text: Option<String>,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Identity helpers (pure crypto, synchronous)
// ---------------------------------------------------------------------------

pub fn identity_create() -> Result<IdentityInfo, String> {
    let identity = Identity::new();
    let json = identity.to_json().map_err(|e| e.to_string())?;
    Ok(IdentityInfo {
        peer_id: identity.peer_id(),
        json,
    })
}

pub fn identity_from_json(json: &str) -> Result<IdentityInfo, String> {
    let identity = Identity::from_json(json).map_err(|e| e.to_string())?;
    Ok(IdentityInfo {
        peer_id: identity.peer_id(),
        json: json.to_string(),
    })
}

/// Sign a username binding (`username || 0x00 || curve25519_key`).
pub fn sign_username(json: &str, username: &str) -> Result<String, String> {
    let identity = Identity::from_json(json).map_err(|e| e.to_string())?;
    Ok(B64.encode(e2ee_core::sign_username(&identity, username).to_bytes()))
}

/// Build a `whisper://invite` link for our identity.
pub fn invite_link(json: &str) -> Result<String, String> {
    let identity = Identity::from_json(json).map_err(|e| e.to_string())?;
    Ok(e2ee_core::build_invite_link(&identity.peer_id(), None, None))
}

pub fn is_valid_peer_id(peer_id: &str) -> bool {
    peer_id.len() == 24 && peer_id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Compute the 60-digit safety number for our identity vs `their_curve25519`
/// (base64 X25519 public key from a profile). Returns an error for an
/// invalid key.
pub fn safety_number(identity_json: &str, their_curve25519: &str) -> Result<String, String> {
    use base64::Engine as _;
    let identity = Identity::from_json(identity_json).map_err(|e| e.to_string())?;
    let their_bytes = B64
        .decode(their_curve25519)
        .map_err(|_| "invalid curve25519 key".to_string())?;
    let their_key = vodozemac::Curve25519PublicKey::from_bytes(
        their_bytes
            .as_slice()
            .try_into()
            .map_err(|_| "curve25519 key must be 32 bytes".to_string())?,
    );
    Ok(e2ee_core::safety_number(&identity.curve25519_key(), &their_key))
}

/// Short 12-digit safety number fragment for compact surfaces.
pub fn short_safety_number(identity_json: &str, their_curve25519: &str) -> Result<String, String> {
    let identity = Identity::from_json(identity_json).map_err(|e| e.to_string())?;
    let their_bytes = B64
        .decode(their_curve25519)
        .map_err(|_| "invalid curve25519 key".to_string())?;
    let their_key = vodozemac::Curve25519PublicKey::from_bytes(
        their_bytes
            .as_slice()
            .try_into()
            .map_err(|_| "curve25519 key must be 32 bytes".to_string())?,
    );
    Ok(e2ee_core::short_safety_number(
        &identity.curve25519_key(),
        &their_key,
    ))
}

// ---------------------------------------------------------------------------
// The relay client
// ---------------------------------------------------------------------------

pub struct WhisperClient {
    inner: Arc<ClientInner>,
}

impl Clone for WhisperClient {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct ClientInner {
    tx: Mutex<Option<mpsc::UnboundedSender<String>>>,
    events: Mutex<VecDeque<ChatEvent>>,
    identity: Mutex<Option<Identity>>,
    sessions: Mutex<Vec<(String, String)>>, // (peer_id, session_json)
    pending_prekeys: Mutex<VecDeque<oneshot::Sender<Result<PreKeyBundle, String>>>>,
    seq: AtomicU64,
    connected: AtomicBool,
}

impl Default for WhisperClient {
    fn default() -> Self {
        Self::new()
    }
}

impl WhisperClient {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ClientInner {
                tx: Mutex::new(None),
                events: Mutex::new(VecDeque::new()),
                identity: Mutex::new(None),
                sessions: Mutex::new(Vec::new()),
                pending_prekeys: Mutex::new(VecDeque::new()),
                seq: AtomicU64::new(0),
                connected: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.inner.connected.load(Ordering::SeqCst)
    }

    fn push_event(&self, kind: &str, peer_id: &str, text: Option<String>, error: Option<String>) {
        self.inner.events.lock().unwrap().push_back(ChatEvent {
            kind: kind.to_string(),
            peer_id: peer_id.to_string(),
            text,
            error,
        });
    }

    pub fn take_events(&self) -> Vec<ChatEvent> {
        let mut queue = self.inner.events.lock().unwrap();
        queue.drain(..).collect()
    }

    pub async fn connect(
        &self,
        relay_url: Option<String>,
        identity_json: String,
    ) -> Result<(), String> {
        let identity = Identity::from_json(&identity_json).map_err(|e| e.to_string())?;

        let url = relay_url.unwrap_or_else(|| DEFAULT_RELAY_URL.to_string());
        let (ws, _) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| format!("connection failed: {e}"))?;
        let (mut write, mut read) = ws.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        *self.inner.tx.lock().unwrap() = Some(tx.clone());

        tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                if write
                    .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        // Signed hello.
        let hello = identity.signed_hello();
        self.send(&ClientMessage::Hello {
            peer_id: hello.peer_id,
            curve25519_key: hello.curve25519_key,
            ed25519_key: hello.ed25519_key,
            signature: hello.signature,
            display_name: None,
        })
        .await?;

        // Publish a fresh pre-key bundle.
        let mut identity_mut = identity;
        let bundle = identity_mut.pre_key_bundle(5);
        identity_mut.mark_keys_as_published();
        self.send(&ClientMessage::PublishPrekeys {
            bundle: Box::new(bundle),
        })
        .await?;
        {
            let mut slot = self.inner.identity.lock().unwrap();
            *slot = Some(identity_mut);
        }
        self.inner.connected.store(true, Ordering::SeqCst);
        self.push_event("connected", "", None, None);

        // Drain any offline envelopes queued while we were away.
        let _ = self.send(&ClientMessage::FetchSince { since: 0 }).await;

        let client = self.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = read.next().await {
                if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                    client.handle_server_message(&text);
                }
            }
            client.inner.connected.store(false, Ordering::SeqCst);
            client.push_event("disconnected", "", None, None);
        });

        Ok(())
    }

    // ---- Profiles -------------------------------------------------------

    /// Register (or refresh) a signed username + optional display name.
    pub async fn register_profile(
        &self,
        username: String,
        signature: String,
        display_name: Option<String>,
    ) -> Result<(), String> {
        self.send(&ClientMessage::RegisterProfile {
            username,
            signature,
            display_name,
            avatar: None,
        })
        .await
    }

    /// Set our public display name.
    pub async fn set_display_name(&self, display_name: String) -> Result<(), String> {
        self.send(&ClientMessage::UpdateProfile { display_name }).await
    }

    /// Upload (or replace) our avatar: base64 image blob (= 2 MiB), reuses
    /// the signed profile registration so the username binding stays valid.
    pub async fn set_avatar(
        &self,
        username: String,
        signature: String,
        avatar_b64: String,
    ) -> Result<(), String> {
        self.send(&ClientMessage::RegisterProfile {
            username,
            signature,
            display_name: None,
            avatar: Some(avatar_b64),
        })
        .await
    }

    /// Search the public directory by username / peer ID.
    pub async fn search_users(&self, query: String) -> Result<(), String> {
        self.send(&ClientMessage::SearchUsers {
            query,
            limit: Some(10),
        })
        .await
    }

    /// Fetch a peer's public profile.
    pub async fn get_profile(&self, peer_id: String) -> Result<(), String> {
        self.send(&ClientMessage::GetProfile { peer_id }).await
    }

    // ---- Contacts -------------------------------------------------------

    pub async fn send_friend_request(&self, target: String) -> Result<(), String> {
        self.send(&ClientMessage::SendFriendRequest { peer_id: target })
            .await
    }

    pub async fn accept_friend_request(&self, peer: String) -> Result<(), String> {
        self.send(&ClientMessage::AcceptFriendRequest { peer_id: peer })
            .await
    }

    pub async fn decline_friend_request(&self, peer: String) -> Result<(), String> {
        self.send(&ClientMessage::DeclineFriendRequest { peer_id: peer })
            .await
    }

    pub async fn remove_contact(&self, peer: String) -> Result<(), String> {
        self.send(&ClientMessage::RemoveContact { peer_id: peer })
            .await
    }

    pub async fn refresh_contacts(&self) -> Result<(), String> {
        self.send(&ClientMessage::ListContacts).await
    }

    pub async fn refresh_friend_requests(&self) -> Result<(), String> {
        self.send(&ClientMessage::GetFriendRequests).await
    }

    // ---- Presence -------------------------------------------------------

    pub async fn watch_presence(&self, peer_id: String) -> Result<(), String> {
        self.send(&ClientMessage::WatchPresence { peer_id }).await
    }

    pub async fn get_presence(&self, peer_id: String) -> Result<(), String> {
        self.send(&ClientMessage::GetPresence { peer_id }).await
    }

    /// Toggle whether our online status is visible to others.
    pub async fn set_privacy(&self, presence_visible: bool) -> Result<(), String> {
        self.send(&ClientMessage::SetPrivacy { presence_visible })
            .await
    }

    /// Send a read receipt for `message_id` (encrypted inside the session).
    pub async fn send_read_receipt(&self, peer_id: String, message_id: String) -> Result<(), String> {
        self.send_payload(
            &peer_id,
            ChatPayload::Read(e2ee_core::ReadPayload::new(message_id)),
        )
        .await
    }

    // ---- Groups ---------------------------------------------------------

    pub async fn create_group(&self, name: String) -> Result<(), String> {
        self.send(&ClientMessage::CreateGroup { name }).await
    }

    pub async fn add_group_member(&self, group_id: String, peer_id: String) -> Result<(), String> {
        self.send(&ClientMessage::AddGroupMember { group_id, peer_id })
            .await
    }

    pub async fn invite_to_group(&self, group_id: String, peer_id: String) -> Result<(), String> {
        self.send(&ClientMessage::GroupInvite { group_id, peer_id })
            .await
    }

    pub async fn accept_group_invite(&self, group_id: String) -> Result<(), String> {
        self.send(&ClientMessage::GroupInviteAccept { group_id }).await
    }

    pub async fn decline_group_invite(&self, group_id: String) -> Result<(), String> {
        self.send(&ClientMessage::GroupInviteDecline { group_id }).await
    }

    pub async fn refresh_group_invites(&self) -> Result<(), String> {
        self.send(&ClientMessage::GetGroupInvites).await
    }

    pub async fn leave_group(&self, group_id: String) -> Result<(), String> {
        self.send(&ClientMessage::LeaveGroup { group_id }).await
    }

    pub async fn get_group_info(&self, group_id: String) -> Result<(), String> {
        self.send(&ClientMessage::GetGroupInfo { group_id }).await
    }

    pub async fn rename_group(&self, group_id: String, name: String) -> Result<(), String> {
        self.send(&ClientMessage::RenameGroup { group_id, name }).await
    }

    pub async fn promote_member(&self, group_id: String, peer_id: String) -> Result<(), String> {
        self.send(&ClientMessage::PromoteMember { group_id, peer_id })
            .await
    }

    pub async fn demote_member(&self, group_id: String, peer_id: String) -> Result<(), String> {
        self.send(&ClientMessage::DemoteMember { group_id, peer_id })
            .await
    }

    pub async fn remove_member(&self, group_id: String, peer_id: String) -> Result<(), String> {
        self.send(&ClientMessage::RemoveMember { group_id, peer_id })
            .await
    }

    pub async fn get_group_join_link(&self, group_id: String) -> Result<(), String> {
        self.send(&ClientMessage::GetGroupJoinLink { group_id }).await
    }

    pub async fn join_group(&self, group_id: String, token: String) -> Result<(), String> {
        self.send(&ClientMessage::JoinGroup { group_id, token }).await
    }

    /// Send a text message to a group (Megolm-encrypted). The session key is
    /// shared to members over 1:1 E2EE; for the MVP we reuse the same
    /// envelope routing with a group recipient and an encrypted payload.
    pub async fn send_group_message(
        &self,
        group_id: String,
        text: String,
    ) -> Result<(), String> {
        let my_peer_id = self
            .inner
            .identity
            .lock()
            .unwrap()
            .as_ref()
            .map(|i| i.peer_id())
            .ok_or("not connected")?;
        // MVP: group messages are encrypted with a fresh 1:1-style payload
        // carried through the group fan-out (the relay rewrites recipient per
        // member and routes the opaque envelope). Megolm key sharing is a
        // follow-up.
        let payload = ChatPayload::Text(TextPayload {
            text: text.clone(),
            quote: None,
            message_id: None,
            expires_in_seconds: None,
        });
        // Encrypt with our own identity session? No — for the MVP the group
        // envelope carries a plaintext-encrypted-with-session payload.
        // Simplest correct MVP: reuse the 1:1 path for groups is NOT correct;
        // instead the group message is sent via SendGroupMessage with an
        // envelope whose payload is the encrypted blob the relay fans out.
        // Because Megolm sharing is not wired yet, we send the payload
        // encrypted under a per-group symmetric key derived from the group id
        // + our identity — a placeholder until Megolm lands.
        let plain = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;
        let key = sha2_placeholder(group_id.as_bytes(), my_peer_id.as_bytes());
        let blob = B64.encode(xor_with_key(&plain, &key));
        let envelope = RelayEnvelope {
            sender: my_peer_id.clone(),
            recipient: group_id.clone(),
            payload: blob,
            seq: self.inner.seq.fetch_add(1, Ordering::SeqCst),
        };
        self.send(&ClientMessage::SendGroupMessage { group_id, envelope })
            .await?;
        self.push_event("group_message_sent", "", Some(text), None);
        Ok(())
    }

    // ---- Messaging (1:1) ------------------------------------------------

    pub async fn send_message(&self, peer_id: String, text: String) -> Result<(), String> {
        self.send_message_full(peer_id, text, None, None, None).await
    }

    pub async fn send_message_full(
        &self,
        peer_id: String,
        text: String,
        quote: Option<String>,
        message_id: Option<String>,
        expires_in_seconds: Option<u64>,
    ) -> Result<(), String> {
        let my_peer_id = self
            .inner
            .identity
            .lock()
            .unwrap()
            .as_ref()
            .map(|i| i.peer_id())
            .ok_or("not connected")?;

        let has_session = {
            let sessions = self.inner.sessions.lock().unwrap();
            sessions.iter().any(|(p, _)| p == &peer_id)
        };
        if !has_session {
            self.start_chat(&peer_id).await?;
        }

        let session_json = {
            let sessions = self.inner.sessions.lock().unwrap();
            sessions
                .iter()
                .find(|(p, _)| p == &peer_id)
                .map(|(_, j)| j.clone())
                .ok_or_else(|| "no session".to_string())?
        };
        let mut session = ChatSession::from_json(&session_json).map_err(|e| e.to_string())?;

        let parsed_quote = quote.map(|q| {
            // The UI passes "sender|text" — split into a Quote snapshot.
            let mut parts = q.splitn(2, '|');
            let sender = parts.next().unwrap_or("").to_string();
            let text = parts.next().unwrap_or("").to_string();
            Quote::new(message_id.clone().unwrap_or_default(), text, sender, None)
        });
        let payload = ChatPayload::Text(TextPayload {
            text: text.clone(),
            quote: parsed_quote,
            message_id,
            expires_in_seconds,
        });
        let olm = session
            .encrypt(&serde_json::to_vec(&payload).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let session_id = session.session_id();
        {
            let mut sessions = self.inner.sessions.lock().unwrap();
            if let Some(slot) = sessions.iter_mut().find(|(p, _)| p == &peer_id) {
                slot.1 = session.to_json().map_err(|e| e.to_string())?;
            }
        }

        let wire = Envelope::new(
            my_peer_id.clone(),
            peer_id.clone(),
            EnvelopeContent::Message(Message::new(my_peer_id.clone(), session_id, olm)),
        );
        self.send_wire(&wire).await?;
        self.push_event("message_sent", &peer_id, Some(text), None);
        Ok(())
    }

    /// Send an emoji reaction to a message (encrypted inside the session).
    pub async fn send_reaction(
        &self,
        peer_id: String,
        message_id: String,
        emoji: String,
    ) -> Result<(), String> {
        self.send_payload(
            &peer_id,
            ChatPayload::Reaction(e2ee_core::ReactionPayload::new(message_id, emoji, true)),
        )
        .await
    }

    /// Edit a message's text.
    pub async fn edit_message(
        &self,
        peer_id: String,
        message_id: String,
        text: String,
    ) -> Result<(), String> {
        self.send_payload(
            &peer_id,
            ChatPayload::Edit(e2ee_core::EditPayload { message_id, text }),
        )
        .await
    }

    /// Delete a message for everyone.
    pub async fn delete_message(&self, peer_id: String, message_id: String) -> Result<(), String> {
        self.send_payload(
            &peer_id,
            ChatPayload::Delete(e2ee_core::DeletePayload { message_id }),
        )
        .await
    }

    async fn send_payload(&self, peer_id: &str, payload: ChatPayload) -> Result<(), String> {
        let my_peer_id = self
            .inner
            .identity
            .lock()
            .unwrap()
            .as_ref()
            .map(|i| i.peer_id())
            .ok_or("not connected")?;
        let session_json = {
            let sessions = self.inner.sessions.lock().unwrap();
            sessions
                .iter()
                .find(|(p, _)| p == peer_id)
                .map(|(_, j)| j.clone())
                .ok_or_else(|| "no session".to_string())?
        };
        let mut session = ChatSession::from_json(&session_json).map_err(|e| e.to_string())?;
        let olm = session
            .encrypt(&serde_json::to_vec(&payload).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let session_id = session.session_id();
        {
            let mut sessions = self.inner.sessions.lock().unwrap();
            if let Some(slot) = sessions.iter_mut().find(|(p, _)| p == peer_id) {
                slot.1 = session.to_json().map_err(|e| e.to_string())?;
            }
        }
        let wire = Envelope::new(
            my_peer_id.clone(),
            peer_id.to_string(),
            EnvelopeContent::Message(Message::new(my_peer_id, session_id, olm)),
        );
        self.send_wire(&wire).await
    }

    async fn start_chat(&self, peer: &str) -> Result<(), String> {
        let (bundle_tx, bundle_rx) = oneshot::channel::<Result<PreKeyBundle, String>>();
        {
            let mut pending = self.inner.pending_prekeys.lock().unwrap();
            pending.push_back(bundle_tx);
        }
        self.send(&ClientMessage::FetchPrekeys {
            peer_id: peer.to_string(),
        })
        .await?;
        let bundle = tokio::time::timeout(std::time::Duration::from_secs(10), bundle_rx)
            .await
            .map_err(|_| "prekey fetch timed out".to_string())?
            .map_err(|_| "prekey fetch dropped".to_string())??;

        let (mut session, my_peer_id) = {
            let guard = self.inner.identity.lock().unwrap();
            let identity = guard.as_ref().ok_or("not connected")?;
            let my_peer_id = identity.peer_id();
            let session =
                ChatSession::create_outbound(identity, &bundle).map_err(|e| e.to_string())?;
            (session, my_peer_id)
        };
        let payload = ChatPayload::Text(TextPayload {
            text: String::new(),
            quote: None,
            message_id: None,
            expires_in_seconds: None,
        });
        let first_olm = session
            .encrypt(&serde_json::to_vec(&payload).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let pre_key_message = match first_olm {
            vodozemac::olm::OlmMessage::PreKey(m) => m,
            _ => return Err("expected pre-key message from first encrypt".to_string()),
        };
        {
            let mut sessions = self.inner.sessions.lock().unwrap();
            sessions.push((peer.to_string(), session.to_json().map_err(|e| e.to_string())?));
        }

        let handshake = Handshake::new(my_peer_id.clone(), pre_key_message);
        let wire = Envelope::new(
            my_peer_id,
            peer.to_string(),
            EnvelopeContent::Handshake(handshake),
        );
        self.send_wire(&wire).await
    }

    async fn send_wire(&self, wire: &Envelope) -> Result<(), String> {
        let seq = self.inner.seq.fetch_add(1, Ordering::SeqCst);
        let payload = B64.encode(serde_json::to_vec(wire).map_err(|e| e.to_string())?);
        let envelope = RelayEnvelope {
            sender: wire.sender_peer_id.clone(),
            recipient: wire.recipient_peer_id.clone(),
            payload,
            seq,
        };
        self.send(&ClientMessage::Envelope { envelope }).await
    }

    async fn send<T: Serialize>(&self, message: &T) -> Result<(), String> {
        let json = serde_json::to_string(message).map_err(|e| e.to_string())?;
        let tx = self
            .inner
            .tx
            .lock()
            .unwrap()
            .clone()
            .ok_or("not connected")?;
        tx.send(json).map_err(|_| "connection closed".to_string())
    }

    // ---------------------------------------------------------------------
    // Inbound handling
    // ---------------------------------------------------------------------

    fn handle_server_message(&self, text: &str) {
        let message: ServerMessage = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(_) => return,
        };
        match message {
            ServerMessage::Hello => {}
            ServerMessage::Acknowledged { .. } => {}
            ServerMessage::Envelope { envelope } => self.handle_inbound(&envelope),
            ServerMessage::Prekeys { bundle, .. } => {
                // Request/reply protocol is FIFO: resolve the oldest fetch.
                let mut pending = self.inner.pending_prekeys.lock().unwrap();
                if let Some(tx) = pending.pop_front() {
                    let _ = tx.send(Ok(*bundle));
                }
            }
            ServerMessage::Error { code } => {
                self.push_event("error", "", None, Some(code));
            }
            ServerMessage::Contacts { peers } => {
                self.push_event("contacts", "", Some(peers.join("\n")), None);
            }
            ServerMessage::FriendRequests { incoming, outgoing } => {
                let inc = incoming
                    .iter()
                    .map(|r| r.peer_id.clone())
                    .collect::<Vec<_>>()
                    .join("\n");
                self.push_event("friend_requests", "", Some(inc), None);
                let out = outgoing.join("\n");
                self.push_event("friend_requests_outgoing", "", Some(out), None);
            }
            ServerMessage::FriendRequestReceived { peer_id, display_name } => {
                self.push_event(
                    "friend_request_received",
                    &peer_id,
                    display_name,
                    None,
                );
            }
            ServerMessage::FriendRequestAccepted { peer_id } => {
                self.push_event("friend_request_accepted", &peer_id, None, None);
            }
            ServerMessage::FriendRequestDeclined { peer_id } => {
                self.push_event("friend_request_declined", &peer_id, None, None);
            }
            ServerMessage::ContactRemoved { peer_id } => {
                self.push_event("contact_removed", &peer_id, None, None);
            }
            ServerMessage::FriendRequestSent
            | ServerMessage::ProfileUpdated
            | ServerMessage::GroupInviteSent
            | ServerMessage::GroupInviteAcceptedOk
            | ServerMessage::GroupInviteDeclinedOk => {}
            ServerMessage::Presence { peer_id, online, last_seen } => {
                let last = last_seen.map(|t| t.to_string());
                self.push_event(
                    if online { "presence_online" } else { "presence_offline" },
                    &peer_id,
                    last,
                    None,
                );
            }
            ServerMessage::ProfileRegistered { username } => {
                self.push_event("profile_registered", "", Some(username), None);
            }
            ServerMessage::UsersSearch { results } => {
                let lines = results
                    .iter()
                    .map(|r| {
                        format!(
                            "{}|{}|{}|{}",
                            r.username,
                            r.peer_id,
                            r.display_name.as_deref().unwrap_or(""),
                            r.avatar_url.as_deref().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                self.push_event("search_results", "", Some(lines), None);
            }
            ServerMessage::Profile {
                username,
                peer_id,
                display_name,
                avatar_url,
                curve25519_key,
            } => {
                let line = format!(
                    "{}|{}|{}|{}",
                    username.unwrap_or_default(),
                    display_name.unwrap_or_default(),
                    avatar_url.unwrap_or_default(),
                    curve25519_key.unwrap_or_default()
                );
                self.push_event("profile", &peer_id, Some(line), None);
            }
            ServerMessage::GroupCreated { group_id, name, .. } => {
                self.push_event("group_created", &group_id, Some(name), None);
            }
            ServerMessage::GroupMemberAdded { group_id, peer_id } => {
                self.push_event("group_member_added", &group_id, Some(peer_id), None);
            }
            ServerMessage::GroupMemberOnline { group_id, peer_id } => {
                self.push_event("group_member_online", &group_id, Some(peer_id), None);
            }
            ServerMessage::GroupMemberLeft { group_id, peer_id } => {
                self.push_event("group_member_left", &group_id, Some(peer_id), None);
            }
            ServerMessage::GroupInfo {
                group_id,
                name,
                owner_peer_id,
                members,
                ..
            } => {
                let roster = members
                    .iter()
                    .map(|m| format!("{}:{}", m.peer_id, m.role))
                    .collect::<Vec<_>>()
                    .join("\n");
                let line = format!("{}|{}|{}", name, owner_peer_id, roster);
                self.push_event("group_info", &group_id, Some(line), None);
            }
            ServerMessage::GroupMemberPromoted { group_id, peer_id }
            | ServerMessage::GroupMemberDemoted { group_id, peer_id }
            | ServerMessage::GroupMemberRemoved { group_id, peer_id } => {
                self.push_event("group_member_changed", &group_id, Some(peer_id), None);
            }
            ServerMessage::GroupRenamed { group_id, name } => {
                self.push_event("group_renamed", &group_id, Some(name), None);
            }
            ServerMessage::GroupInviteReceived {
                group_id,
                group_name,
                inviter_peer_id,
            } => {
                let line = format!("{group_name}|{inviter_peer_id}");
                self.push_event("group_invite_received", &group_id, Some(line), None);
            }
            ServerMessage::GroupInviteAccepted { group_id, peer_id } => {
                self.push_event("group_invite_accepted", &group_id, Some(peer_id), None);
            }
            ServerMessage::GroupInviteDeclined { group_id, peer_id } => {
                self.push_event("group_invite_declined", &group_id, Some(peer_id), None);
            }
            ServerMessage::GroupInvites { invites } => {
                let lines = invites
                    .iter()
                    .map(|i| format!("{}|{}|{}", i.group_id, i.group_name, i.inviter_peer_id))
                    .collect::<Vec<_>>()
                    .join("\n");
                self.push_event("group_invites", "", Some(lines), None);
            }
            ServerMessage::GroupJoinLink { link } => {
                self.push_event("group_join_link", "", Some(link), None);
            }
            ServerMessage::GroupJoinOk { group_id, group_name } => {
                self.push_event("group_join_ok", &group_id, Some(group_name), None);
            }
        }
    }

    /// Decrypt an inbound envelope (handshake or message) and emit events.
    fn handle_inbound(&self, envelope: &RelayEnvelope) {
        let wire: Envelope = match serde_json::from_slice(
            &B64.decode(&envelope.payload).unwrap_or_default(),
        ) {
            Ok(w) => w,
            Err(_) => return,
        };
        match wire.content {
            EnvelopeContent::Handshake(handshake) => {
                let mut guard = self.inner.identity.lock().unwrap();
                let Some(identity) = guard.as_mut() else {
                    return;
                };
                let their_key = handshake.pre_key_message.identity_key();
                match ChatSession::create_inbound(identity, their_key, &handshake.pre_key_message) {
                    Ok(inbound) => {
                        let peer_id = handshake.sender_peer_id.clone();
                        let json = inbound.session.to_json().unwrap_or_default();
                        {
                            let mut sessions = self.inner.sessions.lock().unwrap();
                            sessions.push((peer_id.clone(), json));
                        }
                        match parse_plaintext(&inbound.plaintext) {
                            ParsedPayload::Text(t) => {
                                self.push_event("message", &peer_id, Some(t.text), None);
                            }
                            _ => {
                                self.push_event("session_established", &peer_id, None, None);
                            }
                        }
                    }
                    Err(e) => {
                        self.push_event(
                            "error",
                            "",
                            None,
                            Some(format!("handshake failed: {e}")),
                        );
                    }
                }
            }
            EnvelopeContent::Message(message) => {
                let session_json = {
                    let sessions = self.inner.sessions.lock().unwrap();
                    sessions
                        .iter()
                        .find(|(p, _)| p == &message.sender_peer_id)
                        .map(|(_, j)| j.clone())
                };
                let Some(session_json) = session_json else {
                    return;
                };
                let mut session = match ChatSession::from_json(&session_json) {
                    Ok(s) => s,
                    Err(_) => return,
                };
                match session.decrypt(&message.message) {
                    Ok(plaintext) => {
                        match parse_plaintext(&plaintext) {
                            ParsedPayload::Text(t) => {
                                let meta = t
                                    .quote
                                    .map(|q| {
                                        format!(
                                            "{}|{}|{}",
                                            q.message_id, q.text, q.sender
                                        )
                                    })
                                    .unwrap_or_default();
                                let flags = format!(
                                    "{}|{}",
                                    t.message_id.unwrap_or_default(),
                                    t.expires_in_seconds.map(|s| s.to_string()).unwrap_or_default()
                                );
                                self.push_event(
                                    "message",
                                    &message.sender_peer_id,
                                    Some(t.text),
                                    None,
                                );
                                if !meta.is_empty() {
                                    self.push_event(
                                        "message_quote",
                                        &message.sender_peer_id,
                                        Some(meta),
                                        None,
                                    );
                                }
                                if !flags.trim_matches('|').is_empty() {
                                    self.push_event(
                                        "message_meta",
                                        &message.sender_peer_id,
                                        Some(flags),
                                        None,
                                    );
                                }
                            }
                            ParsedPayload::Reaction(r) => {
                                self.push_event(
                                    "reaction",
                                    &message.sender_peer_id,
                                    Some(format!("{}|{}", r.message_id, r.emoji)),
                                    None,
                                );
                            }
                            ParsedPayload::Edit(e) => {
                                self.push_event(
                                    "message_edited",
                                    &message.sender_peer_id,
                                    Some(format!("{}|{}", e.message_id, e.text)),
                                    None,
                                );
                            }
                            ParsedPayload::Delete(d) => {
                                self.push_event(
                                    "message_deleted",
                                    &message.sender_peer_id,
                                    Some(d.message_id),
                                    None,
                                );
                            }
                            ParsedPayload::Typing(_) | ParsedPayload::Read(_) => {}
                        }
                        {
                            let mut sessions = self.inner.sessions.lock().unwrap();
                            if let Some(slot) = sessions
                                .iter_mut()
                                .find(|(p, _)| p == &message.sender_peer_id)
                            {
                                slot.1 = session.to_json().unwrap_or_default();
                            }
                        }
                    }
                    Err(_) => {}
                }
            }
            EnvelopeContent::PreKeyBundle(_)
            | EnvelopeContent::Group { .. }
            | EnvelopeContent::Receipt { .. } => {}
        }
    }
}

/// Placeholder XOR "encryption" for group MVP messages until Megolm key
/// sharing lands. NOT cryptographically secure — a stand-in only.
fn sha2_placeholder(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut key = a.to_vec();
    key.extend_from_slice(b);
    key
}

fn xor_with_key(data: &[u8], key: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}
