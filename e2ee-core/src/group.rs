//! Megolm group sessions for end-to-end encrypted group chat (protocol v1).
//!
//! One sender (the group's creator) holds an [`OutboundGroup`] and shares its
//! Megolm [`session_key`](OutboundGroup::session_key) with the other members.
//! Each member builds an [`InboundGroup`] from that key and can then decrypt
//! every message the sender encrypts. All crypto is delegated to vodozemac's
//! Megolm implementation; no custom primitives are used.
//!
//! # Key distribution model
//!
//! The Megolm `session_key` is *secret* material: anyone holding it can decrypt
//! the group's messages. It must therefore travel end-to-end between members
//! over an authenticated Double Ratchet channel (an [`crate::wire::Envelope`]
//! of kind `Message`), never through the zero-knowledge relay. The server only
//! routes opaque envelopes and group metadata.
//!
//! # Known limitations (MVP)
//!
//! - Ratchet rotation is not implemented yet: a single `session_key` is shared
//!   at join time and never rotated. Members who join later cannot decrypt
//!   earlier messages (their inbound session starts at the current ratchet
//!   index). Rotation via fresh `session_key` shares is a follow-up.
//! - For a group with several senders each sender needs its own outbound
//!   session and key; the current API models a single creator/sender.

use std::fmt;

use vodozemac::megolm::{
    DecryptionError as MegolmDecryptionError, ExportedSessionKey, GroupSession, GroupSessionPickle,
    InboundGroupSession, InboundGroupSessionPickle, MegolmMessage, SessionConfig, SessionKey,
    SessionKeyDecodeError,
};

/// Errors that can occur while managing or using a Megolm group session.
#[derive(Debug)]
pub enum GroupError {
    /// A session key (or an exported key) could not be parsed or verified.
    SessionKey(SessionKeyDecodeError),
    /// A Megolm ciphertext could not be decoded from base64.
    DecodeMessage(vodozemac::DecodeError),
    /// A Megolm ciphertext could not be decrypted (bad signature, MAC, index).
    Decrypt(MegolmDecryptionError),
    /// The session could not be serialized.
    Serialize(serde_json::Error),
    /// The session could not be deserialized.
    Deserialize(serde_json::Error),
}

impl fmt::Display for GroupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionKey(e) => write!(f, "invalid Megolm session key: {e}"),
            Self::DecodeMessage(e) => write!(f, "failed to decode Megolm ciphertext: {e}"),
            Self::Decrypt(e) => write!(f, "failed to decrypt Megolm message: {e}"),
            Self::Serialize(e) => write!(f, "failed to serialize group session: {e}"),
            Self::Deserialize(e) => write!(f, "failed to deserialize group session: {e}"),
        }
    }
}

impl std::error::Error for GroupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SessionKey(e) => Some(e),
            Self::DecodeMessage(e) => Some(e),
            Self::Decrypt(e) => Some(e),
            Self::Serialize(e) | Self::Deserialize(e) => Some(e),
        }
    }
}

/// The sending side of a Megolm group session.
///
/// One instance exists per group (per sender). It encrypts messages for every
/// member and holds the secret ratchet state plus the group's Ed25519 signing
/// key. The serializable `session_key` is shared with members over an
/// end-to-end channel so they can build an [`InboundGroup`].
pub struct OutboundGroup {
    inner: GroupSession,
}

impl fmt::Debug for OutboundGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutboundGroup")
            .field("session_id", &self.session_id())
            .field("message_index", &self.message_index())
            .finish_non_exhaustive()
    }
}

impl OutboundGroup {
    /// Create a fresh version-1 outbound group session with a random ratchet
    /// state and signing key.
    pub fn new() -> Self {
        Self {
            inner: GroupSession::new(SessionConfig::version_1()),
        }
    }

    /// The base64-encoded session key to share with new members.
    ///
    /// This is secret key material: it must be sent to members over an
    /// authenticated end-to-end channel, never via the relay.
    pub fn session_key(&self) -> String {
        self.inner.session_key().to_base64()
    }

