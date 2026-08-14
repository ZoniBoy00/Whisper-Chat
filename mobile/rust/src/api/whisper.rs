//! Whisper mobile core — E2EE chat over the same zero-knowledge relay the
//! desktop client uses.
//!
//! Everything crypto lives in `e2ee-core` (identity, X3DH + Double Ratchet);
//! this crate adds the wire protocol, the WebSocket connection and a small
//! event queue the Flutter UI polls. The relay address is hardcoded here on
//! purpose (mirroring the desktop client) — it can only change with a build.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use e2ee_core::{
    parse_plaintext, ChatPayload, ChatSession, Envelope, EnvelopeContent, Handshake, Identity,
    Message, ParsedPayload, PreKeyBundle, TextPayload,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

/// Hardcoded relay endpoint — mirror of the desktop `DEFAULT_RELAY_URL`.
pub const DEFAULT_RELAY_URL: &str = "wss://whisper-test.homelab.cfd/ws";

// ---------------------------------------------------------------------------
// Wire protocol (client -> relay)
// ---------------------------------------------------------------------------

/// Messages the client sends to the relay. Field names and shapes must match
/// `whisper-relay` exactly (serde rename_all = snake_case).
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
    GetFriendRequests,
    ListContacts,
    WatchPresence {
        peer_id: String,
    },
}

/// The routing envelope the relay understands (payload = base64 JSON).
#[derive(Debug, Serialize, Deserialize)]
struct RelayEnvelope {
    sender: String,
    recipient: String,
    payload: String,
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
        seq: u64,
    },
    PreKeyBundle {
        peer_id: String,
        bundle: PreKeyBundle,
    },
    Error {
        code: String,
    },
    Contacts {
        peers: Vec<String>,
    },
    FriendRequests {
        requests: Vec<String>,
    },
    Presence {
        peer_id: String,
        online: bool,
        last_seen: Option<u64>,
    },
}

// ---------------------------------------------------------------------------
// FFI-facing types
// ---------------------------------------------------------------------------

/// A freshly created (or loaded) identity: its peer ID plus the JSON blob to
/// persist on the device.
#[derive(Debug, Clone)]
pub struct IdentityInfo {
    pub peer_id: String,
    pub json: String,
}

/// One event emitted by the relay loop, drained by the UI via `take_events`.
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

/// Validate a peer ID shape (24 lowercase hex chars).
pub fn is_valid_peer_id(peer_id: &str) -> bool {
    peer_id.len() == 24 && peer_id.bytes().all(|b| b.is_ascii_hexdigit())
}

// ---------------------------------------------------------------------------
// The relay client
// ---------------------------------------------------------------------------

