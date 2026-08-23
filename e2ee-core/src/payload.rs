//! End-to-end plaintext payloads (protocol version 1, extension).
//!
//! Ordinary chat messages are encrypted as raw plaintext bytes by the Double
//! Ratchet / Megolm session, so the relay never sees them. Historically the
//! plaintext of a message was simply the text itself. To carry richer content
//! (quoted replies, emoji reactions) without breaking that model, new payload
//! kinds travel as a small tagged JSON envelope — [`ChatPayload`] — while
//! legacy raw text keeps working untouched.
//!
//! # Backwards compatibility
//!
//! [`parse_plaintext`] is the single entry point for inbound plaintext. It
//! returns [`ParsedPayload::Text`] for both modern text envelopes *and*
//! legacy raw text, so a client that has not been upgraded yet continues to
//! render old and new messages identically. Unknown JSON shapes (for example
//! a future payload kind) fall back to raw text too, so a newer peer's
//! messages degrade gracefully instead of breaking the inbox.
//!
//! # Zero-knowledge note
//!
//! These payloads are always encrypted *before* they leave the device: a
//! `ChatPayload` is serialized with serde_json and then fed to the session
//! cipher like any other plaintext. The relay therefore only ever sees the
//! ciphertext of a [`crate::wire::EnvelopeContent::Message`].

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};

/// The decrypted plaintext of an inbound chat message.
///
/// Mirrors [`ChatPayload`] but additionally maps legacy raw text onto
/// [`TextPayload`] so callers have a single, uniform shape to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedPayload {
    /// An ordinary (possibly quoting) text message.
    Text(TextPayload),
    /// An emoji reaction attached to another message.
    Reaction(ReactionPayload),
    /// A group typing indicator.
    Typing(TypingPayload),
    /// A group read receipt.
    Read(ReadPayload),
    /// A request to edit an earlier message's text.
    Edit(EditPayload),
    /// A request to delete an earlier message on every recipient's device.
    Delete(DeletePayload),
    /// Encrypted media metadata and the key needed to fetch and decrypt it.
    Media(MediaPayload),
}

/// The tagged wire form of a modern payload. The `kind` field selects the
/// payload shape; unknown `kind` values fail deserialization and fall back to
/// raw text in [`parse_plaintext`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatPayload {
    /// A text message, optionally quoting an earlier message.
    Text(TextPayload),
    /// An emoji reaction to an earlier message.
    Reaction(ReactionPayload),
    /// A group typing indicator.
    Typing(TypingPayload),
    /// A group read receipt.
    Read(ReadPayload),
    /// Edit an earlier message's text on every recipient's device.
    Edit(EditPayload),
    /// Delete an earlier message on every recipient's device.
    Delete(DeletePayload),
    /// Metadata for an encrypted media blob stored by its hash.
    Media(MediaPayload),
}

/// Metadata carried inside the E2EE message for one encrypted media blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MediaPayload {
    pub message_id: String,
    pub hash: String,
    pub key: String,
    pub mime: String,
    pub size: u64,
    /// Optional separately encrypted preview blob.
    #[serde(default, rename = "thumb", skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<MediaThumbnail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

#[derive(Deserialize)]
struct RawMediaPayload {
    message_id: String,
    hash: String,
    key: String,
    mime: String,
    size: u64,
    #[serde(default, rename = "thumb")]
    thumbnail: Option<MediaThumbnail>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    duration_ms: Option<u64>,
}

impl<'de> Deserialize<'de> for MediaPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawMediaPayload::deserialize(deserializer)?;
        Self::new(
            raw.message_id,
            raw.hash,
            raw.key,
            raw.mime,
            raw.size,
            raw.name,
            raw.duration_ms,
        )
        .and_then(|mut payload| {
            payload.thumbnail = raw.thumbnail;
            payload.validate().map(|()| payload)
        })
        .map_err(serde::de::Error::custom)
    }
}

/// Metadata and key for a separately encrypted thumbnail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaThumbnail {
    pub hash: String,
    pub key: String,
    pub mime: String,
    pub size: u64,
}

impl MediaThumbnail {
    /// Decode the thumbnail's 256-bit key.
    pub fn key_bytes(&self) -> Option<[u8; 32]> {
        decode_key(&self.key).and_then(|bytes| bytes.try_into().ok())
    }
}