    /// The base64-encoded session ID, derived from the group's Ed25519 signing
    /// public key.
    pub fn session_id(&self) -> String {
        self.inner.session_id()
    }

    /// The current ratchet index, incremented with every encrypted message.
    pub fn message_index(&self) -> u32 {
        self.inner.message_index()
    }

    /// Encrypt `plaintext` with the group ratchet, returning the
    /// base64-encoded Megolm ciphertext.
    pub fn encrypt(&mut self, plaintext: impl AsRef<[u8]>) -> String {
        self.inner.encrypt(plaintext).to_base64()
    }

    /// Serialize the full outbound session state to JSON.
    pub fn to_json(&self) -> Result<String, GroupError> {
        let pickle = self.inner.pickle();
        serde_json::to_string(&pickle).map_err(GroupError::Serialize)
    }

    /// Restore an outbound session previously stored with
    /// [`OutboundGroup::to_json`].
    pub fn from_json(json: &str) -> Result<Self, GroupError> {
        let pickle: GroupSessionPickle =
            serde_json::from_str(json).map_err(GroupError::Deserialize)?;
        Ok(Self {
            inner: GroupSession::from_pickle(pickle),
        })
    }
}

impl Default for OutboundGroup {
    fn default() -> Self {
        Self::new()
    }
}

/// The receiving side of a Megolm group session.
///
/// A member builds one inbound session from the creator's `session_key` and
/// uses it to decrypt that creator's messages. It holds the ratchet plus the
/// group's Ed25519 *public* key used to verify message signatures.
pub struct InboundGroup {
    inner: InboundGroupSession,
}

impl fmt::Debug for InboundGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InboundGroup")
            .field("session_id", &self.session_id())
            .field("first_known_index", &self.first_known_index())
            .finish_non_exhaustive()
    }
}

impl InboundGroup {
    /// Build an inbound session from the creator's base64 `session_key`.
    ///
    /// The key is parsed and its embedded Ed25519 signature verified; a
    /// malformed or tampered key is rejected.
    pub fn new(session_key: &str) -> Result<Self, GroupError> {
        let key = SessionKey::from_base64(session_key).map_err(GroupError::SessionKey)?;
        Ok(Self {
            inner: InboundGroupSession::new(&key, SessionConfig::version_1()),
        })
    }

    /// The base64-encoded session ID, matching the creator's outbound
    /// [`OutboundGroup::session_id`].
    pub fn session_id(&self) -> String {
        self.inner.session_id()
    }

    /// The first message index this session can still decrypt.
    pub fn first_known_index(&self) -> u32 {
        self.inner.first_known_index()
    }

    /// Decrypt a base64-encoded Megolm ciphertext.
    ///
    /// Returns the plaintext bytes. Authentication failures (invalid Ed25519
    /// signature, bad MAC or an unknown ratchet index) are reported as
    /// [`GroupError::Decrypt`].
    pub fn decrypt(&mut self, ciphertext_b64: &str) -> Result<Vec<u8>, GroupError> {
        let message =
            MegolmMessage::from_base64(ciphertext_b64).map_err(GroupError::DecodeMessage)?;
        let decrypted = self.inner.decrypt(&message).map_err(GroupError::Decrypt)?;
        Ok(decrypted.plaintext)
    }

    /// Serialize the full inbound session state to JSON.
    pub fn to_json(&self) -> Result<String, GroupError> {
        let pickle = self.inner.pickle();
        serde_json::to_string(&pickle).map_err(GroupError::Serialize)
    }

    /// Restore an inbound session previously stored with
    /// [`InboundGroup::to_json`].
    pub fn from_json(json: &str) -> Result<Self, GroupError> {
        let pickle: InboundGroupSessionPickle =
            serde_json::from_str(json).map_err(GroupError::Deserialize)?;
        Ok(Self {
            inner: InboundGroupSession::from_pickle(pickle),
        })
    }

