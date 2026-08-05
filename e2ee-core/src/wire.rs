//! Versioned wire protocol types (protocol version 1).
//!
//! These are the envelopes the zero-knowledge relay forwards between clients.
//! Every payload is opaque to the relay: it only sees the base64-encoded
//! ciphertext and metadata, never plaintext or key material.
//!
//! Five kinds of content exist:
//!
//! - [`EnvelopeContent::PreKeyBundle`]: published key material.
//! - [`EnvelopeContent::Handshake`]: the first, session-establishing message.
//! - [`EnvelopeContent::Message`]: an ordinary encrypted message.
//! - [`EnvelopeContent::Group`]: a Megolm group ciphertext (the group_id
//!   selects the inbound session to decrypt it with).
//! - [`EnvelopeContent::Receipt`]: a lightweight end-to-end control signal
//!   (read receipts and typing indicators).

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

/// The kind of end-to-end control signal carried in an
/// [`EnvelopeContent::Receipt`].
///
/// Read receipts and typing indicators are per-message client signals. They
/// are not message payloads themselves: like [`Message`] they travel as
/// small encrypted envelopes inside the Double Ratchet session, so the
/// zero-knowledge relay only ever sees their ciphertext.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptKind {
    /// The recipient has read the sender's messages.
    Read,
    /// The recipient is currently composing a reply.
    Typing,
    /// The recipient has stopped composing.
    TypingStopped,
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
    /// A Megolm-encrypted group message.
    ///
    /// `ciphertext` is the base64 Megolm ciphertext produced by an
    /// [`crate::OutboundGroup`]; `group_id` is the relay-assigned group
    /// identifier that selects the member's [`crate::InboundGroup`] session.
    /// The relay treats the whole payload as opaque and only routes it.
    Group {
        group_id: String,
        ciphertext: String,
    },
    /// A lightweight end-to-end control signal: read receipts and typing
    /// indicators.
    ///
    /// The serde field is renamed to `receipt` because the enum's internal
    /// tag (`kind`) already occupies the key `kind` in the JSON object; a
    /// plain `kind` field would collide with the tag and break round-tripping.
    Receipt {
        #[serde(rename = "receipt")]
        kind: ReceiptKind,
    },
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
        let receipt_content = EnvelopeContent::Receipt {
            kind: ReceiptKind::Read,
        };
        let group_content = EnvelopeContent::Group {
            group_id: "group-1".to_string(),
            ciphertext: "some-megolm-ciphertext".to_string(),
        };

        for content in [
            message_content,
            handshake_content,
            bundle_content,
            receipt_content,
            group_content,
        ] {
            let envelope = Envelope::new("alice".to_string(), "bob".to_string(), content);
            let json = serde_json::to_string(&envelope).expect("serialization must succeed");
            let restored: Envelope =
                serde_json::from_str(&json).expect("deserialization must succeed");

            assert_eq!(restored, envelope);
            assert_eq!(restored.version, WIRE_VERSION);
        }
    }

    #[test]
    fn receipt_kind_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&ReceiptKind::Read).expect("serialization must succeed"),
            r#""read""#
        );
        assert_eq!(
            serde_json::to_string(&ReceiptKind::Typing).expect("serialization must succeed"),
            r#""typing""#
        );
        assert_eq!(
            serde_json::to_string(&ReceiptKind::TypingStopped).expect("serialization must succeed"),
            r#""typing_stopped""#
        );
    }

    #[test]
    fn receipt_kind_roundtrips_through_json() {
        for kind in [
            ReceiptKind::Read,
            ReceiptKind::Typing,
            ReceiptKind::TypingStopped,
        ] {
            let json = serde_json::to_string(&kind).expect("serialization must succeed");
            let restored: ReceiptKind =
                serde_json::from_str(&json).expect("deserialization must succeed");
            assert_eq!(restored, kind);
        }
    }

    #[test]
    fn receipt_content_roundtrips_inside_envelope() {
        for kind in [
            ReceiptKind::Read,
            ReceiptKind::Typing,
            ReceiptKind::TypingStopped,
        ] {
            let envelope = Envelope::new(
                "alice".to_string(),
                "bob".to_string(),
                EnvelopeContent::Receipt { kind },
            );
            let json = serde_json::to_string(&envelope).expect("serialization must succeed");
            let restored: Envelope =
                serde_json::from_str(&json).expect("deserialization must succeed");

            assert_eq!(restored, envelope);
            assert_eq!(restored.version, WIRE_VERSION);
        }
    }

    #[test]
    fn receipt_wire_format_avoids_kind_tag_collision() {
        let content = EnvelopeContent::Receipt {
            kind: ReceiptKind::TypingStopped,
        };
        let json = serde_json::to_string(&content).expect("serialization must succeed");
        // The `kind` tag names the variant; the receipt kind lives under a
        // distinct `receipt` key so the JSON object stays unambiguous.
        assert_eq!(json, r#"{"kind":"receipt","receipt":"typing_stopped"}"#);
    }

    #[test]
    fn group_content_serializes_with_expected_wire_format() {
        let content = EnvelopeContent::Group {
            group_id: "group-42".to_string(),
            ciphertext: "c2VjcmV0".to_string(),
        };
        let json = serde_json::to_string(&content).expect("serialization must succeed");
        assert_eq!(
            json,
            r#"{"kind":"group","group_id":"group-42","ciphertext":"c2VjcmV0"}"#
        );
    }

    #[test]
    fn group_content_roundtrips_inside_envelope() {
        let content = EnvelopeContent::Group {
            group_id: "group-7".to_string(),
            ciphertext: "aGFja2Vk".to_string(),
        };
        let envelope = Envelope::new("alice".to_string(), "bob".to_string(), content);
        let json = serde_json::to_string(&envelope).expect("serialization must succeed");
        let restored: Envelope = serde_json::from_str(&json).expect("deserialization must succeed");

        assert_eq!(restored, envelope);
        assert_eq!(restored.version, WIRE_VERSION);
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