impl MediaPayload {
    /// Construct metadata after checking all fields that are security-relevant
    /// or would otherwise make the wire representation ambiguous.
    pub fn new(
        message_id: impl Into<String>,
        hash: impl Into<String>,
        key: impl Into<String>,
        mime: impl Into<String>,
        size: u64,
        name: Option<String>,
        duration_ms: Option<u64>,
    ) -> Result<Self, String> {
        let payload = Self {
            message_id: message_id.into(),
            hash: hash.into(),
            key: key.into(),
            mime: mime.into(),
            size,
            thumbnail: None,
            name,
            duration_ms,
        };
        payload.validate()?;
        Ok(payload)
    }

    /// Attach a separately encrypted thumbnail to this media payload.
    pub fn with_thumbnail(mut self, thumbnail: MediaThumbnail) -> Result<Self, String> {
        self.thumbnail = Some(thumbnail);
        self.validate()?;
        Ok(self)
    }

    /// Attach an optional thumbnail without forcing callers to branch.
    pub fn with_thumbnail_opt(mut self, thumbnail: Option<MediaThumbnail>) -> Result<Self, String> {
        self.thumbnail = thumbnail;
        self.validate()?;
        Ok(self)
    }

    /// Validate metadata received from an untrusted peer.
    pub fn validate(&self) -> Result<(), String> {
        if self.message_id.is_empty() || self.message_id.len() > 256 {
            return Err("invalid media message_id".into());
        }
        if self.hash.len() != 64 || !self.hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid media hash".into());
        }
        if decode_key(&self.key).is_none() {
            return Err("invalid media key".into());
        }
        if self.mime.is_empty()
            || self.mime.len() > 255
            || self.mime.bytes().any(|b| b.is_ascii_control())
        {
            return Err("invalid media mime".into());
        }
        if self.name.as_ref().is_some_and(|name| {
            name.is_empty() || name.len() > 255 || name.bytes().any(|b| b.is_ascii_control())
        }) {
            return Err("invalid media name".into());
        }
        if let Some(thumbnail) = &self.thumbnail {
            if thumbnail.hash.len() != 64
                || !thumbnail.hash.bytes().all(|b| b.is_ascii_hexdigit())
                || decode_key(&thumbnail.key).is_none()
                || thumbnail.mime.is_empty()
                || thumbnail.mime.len() > 255
                || thumbnail.mime.bytes().any(|b| b.is_ascii_control())
                || thumbnail.size == 0
            {
                return Err("invalid media thumbnail".into());
            }
        }
        Ok(())
    }

    /// Decode the per-file AES-256 key.
    pub fn key_bytes(&self) -> Option<[u8; 32]> {
        decode_key(&self.key).and_then(|bytes| bytes.try_into().ok())
    }
}

fn decode_key(value: &str) -> Option<Vec<u8>> {
    general_purpose::STANDARD
        .decode(value)
        .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(value))
        .or_else(|_| general_purpose::URL_SAFE.decode(value))
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(value))
        .ok()
        .filter(|bytes| bytes.len() == 32)
}

/// An ordinary text message with an optional quoted reply context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPayload {
    /// The message body.
    pub text: String,
    /// The message this one replies to, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<Quote>,
    /// The sender's id for this message, when known. The recipient stores the
    /// message under this SAME id (instead of a locally generated one), so
    /// reactions and replies — which reference the sender's id — resolve
    /// correctly on both ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// Disappearing-message lifetime in seconds: both ends auto-delete the
    /// message `expires_in_seconds` after it was sent/received. `None` (the
    /// default for older senders) means the message persists forever. Carried
    /// inside the encrypted payload, so the relay stays zero-knowledge — it
    /// cannot see which messages will expire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_seconds: Option<u64>,
}