    /// Export the session at its first known message index as a base64
    /// "exported session key".
    ///
    /// An exported key lets another device import the session *without* the
    /// creator's signature, so it must be authenticated out of band. This is
    /// the building block for syncing a joined member's decryption state to a
    /// second device; rotation of ratchet keys is not yet implemented.
    pub fn export(&self) -> String {
        self.inner.export_at_first_known_index().to_base64()
    }

    /// Import a session from an [`export`](InboundGroup::export)-produced
    /// base64 exported session key.
    ///
    /// The signature of the original creator is not part of an exported key,
    /// so the caller is responsible for authenticating the source.
    pub fn import(exported_key: &str) -> Result<Self, GroupError> {
        let key = ExportedSessionKey::from_base64(exported_key).map_err(GroupError::SessionKey)?;
        Ok(Self {
            inner: InboundGroupSession::import(&key, SessionConfig::version_1()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A freshly created outbound/inbound pair bound by the same session key.
    fn pair() -> (OutboundGroup, InboundGroup) {
        let outbound = OutboundGroup::new();
        let key = outbound.session_key();
        let inbound = InboundGroup::new(&key).expect("session key must parse");
        assert_eq!(outbound.session_id(), inbound.session_id());
        (outbound, inbound)
    }

    #[test]
    fn outbound_inbound_roundtrip_decrypts_three_messages() {
        let (mut outbound, mut inbound) = pair();
        let messages = [
            b"hello group".as_slice(),
            b"second message".as_slice(),
            b"third message".as_slice(),
        ];

        for message in &messages {
            let ciphertext = outbound.encrypt(message);
            assert_eq!(
                inbound.decrypt(&ciphertext).expect("decrypt must succeed"),
                *message
            );
        }
    }

    #[test]
    fn different_session_key_cannot_decrypt() {
        let (mut outbound, _inbound) = pair();
        let other = OutboundGroup::new();
        let mut stranger = InboundGroup::new(&other.session_key()).expect("key must parse");

        let ciphertext = outbound.encrypt(b"classified");
        assert!(
            matches!(stranger.decrypt(&ciphertext), Err(GroupError::Decrypt(_))),
            "a session built from another key must not open the ciphertext"
        );
    }

    #[test]
    fn tampered_session_key_is_rejected() {
        let outbound = OutboundGroup::new();
        let mut key = outbound.session_key();
        // Flip one base64 character in the middle of the key. The embedded
        // Ed25519 signature (or the base64 decoder) must reject it.
        let i = key.len() / 2;
        key.replace_range(i..=i, if key.as_bytes()[i] == b'A' { "B" } else { "A" });

        assert!(
            InboundGroup::new(&key).is_err(),
            "a tampered session key must be rejected"
        );
    }

    #[test]
    fn pickle_roundtrip_continues_operation() {
        let (outbound, inbound) = pair();

        let outbound_json = outbound.to_json().expect("serialization must succeed");
        let inbound_json = inbound.to_json().expect("serialization must succeed");

        let mut restored_outbound =
            OutboundGroup::from_json(&outbound_json).expect("outbound must deserialize");
        let mut restored_inbound =
            InboundGroup::from_json(&inbound_json).expect("inbound must deserialize");

        assert_eq!(restored_outbound.session_id(), inbound.session_id());

        for message in [b"after restore".as_slice(), b"still encrypted".as_slice()] {
            let ciphertext = restored_outbound.encrypt(message);
            assert_eq!(
                restored_inbound
                    .decrypt(&ciphertext)
                    .expect("decrypt must succeed after restore"),
                *message
            );
        }
    }

    #[test]
    fn session_id_survives_pickle_roundtrip() {
        let (outbound, inbound) = pair();
        let outbound_id = outbound.session_id();
        let inbound_id = inbound.session_id();

        let restored_out =
            OutboundGroup::from_json(&outbound.to_json().unwrap()).expect("must deserialize");
        let restored_in =
            InboundGroup::from_json(&inbound.to_json().unwrap()).expect("must deserialize");

        assert_eq!(restored_out.session_id(), outbound_id);
        assert_eq!(restored_in.session_id(), inbound_id);
    }

    #[test]
    fn export_import_recreates_working_inbound_session() {
        let (mut outbound, mut inbound) = pair();
        for i in 0..2 {
            let ciphertext = outbound.encrypt(format!("msg {i}"));
            inbound.decrypt(&ciphertext).expect("decrypt must succeed");
        }

        let exported = inbound.export();
        let mut imported = InboundGroup::import(&exported).expect("exported key must parse");
        assert_eq!(imported.session_id(), inbound.session_id());

        // The imported session decrypts the sender's next message (index 2)
        // just like the original.
        let ciphertext = outbound.encrypt(b"replay check");
        assert_eq!(
            imported.decrypt(&ciphertext).expect("decrypt must succeed"),
            b"replay check"
        );
    }

    #[test]
    fn tampered_exported_key_is_rejected() {
        let (_, inbound) = pair();
        let exported = inbound.export();

        // An exported key is NOT self-authenticating (it carries no signature
        // by design), so the caller must authenticate its source. Malformed
        // base64 and a truncated payload are still rejected outright.
        assert!(
            InboundGroup::import("!!!not-base64!!!").is_err(),
            "malformed base64 must be rejected"
        );
        assert!(
            InboundGroup::import(&exported[..exported.len() / 2]).is_err(),
            "a truncated exported key must be rejected"
        );
        assert!(InboundGroup::import(&exported).is_ok());
    }

    #[test]
    fn encrypt_and_decrypt_roundtrip_via_wire_base64() {
        // Guard: what `encrypt` emits is exactly what `decrypt` consumes — a
        // plain base64 string with no extra framing.
        let (mut outbound, mut inbound) = pair();
        let ciphertext = outbound.encrypt(b"payload");
        assert!(ciphertext
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c)));
        assert_eq!(
            inbound.decrypt(&ciphertext).expect("decrypt must succeed"),
            b"payload"
        );
    }

    #[test]
    fn message_index_increments_with_each_encryption() {
        // The rotation trigger: `message_index()` must climb with every
        // encrypted message so the caller can rotate after a bounded number.
        let mut outbound = OutboundGroup::new();
        assert_eq!(outbound.message_index(), 0);
        for expected in 1..=5 {
            let _ = outbound.encrypt(b"x");
            assert_eq!(outbound.message_index(), expected);
        }
    }

    #[test]
    fn rotation_fresh_session_key_differs_and_old_key_cannot_decrypt() {
        // Rotating to a fresh OutboundGroup yields a DIFFERENT session key:
        // the old key (which an attacker may have captured) can no longer
        // decrypt anything encrypted after the rotation — backward secrecy.
        let mut old = OutboundGroup::new();
        let old_key = old.session_key();
        // Burn some messages on the old stream, then rotate.
        for _ in 0..3 {
            let _ = old.encrypt(b"before");
        }
        let rotated = OutboundGroup::new();
        assert_ne!(
            old_key,
            rotated.session_key(),
            "rotation must mint a new key"
        );

        // The old inbound (from the old key) opens old ciphertext but NOT the
        // fresh stream's ciphertext.
        let mut old_inbound = InboundGroup::new(&old_key).expect("old key parses");
        let pre = old.encrypt(b"old message");
        assert_eq!(
            old_inbound
                .decrypt(&pre)
                .expect("old key opens old message"),
            b"old message"
        );
        let mut rotated_outbound = rotated;
        let post = rotated_outbound.encrypt(b"new message");
        assert!(
            old_inbound.decrypt(&post).is_err(),
            "the rotated-away key must not open post-rotation messages"
        );
    }
}