/// Internal client state. The WebSocket lives on a tokio task; commands are
/// queued through an outbound channel and replies/events land in a queue the
/// UI drains with `take_events`.
#[derive(Clone)]
pub struct WhisperClient {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    tx: Mutex<Option<mpsc::UnboundedSender<String>>>,
    events: Mutex<VecDeque<ChatEvent>>,
    identity: Mutex<Option<Identity>>,
    sessions: Mutex<Vec<(String, String)>>, // (peer_id, session_json)
    pending_prekeys: Mutex<HashMap<String, oneshot::Sender<Result<PreKeyBundle, String>>>>,
    seq: AtomicU64,
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
                pending_prekeys: Mutex::new(HashMap::new()),
                seq: AtomicU64::new(0),
            }),
        }
    }

    fn push_event(&self, kind: &str, peer_id: &str, text: Option<String>, error: Option<String>) {
        self.inner.events.lock().unwrap().push_back(ChatEvent {
            kind: kind.to_string(),
            peer_id: peer_id.to_string(),
            text,
            error,
        });
    }

    /// Drain all pending events (polled by the UI, e.g. once per second).
    pub fn take_events(&self) -> Vec<ChatEvent> {
        let mut queue = self.inner.events.lock().unwrap();
        queue.drain(..).collect()
    }

    /// Connect to the relay: open the WebSocket, send the signed hello and
    /// publish our pre-key bundle so other peers can start sessions.
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

        // Pump the outbound queue to the socket.
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

        // Publish a fresh pre-key bundle so we are reachable.
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

        self.push_event("connected", "", None, None);

        // Inbound loop: decrypt envelopes and answer protocol requests.
        let client = self.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = read.next().await {
                if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                    client.handle_server_message(&text);
                }
            }
            client.push_event("disconnected", "", None, None);
        });

        Ok(())
    }

    /// Send a friend request to `target`.
    pub async fn send_friend_request(&self, target: String) -> Result<(), String> {
        self.send(&ClientMessage::SendFriendRequest { peer_id: target })
            .await
    }

    /// Accept a pending friend request from `peer`.
    pub async fn accept_friend_request(&self, peer: String) -> Result<(), String> {
        self.send(&ClientMessage::AcceptFriendRequest { peer_id: peer })
            .await
    }

    /// Ask the relay for our accepted contacts (as peer IDs).
    pub async fn refresh_contacts(&self) -> Result<(), String> {
        self.send(&ClientMessage::ListContacts).await
    }

    /// Ask the relay for pending friend requests.
    pub async fn refresh_friend_requests(&self) -> Result<(), String> {
        self.send(&ClientMessage::GetFriendRequests).await
    }

    /// Subscribe to online/offline pushes for `peer_id`.
    pub async fn watch_presence(&self, peer_id: String) -> Result<(), String> {
        self.send(&ClientMessage::WatchPresence { peer_id }).await
    }

    /// Send a text message to `peer_id`, establishing a Double Ratchet
    /// session with a handshake on the first message.
    pub async fn send_message(&self, peer_id: String, text: String) -> Result<(), String> {
        let my_peer_id = self
            .inner
            .identity
            .lock()
            .unwrap()
            .as_ref()
            .map(|i| i.peer_id())
            .ok_or("not connected")?;

        // Lazy session heal: no session yet -> fetch prekeys + handshake.
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

        let payload = ChatPayload::Text(TextPayload {
            text: text.clone(),
            quote: None,
            message_id: None,
            expires_in_seconds: None,
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

    /// Establish a 1:1 session with `peer` (fetch prekeys, X3DH, send the
    /// handshake envelope).
    async fn start_chat(&self, peer: &str) -> Result<(), String> {
        // Fetch the peer's pre-key bundle via a pending-request mechanism.
        let (bundle_tx, bundle_rx) = oneshot::channel::<Result<PreKeyBundle, String>>();
        {
            let mut pending = self.inner.pending_prekeys.lock().unwrap();
            pending.insert(peer.to_string(), bundle_tx);
        }
        self.send(&ClientMessage::FetchPrekeys {
            peer_id: peer.to_string(),
        })
        .await?;
        let bundle = tokio::time::timeout(std::time::Duration::from_secs(10), bundle_rx)
            .await
            .map_err(|_| "prekey fetch timed out".to_string())?
            .map_err(|_| "prekey fetch dropped".to_string())??;

        // X3DH: create the outbound session. The session's first encrypt
        // produces the pre-key message that carries the handshake. The mutex
        // guard stays alive for the (synchronous) create_outbound call.
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

    fn handle_server_message(&self, text: &str) {
        let message: ServerMessage = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(_) => return,
        };
        match message {
            ServerMessage::Hello => {}
            ServerMessage::Acknowledged { .. } => {}
            ServerMessage::Envelope { envelope } => self.handle_inbound(&envelope),
            ServerMessage::PreKeyBundle { peer_id, bundle } => {
                let mut pending = self.inner.pending_prekeys.lock().unwrap();
                if let Some(tx) = pending.remove(&peer_id) {
                    let _ = tx.send(Ok(bundle));
                }
            }
            ServerMessage::Error { code } => {
                self.push_event("error", "", None, Some(code));
            }
            ServerMessage::Contacts { peers } => {
                self.push_event("contacts", "", Some(peers.join("\n")), None);
            }
            ServerMessage::FriendRequests { requests } => {
                self.push_event("friend_requests", "", Some(requests.join("\n")), None);
            }
            ServerMessage::Presence { peer_id, online, .. } => {
                self.push_event(
                    if online { "peer_online" } else { "peer_offline" },
                    &peer_id,
                    None,
                    None,
                );
            }
        }
    }

    /// Decrypt an inbound envelope: handshake -> create inbound session,
    /// message -> decrypt + emit the plaintext text.
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
                        // The handshake carries the initiator's first payload.
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
                                self.push_event(
                                    "message",
                                    &message.sender_peer_id,
                                    Some(t.text),
                                    None,
                                );
                            }
                            _ => {}
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
            EnvelopeContent::PreKeyBundle(_) => {}
            EnvelopeContent::Group { .. } | EnvelopeContent::Receipt { .. } => {}
        }
    }
}
