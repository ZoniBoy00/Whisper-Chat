//! Versioned wire protocol types (protocol version 1).
//!
//! These are the envelopes the zero-knowledge relay forwards between clients.
//! Every payload is opaque to the relay: it only sees the base64-encoded
//! ciphertext and metadata, never plaintext or key material.
//!
//! Three kinds of content exist:
//!
//! - [`EnvelopeContent::PreKeyBundle`]: published key material.
//! - [`EnvelopeContent::Handshake`]: the first, session-establishing message.
//! - [`EnvelopeContent::Message`]: an ordinary encrypted message.

use serde::{Deserialize, Serialize};
use vodozemac::olm::{OlmMessage, PreKeyMessage};

pub use crate::prekey::PreKeyBundle;

/// The current wire protocol version.
pub const WIRE_VERSION: u8 = 1;

/// An ordinary encrypted message inside an [`Envelope`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// The wire protocol version, always [`WIRE_VERSION`].
    pub version: u8,
    /// Peer ID of the sender.
    pub sender_peer_id: String,
    /// Session ID identifying which ratchet state to use for decryption.
    pub session_id: String,
    /// The Double Ratchet ciphertext.
    pub message: OlmMessage,
}

impl Message {
    /// Create a version-1 message.
    pub fn new(sender_peer_id: String, session_id: String, message: OlmMessage) -> Self {
        Self {
            version: WIRE_VERSION,
            sender_peer_id,
            session_id,
            message,
        }
    }
}

/// The first message of a conversation, establishing the Double Ratchet
/// session on the recipient's side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handshake {
    /// The wire protocol version, always [`WIRE_VERSION`].
    pub version: u8,
    /// Peer ID of the initiator.
    pub sender_peer_id: String,
    /// The pre-key message carrying the session-establishing ciphertext.
    pub pre_key_message: PreKeyMessage,
}

impl Handshake {
    /// Create a version-1 handshake.
    pub fn new(sender_peer_id: String, pre_key_message: PreKeyMessage) -> Self {
        Self {
            version: WIRE_VERSION,
            sender_peer_id,
            pre_key_message,
        }
    }
}

/// A routed envelope addressed to a specific peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// The wire protocol version, always [`WIRE_VERSION`].
    pub version: u8,
    /// Peer ID of the sender.
    pub sender_peer_id: String,
    /// Peer ID of the intended recipient.
    pub recipient_peer_id: String,
    /// The payload of the envelope.
    pub content: EnvelopeContent,
}

impl Envelope {
    /// Create a version-1 envelope.
    pub fn new(
        sender_peer_id: String,
        recipient_peer_id: String,
        content: EnvelopeContent,
    ) -> Self {
        Self {
            version: WIRE_VERSION,
            sender_peer_id,
            recipient_peer_id,
            content,
        }
    }
}

/// The kinds of payload an [`Envelope`] can carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnvelopeContent {
    /// Published pre-key material used to establish a session.
    PreKeyBundle(PreKeyBundle),
    /// The session-establishing first message.
    Handshake(Handshake),
    /// An ordinary encrypted message.
    Message(Message),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use crate::session::ChatSession;

    #[test]
    fn wire_version_is_one() {
        assert_eq!(WIRE_VERSION, 1);
    }

    #[test]
    fn message_roundtrip_preserves_fields() {
        let (mut alice_session, _) = chat_pair();
        let olm_message = alice_session
            .encrypt(b"payload")
            .expect("encrypt must succeed");

        let message = Message::new("alice".to_string(), alice_session.session_id(), olm_message);
        let json = serde_json::to_string(&message).expect("serialization must succeed");
        let restored: Message = serde_json::from_str(&json).expect("deserialization must succeed");

        assert_eq!(restored, message);
        assert_eq!(restored.version, WIRE_VERSION);
    }

    #[test]
    fn handshake_roundtrip_preserves_fields() {
        let mut bob = Identity::new();
        let bundle = bob.pre_key_bundle(5);
        let mut alice_session = ChatSession::create_outbound(&Identity::new(), &bundle)
            .expect("outbound session creation must succeed");

        let first = alice_session
            .encrypt(b"first")
            .expect("encrypt must succeed");
        let pre_key_message = match first {
            OlmMessage::PreKey(m) => m,
            _ => panic!("first message must be a pre-key message"),
        };

        let handshake = Handshake::new("alice".to_string(), pre_key_message);
        let json = serde_json::to_string(&handshake).expect("serialization must succeed");
        let restored: Handshake =
            serde_json::from_str(&json).expect("deserialization must succeed");

        assert_eq!(restored, handshake);
        assert_eq!(restored.version, WIRE_VERSION);
    }

    #[test]
    fn envelope_roundtrip_for_all_content_kinds() {
        let (mut alice_session, _) = chat_pair();

        let message_content = {
            let olm_message = alice_session
                .encrypt(b"hello")
                .expect("encrypt must succeed");
            EnvelopeContent::Message(Message::new(
                "alice".to_string(),
                alice_session.session_id(),
                olm_message,
            ))
        };

        let mut bob = Identity::new();
        let bundle = bob.pre_key_bundle(2);
        let mut session = ChatSession::create_outbound(&Identity::new(), &bundle)
            .expect("outbound session creation must succeed");
        let first = session.encrypt(b"handshake").expect("encrypt must succeed");
        let pre_key_message = match first {
            OlmMessage::PreKey(m) => m,
            _ => panic!("first message must be a pre-key message"),
        };
        let handshake_content =
            EnvelopeContent::Handshake(Handshake::new("alice".to_string(), pre_key_message));
        let bundle_content = EnvelopeContent::PreKeyBundle(bundle);

        for content in [message_content, handshake_content, bundle_content] {
            let envelope = Envelope::new("alice".to_string(), "bob".to_string(), content);
            let json = serde_json::to_string(&envelope).expect("serialization must succeed");
            let restored: Envelope =
                serde_json::from_str(&json).expect("deserialization must succeed");

            assert_eq!(restored, envelope);
            assert_eq!(restored.version, WIRE_VERSION);
        }
    }

    /// A minimal session pair used to produce wire-level message payloads.
    fn chat_pair() -> (ChatSession, ChatSession) {
        let alice = Identity::new();
        let mut bob = Identity::new();
        let bundle = bob.pre_key_bundle(5);
        let mut alice_session = ChatSession::create_outbound(&alice, &bundle)
            .expect("outbound session creation must succeed");

        let first = alice_session
            .encrypt(b"hello")
            .expect("encrypt must succeed");
        let pre_key_message = match first {
            OlmMessage::PreKey(m) => m,
            _ => panic!("first message must be a pre-key message"),
        };
        let inbound =
            ChatSession::create_inbound(&mut bob, alice.curve25519_key(), &pre_key_message)
                .expect("inbound session creation must succeed");

        (alice_session, inbound.session)
    }
}