impl TextPayload {
    /// Create a plain text payload without a quote or explicit message id.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            quote: None,
            message_id: None,
            expires_in_seconds: None,
        }
    }

    /// Create a text payload that quotes an earlier message.
    pub fn with_quote(text: impl Into<String>, quote: Quote) -> Self {
        Self {
            text: text.into(),
            quote: Some(quote),
            message_id: None,
            expires_in_seconds: None,
        }
    }

    /// Create a text payload carrying the sender's message id.
    pub fn with_id(text: impl Into<String>, message_id: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            quote: None,
            message_id: Some(message_id.into()),
            expires_in_seconds: None,
        }
    }

    /// Create a text payload that expires `seconds` after send/receive.
    pub fn with_expiration(
        text: impl Into<String>,
        message_id: Option<String>,
        expires_in_seconds: u64,
    ) -> Self {
        Self {
            text: text.into(),
            quote: None,
            message_id,
            expires_in_seconds: Some(expires_in_seconds),
        }
    }
}

/// The quoted message a reply refers to. Carries a snapshot of the quoted
/// message's text and sender so the reply renders standalone even if the
/// original message is later deleted locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    /// The id of the quoted message.
    pub message_id: String,
    /// A snapshot of the quoted message's plaintext.
    pub text: String,
    /// Peer ID of the quoted message's sender.
    pub sender: String,
    /// Display name of the quoted message's sender (optional; the UI falls
    /// back to the peer ID when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_name: Option<String>,
}

impl Quote {
    /// Create a quote snapshot.
    pub fn new(
        message_id: impl Into<String>,
        text: impl Into<String>,
        sender: impl Into<String>,
        sender_name: Option<String>,
    ) -> Self {
        Self {
            message_id: message_id.into(),
            text: text.into(),
            sender: sender.into(),
            sender_name,
        }
    }
}

/// An emoji reaction attached to a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionPayload {
    /// The id of the reacted-to message.
    pub message_id: String,
    /// The reaction emoji (a single Unicode grapheme cluster in practice;
    /// ZWJ sequences are allowed, e.g. family emojis).
    pub emoji: String,
    /// The reaction's absolute state: `true` = present, `false` = removed.
    ///
    /// The sender computes their own new state (toggle) and transmits it as an
    /// idempotent state signal, so the recipient applies it without knowing
    /// the previous state. This avoids toggle races and stays correct even
    /// when a reaction envelope is delivered before the message it targets.
    #[serde(default = "default_reaction_active")]
    pub active: bool,
}

/// A group typing indicator. Travels inside a Megolm-encrypted group envelope
/// (like reactions), so the relay never sees it and the recipient knows exactly
/// which member is composing — unlike 1:1 typing, which uses the Double
/// Ratchet receipt channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypingPayload {
    /// `true` = the sender started typing, `false` = stopped.
    pub active: bool,
}

impl TypingPayload {
    /// Create a typing indicator.
    pub fn new(active: bool) -> Self {
        Self { active }
    }
}

/// A group read receipt. Travels inside a Megolm-encrypted group envelope so
/// every member can count how many peers have read a message. The `message_id`
/// is the SENDER's id (the one embedded in the text payload), which is shared
/// across all recipients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadPayload {
    /// The id of the read message (the sender's id).
    pub message_id: String,
}

impl ReadPayload {
    /// Create a read receipt for `message_id`.
    pub fn new(message_id: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
        }
    }
}

/// An edit of an earlier message: replace the target message's text on every
/// recipient's device. `message_id` is the SENDER's id (the shared id both
/// ends stored the message under). Best-effort: if the target is unknown
/// (already expired or never delivered), the edit is a harmless no-op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditPayload {
    /// The id of the message being edited (the sender's id).
    pub message_id: String,
    /// The replacement plaintext.
    pub text: String,
}

impl EditPayload {
    /// Create an edit payload for `message_id` with the new `text`.
    pub fn new(message_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
            text: text.into(),
        }
    }
}

/// A delete-for-everyone request: remove the target message from every
/// recipient's device. `message_id` is the sender's id. Best-effort like
/// edits — an unknown target is a harmless no-op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletePayload {
    /// The id of the message to delete (the sender's id).
    pub message_id: String,
}

impl DeletePayload {
    /// Create a delete payload for `message_id`.
    pub fn new(message_id: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
        }
    }
}

/// Default for [`ReactionPayload::active`]: a payload without the field (older
/// senders) is treated as "reaction present".
fn default_reaction_active() -> bool {
    true
}

impl ReactionPayload {
    /// Create a reaction payload in the given state.
    pub fn new(message_id: impl Into<String>, emoji: impl Into<String>, active: bool) -> Self {
        Self {
            message_id: message_id.into(),
            emoji: emoji.into(),
            active,
        }
    }
}

/// Parse inbound plaintext into a uniform [`ParsedPayload`].
///
/// Order of attempts:
/// 1. tagged [`ChatPayload`] JSON — modern text and reaction envelopes;
/// 2. anything else — treated as legacy raw text (lossy UTF-8 conversion).
///
/// An unknown `kind` or malformed JSON falls through to the raw-text branch,
/// so messages from newer or foreign clients never crash the inbox.
pub fn parse_plaintext(bytes: &[u8]) -> ParsedPayload {
    if let Ok(payload) = serde_json::from_slice::<ChatPayload>(bytes) {
        return match payload {
            ChatPayload::Text(text) => ParsedPayload::Text(text),
            ChatPayload::Reaction(reaction) => ParsedPayload::Reaction(reaction),
            ChatPayload::Typing(typing) => ParsedPayload::Typing(typing),
            ChatPayload::Read(read) => ParsedPayload::Read(read),
            ChatPayload::Edit(edit) => ParsedPayload::Edit(edit),
            ChatPayload::Delete(delete) => ParsedPayload::Delete(delete),
            ChatPayload::Media(media) if media.validate().is_ok() => ParsedPayload::Media(media),
            ChatPayload::Media(_) => {
                return ParsedPayload::Text(TextPayload::new(
                    String::from_utf8_lossy(bytes).to_string(),
                ))
            }
        };
    }
    let text = String::from_utf8_lossy(bytes).to_string();
    ParsedPayload::Text(TextPayload::new(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_payload_roundtrip_preserves_fields() {
        let payload = ChatPayload::Text(TextPayload::new("hello"));
        let json = serde_json::to_string(&payload).expect("serialize");
        let restored: ChatPayload = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored, payload);
        match restored {
            ChatPayload::Text(text) => {
                assert_eq!(text.text, "hello");
                assert_eq!(text.quote, None);
            }
            ChatPayload::Reaction(_)
            | ChatPayload::Typing(_)
            | ChatPayload::Read(_)
            | ChatPayload::Edit(_)
            | ChatPayload::Delete(_)
            | ChatPayload::Media(_) => {
                panic!("expected text payload")
            }
        }
    }

    #[test]
    fn text_payload_with_quote_roundtrip_preserves_quote() {
        let quote = Quote::new("msg-1", "original text", "alice", Some("Alice".to_string()));
        let payload = ChatPayload::Text(TextPayload::with_quote("my reply", quote));
        let json = serde_json::to_string(&payload).expect("serialize");
        let restored: ChatPayload = serde_json::from_str(&json).expect("deserialize");

        let text = match restored {
            ChatPayload::Text(t) => t,
            _ => panic!("expected text payload"),
        };
        let quote = text.quote.expect("quote must be present");
        assert_eq!(text.text, "my reply");
        assert_eq!(quote.message_id, "msg-1");
        assert_eq!(quote.text, "original text");
        assert_eq!(quote.sender, "alice");
        assert_eq!(quote.sender_name.as_deref(), Some("Alice"));
    }

    #[test]
    fn quote_without_sender_name_skips_the_field_in_json() {
        let quote = Quote::new("msg-1", "original", "alice", None);
        let payload = ChatPayload::Text(TextPayload::with_quote("reply", quote));
        let json = serde_json::to_string(&payload).expect("serialize");

        assert!(
            !json.contains("sender_name"),
            "absent optional fields must be skipped: {json}"
        );
        assert!(
            !json.contains("quote\":null"),
            "absent quote must be skipped: {json}"
        );
    }

    #[test]
    fn reaction_payload_roundtrip_preserves_fields() {
        let payload = ChatPayload::Reaction(ReactionPayload::new("msg-1", "👍", true));
        let json = serde_json::to_string(&payload).expect("serialize");
        let restored: ChatPayload = serde_json::from_str(&json).expect("deserialize");

        let reaction = match restored {
            ChatPayload::Reaction(r) => r,
            _ => panic!("expected reaction payload"),
        };
        assert_eq!(reaction.message_id, "msg-1");
        assert_eq!(reaction.emoji, "👍");
        assert!(reaction.active);
    }

    #[test]
    fn reaction_payload_with_active_false_roundtrips() {
        let payload = ChatPayload::Reaction(ReactionPayload::new("msg-1", "👍", false));
        let json = serde_json::to_string(&payload).expect("serialize");
        let restored: ChatPayload = serde_json::from_str(&json).expect("deserialize");

        match restored {
            ChatPayload::Reaction(r) => assert!(!r.active),
            _ => panic!("expected reaction payload"),
        }
    }

    #[test]
    fn reaction_payload_defaults_active_to_true() {
        // A reaction without the `active` field (older sender) means "present".
        let bytes = r#"{"kind":"reaction","message_id":"m1","emoji":"👍"}"#;
        let parsed: ChatPayload = serde_json::from_str(bytes).expect("deserialize");
        match parsed {
            ChatPayload::Reaction(r) => assert!(r.active),
            _ => panic!("expected reaction payload"),
        }
    }

    #[test]
    fn reaction_supports_multi_codepoint_emoji() {
        // ZWJ sequence (family emoji) must survive a full roundtrip.
        let emoji = "👨‍👩‍👧‍👦";
        let payload = ChatPayload::Reaction(ReactionPayload::new("msg-1", emoji, true));
        let json = serde_json::to_string(&payload).expect("serialize");
        let restored: ChatPayload = serde_json::from_str(&json).expect("deserialize");

        match restored {
            ChatPayload::Reaction(r) => assert_eq!(r.emoji, emoji),
            _ => panic!("expected reaction payload"),
        }
    }

    #[test]
    fn parse_legacy_raw_text_returns_plain_text() {
        let parsed = parse_plaintext(b"just some text");
        match parsed {
            ParsedPayload::Text(text) => {
                assert_eq!(text.text, "just some text");
                assert_eq!(text.quote, None);
            }
            ParsedPayload::Reaction(_)
            | ParsedPayload::Typing(_)
            | ParsedPayload::Read(_)
            | ParsedPayload::Edit(_)
            | ParsedPayload::Delete(_)
            | ParsedPayload::Media(_) => {
                panic!("expected text")
            }
        }
    }

    #[test]
    fn parse_modern_text_envelope_returns_quote() {
        let bytes = br#"{"kind":"text","text":"replied","quote":{"message_id":"m1","text":"orig","sender":"bob","sender_name":"Bob"}}"#;
        match parse_plaintext(bytes) {
            ParsedPayload::Text(text) => {
                assert_eq!(text.text, "replied");
                let quote = text.quote.expect("quote present");
                assert_eq!(quote.message_id, "m1");
                assert_eq!(quote.sender_name.as_deref(), Some("Bob"));
            }
            ParsedPayload::Reaction(_)
            | ParsedPayload::Typing(_)
            | ParsedPayload::Read(_)
            | ParsedPayload::Edit(_)
            | ParsedPayload::Delete(_)
            | ParsedPayload::Media(_) => {
                panic!("expected text")
            }
        }
    }

    #[test]
    fn parse_modern_text_envelope_without_quote() {
        let bytes = br#"{"kind":"text","text":"hi"}"#;
        match parse_plaintext(bytes) {
            ParsedPayload::Text(text) => {
                assert_eq!(text.text, "hi");
                assert_eq!(text.quote, None);
            }
            ParsedPayload::Reaction(_)
            | ParsedPayload::Typing(_)
            | ParsedPayload::Read(_)
            | ParsedPayload::Edit(_)
            | ParsedPayload::Delete(_)
            | ParsedPayload::Media(_) => {
                panic!("expected text")
            }
        }
    }

    #[test]
    fn text_payload_with_message_id_roundtrips() {
        let payload = ChatPayload::Text(TextPayload::with_id("hello", "out-7"));
        let json = serde_json::to_string(&payload).expect("serialize");
        let restored: ChatPayload = serde_json::from_str(&json).expect("deserialize");

        match restored {
            ChatPayload::Text(text) => {
                assert_eq!(text.message_id.as_deref(), Some("out-7"));
            }
            ChatPayload::Reaction(_)
            | ChatPayload::Typing(_)
            | ChatPayload::Read(_)
            | ChatPayload::Edit(_)
            | ChatPayload::Delete(_)
            | ChatPayload::Media(_) => {
                panic!("expected text")
            }
        }
    }

    #[test]
    fn text_payload_with_expiration_roundtrips() {
        let payload = ChatPayload::Text(TextPayload::with_expiration(
            "secret",
            Some("out-9".into()),
            30,
        ));
        let json = serde_json::to_string(&payload).expect("serialize");
        let restored: ChatPayload = serde_json::from_str(&json).expect("deserialize");

        match restored {
            ChatPayload::Text(text) => {
                assert_eq!(text.text, "secret");
                assert_eq!(text.expires_in_seconds, Some(30));
            }
            ChatPayload::Reaction(_)
            | ChatPayload::Typing(_)
            | ChatPayload::Read(_)
            | ChatPayload::Edit(_)
            | ChatPayload::Delete(_)
            | ChatPayload::Media(_) => {
                panic!("expected text")
            }
        }
    }

    #[test]
    fn text_payload_without_expiration_skips_the_field_in_json() {
        let payload = ChatPayload::Text(TextPayload::new("hi"));
        let json = serde_json::to_string(&payload).expect("serialize");
        assert!(
            !json.contains("expires_in_seconds"),
            "absent expiration must be skipped: {json}"
        );
    }

    #[test]
    fn parse_text_envelope_exposes_expiration() {
        let bytes = br#"{"kind":"text","text":"bye","expires_in_seconds":3600}"#;
        match parse_plaintext(bytes) {
            ParsedPayload::Text(text) => {
                assert_eq!(text.expires_in_seconds, Some(3600));
            }
            ParsedPayload::Reaction(_)
            | ParsedPayload::Typing(_)
            | ParsedPayload::Read(_)
            | ParsedPayload::Edit(_)
            | ParsedPayload::Delete(_)
            | ParsedPayload::Media(_) => {
                panic!("expected text")
            }
        }
    }

    #[test]
    fn parse_text_envelope_exposes_the_sender_message_id() {
        let bytes = br#"{"kind":"text","text":"hi","message_id":"9f8e-3ab"}"#;
        match parse_plaintext(bytes) {
            ParsedPayload::Text(text) => {
                assert_eq!(text.message_id.as_deref(), Some("9f8e-3ab"));
            }
            ParsedPayload::Reaction(_)
            | ParsedPayload::Typing(_)
            | ParsedPayload::Read(_)
            | ParsedPayload::Edit(_)
            | ParsedPayload::Delete(_)
            | ParsedPayload::Media(_) => {
                panic!("expected text")
            }
        }
    }

    #[test]
    fn legacy_raw_text_has_no_message_id() {
        match parse_plaintext(b"plain legacy text") {
            ParsedPayload::Text(text) => assert_eq!(text.message_id, None),
            ParsedPayload::Reaction(_)
            | ParsedPayload::Typing(_)
            | ParsedPayload::Read(_)
            | ParsedPayload::Edit(_)
            | ParsedPayload::Delete(_)
            | ParsedPayload::Media(_) => {
                panic!("expected text")
            }
        }
    }

    #[test]
    fn parse_reaction_envelope_returns_reaction() {
        let bytes = "{\"kind\":\"reaction\",\"message_id\":\"m1\",\"emoji\":\"🔥\"}".as_bytes();
        match parse_plaintext(bytes) {
            ParsedPayload::Reaction(reaction) => {
                assert_eq!(reaction.message_id, "m1");
                assert_eq!(reaction.emoji, "🔥");
                assert!(reaction.active, "missing active defaults to present");
            }
            _ => panic!("expected reaction"),
        }
    }

    #[test]
    fn parse_reaction_envelope_with_active_false_returns_inactive() {
        let bytes =
            "{\"kind\":\"reaction\",\"message_id\":\"m1\",\"emoji\":\"🔥\",\"active\":false}"
                .as_bytes();
        match parse_plaintext(bytes) {
            ParsedPayload::Reaction(reaction) => assert!(!reaction.active),
            _ => panic!("expected reaction"),
        }
    }

    #[test]
    fn typing_payload_roundtrips_and_parses() {
        let payload = ChatPayload::Typing(TypingPayload::new(true));
        let json = serde_json::to_string(&payload).expect("serialize");
        let restored: ChatPayload = serde_json::from_str(&json).expect("deserialize");
        match restored {
            ChatPayload::Typing(typing) => assert!(typing.active),
            _ => panic!("expected typing"),
        }

        let bytes = "{\"kind\":\"typing\",\"active\":false}".as_bytes();
        match parse_plaintext(bytes) {
            ParsedPayload::Typing(typing) => assert!(!typing.active),
            _ => panic!("expected typing payload"),
        }
    }

    #[test]
    fn read_payload_roundtrips_and_parses() {
        let payload = ChatPayload::Read(ReadPayload::new("out-7"));
        let json = serde_json::to_string(&payload).expect("serialize");
        let restored: ChatPayload = serde_json::from_str(&json).expect("deserialize");
        match restored {
            ChatPayload::Read(read) => assert_eq!(read.message_id, "out-7"),
            ChatPayload::Reaction(_)
            | ChatPayload::Typing(_)
            | ChatPayload::Edit(_)
            | ChatPayload::Delete(_)
            | ChatPayload::Text(_)
            | ChatPayload::Media(_) => panic!("expected read"),
        }

        let bytes = "{\"kind\":\"read\",\"message_id\":\"out-7\"}".as_bytes();
        match parse_plaintext(bytes) {
            ParsedPayload::Read(read) => assert_eq!(read.message_id, "out-7"),
            ParsedPayload::Reaction(_)
            | ParsedPayload::Typing(_)
            | ParsedPayload::Edit(_)
            | ParsedPayload::Delete(_)
            | ParsedPayload::Text(_)
            | ParsedPayload::Media(_) => panic!("expected read payload"),
        }
    }

    #[test]
    fn edit_payload_roundtrips_and_parses() {
        let payload = ChatPayload::Edit(EditPayload::new("out-7", "corrected text"));
        let json = serde_json::to_string(&payload).expect("serialize");
        let restored: ChatPayload = serde_json::from_str(&json).expect("deserialize");
        match restored {
            ChatPayload::Edit(edit) => {
                assert_eq!(edit.message_id, "out-7");
                assert_eq!(edit.text, "corrected text");
            }
            _ => panic!("expected edit"),
        }

        let bytes = br#"{"kind":"edit","message_id":"out-7","text":"new body"}"#;
        match parse_plaintext(bytes) {
            ParsedPayload::Edit(edit) => {
                assert_eq!(edit.message_id, "out-7");
                assert_eq!(edit.text, "new body");
            }
            _ => panic!("expected edit payload"),
        }
    }

    #[test]
    fn delete_payload_roundtrips_and_parses() {
        let payload = ChatPayload::Delete(DeletePayload::new("out-7"));
        let json = serde_json::to_string(&payload).expect("serialize");
        let restored: ChatPayload = serde_json::from_str(&json).expect("deserialize");
        match restored {
            ChatPayload::Delete(delete) => assert_eq!(delete.message_id, "out-7"),
            _ => panic!("expected delete"),
        }

        let bytes = br#"{"kind":"delete","message_id":"out-7"}"#;
        match parse_plaintext(bytes) {
            ParsedPayload::Delete(delete) => assert_eq!(delete.message_id, "out-7"),
            _ => panic!("expected delete payload"),
        }
    }

    #[test]
    fn parse_unknown_kind_falls_back_to_raw_text() {
        // A future payload kind must degrade to raw text, never crash.
        let bytes = br#"{"kind":"voice_message","data":"xyz"}"#;
        match parse_plaintext(bytes) {
            ParsedPayload::Text(text) => {
                assert_eq!(text.text, r#"{"kind":"voice_message","data":"xyz"}"#);
                assert_eq!(text.quote, None);
            }
            ParsedPayload::Reaction(_)
            | ParsedPayload::Typing(_)
            | ParsedPayload::Read(_)
            | ParsedPayload::Edit(_)
            | ParsedPayload::Delete(_)
            | ParsedPayload::Media(_) => {
                panic!("expected text")
            }
        }
    }

    #[test]
    fn parse_group_key_json_falls_back_to_raw_text() {
        // Group-key shares are handled by the caller before payload parsing;
        // they must never be mistaken for a chat payload here.
        let bytes =
            br#"{"kind":"group_key","group_id":"g-1","session_key":"abc","group_name":"Squad"}"#;
        match parse_plaintext(bytes) {
            ParsedPayload::Text(text) => assert_eq!(text.quote, None),
            ParsedPayload::Reaction(_)
            | ParsedPayload::Typing(_)
            | ParsedPayload::Read(_)
            | ParsedPayload::Edit(_)
            | ParsedPayload::Delete(_)
            | ParsedPayload::Media(_) => {
                panic!("expected text")
            }
        }
    }

    #[test]
    fn parse_malformed_json_falls_back_to_raw_text() {
        let bytes = br#"{"kind":"text","text":"broken"#; // unterminated
        match parse_plaintext(bytes) {
            ParsedPayload::Text(text) => {
                assert!(text.text.contains("broken"));
            }
            ParsedPayload::Reaction(_)
            | ParsedPayload::Typing(_)
            | ParsedPayload::Read(_)
            | ParsedPayload::Edit(_)
            | ParsedPayload::Delete(_)
            | ParsedPayload::Media(_) => {
                panic!("expected text")
            }
        }
    }

    #[test]
    fn parse_non_utf8_bytes_are_lossy_converted() {
        let bytes = [0xff, 0xfe, b'a', b'b'];
        match parse_plaintext(&bytes) {
            ParsedPayload::Text(text) => {
                // Invalid bytes become U+FFFD replacement characters.
                assert!(text.text.contains('\u{FFFD}'));
            }
            ParsedPayload::Reaction(_)
            | ParsedPayload::Typing(_)
            | ParsedPayload::Read(_)
            | ParsedPayload::Edit(_)
            | ParsedPayload::Delete(_)
            | ParsedPayload::Media(_) => {
                panic!("expected text")
            }
        }
    }

    #[test]
    fn parse_plain_json_that_is_not_a_payload_falls_back() {
        // A plain JSON object with no `kind` tag is not a payload envelope.
        let bytes = br#"{"hello":"world"}"#;
        match parse_plaintext(bytes) {
            ParsedPayload::Text(text) => assert_eq!(text.text, r#"{"hello":"world"}"#),
            ParsedPayload::Reaction(_)
            | ParsedPayload::Typing(_)
            | ParsedPayload::Read(_)
            | ParsedPayload::Edit(_)
            | ParsedPayload::Delete(_)
            | ParsedPayload::Media(_) => {
                panic!("expected text")
            }
        }
    }

    #[test]
    fn media_payload_roundtrips_and_parses() {
        let key = [9u8; 32];
        let key_b64 = general_purpose::STANDARD.encode(key);
        let payload = MediaPayload::new(
            "msg-1",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            key_b64,
            "image/jpeg",
            123,
            Some("photo.jpg".into()),
            Some(2500),
        )
        .expect("valid media metadata");
        let wire = serde_json::to_vec(&ChatPayload::Media(payload.clone())).unwrap();
        assert_eq!(parse_plaintext(&wire), ParsedPayload::Media(payload));
    }

    #[test]
    fn media_thumbnail_roundtrips_and_validates_its_key() {
        let key_b64 = general_purpose::STANDARD.encode([9u8; 32]);
        let thumbnail = MediaThumbnail {
            hash: "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd".into(),
            key: key_b64.clone(),
            mime: "image/jpeg".into(),
            size: 42,
        };
        assert_eq!(thumbnail.key_bytes(), Some([9u8; 32]));
        let payload = MediaPayload::new(
            "msg-1",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            key_b64,
            "image/jpeg",
            123,
            None,
            None,
        )
        .unwrap()
        .with_thumbnail(thumbnail.clone())
        .unwrap();
        let wire = serde_json::to_vec(&ChatPayload::Media(payload.clone())).unwrap();
        assert_eq!(parse_plaintext(&wire), ParsedPayload::Media(payload));
    }

    #[test]
    fn invalid_media_metadata_uses_legacy_fallback() {
        let wire = br#"{"kind":"media","message_id":"m","hash":"bad","key":"bad","mime":"image/jpeg","size":1}"#;
        match parse_plaintext(wire) {
            ParsedPayload::Text(text) => assert_eq!(text.text, String::from_utf8_lossy(wire)),
            ParsedPayload::Media(_) => panic!("invalid media must not be accepted"),
            _ => panic!("expected legacy fallback"),
        }
    }
}
