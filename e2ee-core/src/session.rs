//! Double Ratchet sessions built on top of the X3DH handshake.
//!
//! The initiator creates an outbound [`ChatSession`] from the recipient's
//! verified [`PreKeyBundle`]; the recipient creates the matching inbound
//! session from the first pre-key message. After the handshake both sides
//! encrypt and decrypt messages with the Double Ratchet provided by vodozemac.
//!
//! Sessions can be serialized to JSON so they survive restarts (for example
//! persisted in SQLCipher).

use std::fmt;
use vodozemac::olm::{
    DecryptionError, EncryptionError, InboundCreationResult, OlmMessage, PreKeyMessage, Session,
    SessionConfig, SessionCreationError, SessionPickle,
};
use vodozemac::Curve25519PublicKey;

use crate::identity::Identity;
use crate::prekey::{PreKeyBundle, PreKeyBundleError};

/// Errors that can occur during session establishment or message exchange.
#[derive(Debug)]
pub enum SessionError {
    /// The remote [`PreKeyBundle`] could not be trusted.
    InvalidBundle(PreKeyBundleError),
    /// The X3DH session could not be created.
    Creation(SessionCreationError),
    /// A message could not be encrypted.
    Encrypt(EncryptionError),
    /// A message could not be decrypted.
    Decrypt(DecryptionError),
    /// The session could not be serialized.
    Serialize(serde_json::Error),
    /// The session could not be deserialized.
    Deserialize(serde_json::Error),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBundle(e) => write!(f, "invalid pre-key bundle: {e}"),
            Self::Creation(e) => write!(f, "failed to create session: {e}"),
            Self::Encrypt(e) => write!(f, "failed to encrypt message: {e}"),
            Self::Decrypt(e) => write!(f, "failed to decrypt message: {e}"),
            Self::Serialize(e) => write!(f, "failed to serialize session: {e}"),
            Self::Deserialize(e) => write!(f, "failed to deserialize session: {e}"),
        }
    }
}

impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidBundle(e) => Some(e),
            Self::Creation(e) => Some(e),
            Self::Encrypt(e) => Some(e),
            Self::Decrypt(e) => Some(e),
            Self::Serialize(e) | Self::Deserialize(e) => Some(e),
        }
    }
}

/// One end of an encrypted 1:1 communication channel.
///
/// After the initial X3DH handshake the session provides Double Ratchet
/// encryption and decryption. It is safe to clone-persist via [`ChatSession::to_json`].
#[derive(Debug)]
pub struct ChatSession {
    inner: Session,
}

impl ChatSession {
    /// Establish an outbound session as the initiator, from the recipient's
    /// verified [`PreKeyBundle`].
    ///
    /// The bundle signature is verified first; the first one-time key in the
    /// bundle is consumed for the X3DH handshake.
    pub fn create_outbound(
        identity: &Identity,
        bundle: &PreKeyBundle,
    ) -> Result<Self, SessionError> {
        bundle.ensure_valid().map_err(SessionError::InvalidBundle)?;
        let one_time_key = bundle
            .one_time_keys
            .first()
            .ok_or(PreKeyBundleError::NoOneTimeKeys)
            .map_err(SessionError::InvalidBundle)?;

        let session = identity
            .account()
            .create_outbound_session(
                SessionConfig::version_1(),
                bundle.identity_key,
                *one_time_key,
            )
            .map_err(SessionError::Creation)?;

        Ok(Self { inner: session })
    }

    /// Establish the matching inbound session as the recipient.
    ///
    /// `their_identity_key` is the initiator's X25519 identity key, which is
    /// cross-checked against the one carried in the pre-key message. The
    /// returned [`InboundSession`] also contains the plaintext of the very
    /// first message.
    pub fn create_inbound(
        identity: &mut Identity,
        their_identity_key: Curve25519PublicKey,
        pre_key_message: &PreKeyMessage,
    ) -> Result<InboundSession, SessionError> {
        let InboundCreationResult { session, plaintext } = identity
            .account_mut()
            .create_inbound_session(
                SessionConfig::version_1(),
                their_identity_key,
                pre_key_message,
            )
            .map_err(SessionError::Creation)?;

        Ok(InboundSession {
            session: Self { inner: session },
            plaintext,
        })
    }

    /// Encrypt a plaintext message into a transportable [`OlmMessage`].
    pub fn encrypt(&mut self, plaintext: impl AsRef<[u8]>) -> Result<OlmMessage, SessionError> {
        self.inner.encrypt(plaintext).map_err(SessionError::Encrypt)
    }

    /// Decrypt an incoming [`OlmMessage`].
    pub fn decrypt(&mut self, message: &OlmMessage) -> Result<Vec<u8>, SessionError> {
        self.inner.decrypt(message).map_err(SessionError::Decrypt)
    }

    /// The globally unique base64-encoded session ID shared by both sides.
    pub fn session_id(&self) -> String {
        self.inner.session_id()
    }

    /// Whether this session has received and decrypted a message yet.
    pub fn has_received_message(&self) -> bool {
        self.inner.has_received_message()
    }

    /// Serialize the full session state to JSON.
    pub fn to_json(&self) -> Result<String, SessionError> {
        let pickle = self.inner.pickle();
        serde_json::to_string(&pickle).map_err(SessionError::Serialize)
    }

    /// Restore a session previously stored with [`ChatSession::to_json`].
    pub fn from_json(json: &str) -> Result<Self, SessionError> {
        let pickle: SessionPickle =
            serde_json::from_str(json).map_err(SessionError::Deserialize)?;
        Ok(Self {
            inner: Session::from_pickle(pickle),
        })
    }
}

/// The result of establishing an inbound session: the new session plus the
/// plaintext of the first message that triggered the handshake.
#[derive(Debug)]
pub struct InboundSession {
    /// The freshly established session.
    pub session: ChatSession,
    /// The plaintext of the first message from the initiator.
    pub plaintext: Vec<u8>,
}

/// Establish a session pair between two identities for tests.
#[cfg(test)]
fn establish_session_pair() -> (ChatSession, ChatSession) {
    let alice = Identity::new();
    let mut bob = Identity::new();
    let bundle = bob.pre_key_bundle(5);
    let mut alice_session = ChatSession::create_outbound(&alice, &bundle)
        .expect("outbound session creation must succeed");

    let first_message = alice_session
        .encrypt(b"hello bob")
        .expect("encrypt must succeed");
    let inbound = match first_message {
        OlmMessage::PreKey(message) => {
            ChatSession::create_inbound(&mut bob, alice.curve25519_key(), &message)
                .expect("inbound session creation must succeed")
        }
        _ => panic!("first message must be a pre-key message"),
    };

    assert_eq!(inbound.plaintext, b"hello bob");
    assert_eq!(alice_session.session_id(), inbound.session.session_id());

    (alice_session, inbound.session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x3dh_roundtrip_decrypts_first_message() {
        let (mut alice_session, mut bob_session) = establish_session_pair();

        let reply = bob_session
            .encrypt(b"got it")
            .expect("encrypt must succeed");
        let plaintext = alice_session.decrypt(&reply).expect("decrypt must succeed");
        assert_eq!(plaintext, b"got it");
    }

    #[test]
    fn multiple_messages_ratchet_forward_sequentially() {
        let (mut alice_session, mut bob_session) = establish_session_pair();

        let messages = [b"one".as_slice(), b"two".as_slice(), b"three".as_slice()];
        let mut ciphertexts = Vec::new();
        for message in &messages {
            ciphertexts.push(
                alice_session
                    .encrypt(message)
                    .expect("encrypt must succeed"),
            );
        }

        for (ciphertext, expected) in ciphertexts.iter().zip(&messages) {
            let plaintext = bob_session
                .decrypt(ciphertext)
                .expect("decrypt must succeed");
            assert_eq!(&plaintext[..], *expected);
        }

        // The exchange continues in both directions.
        let reply = bob_session.encrypt(b"four").expect("encrypt must succeed");
        assert_eq!(
            alice_session.decrypt(&reply).expect("decrypt must succeed"),
            b"four"
        );
    }

    #[test]
    fn sessions_use_distinct_keys_per_message_forward_secrecy() {
        let (mut alice_session, mut bob_session) = establish_session_pair();

        let m1 = alice_session
            .encrypt(b"first")
            .expect("encrypt must succeed");
        let m2 = alice_session
            .encrypt(b"second")
            .expect("encrypt must succeed");
        let m3 = alice_session
            .encrypt(b"third")
            .expect("encrypt must succeed");

        assert_eq!(
            bob_session.decrypt(&m1).expect("decrypt must succeed"),
            b"first"
        );
        assert_eq!(
            bob_session.decrypt(&m2).expect("decrypt must succeed"),
            b"second"
        );
        assert_eq!(
            bob_session.decrypt(&m3).expect("decrypt must succeed"),
            b"third"
        );

        // Once the ratchet has advanced, the message key for m1 is destroyed:
        // the old ciphertext can no longer be opened with the current state.
        assert!(
            matches!(
                bob_session.decrypt(&m1),
                Err(SessionError::Decrypt(DecryptionError::MissingMessageKey(_)))
            ),
            "replaying an old ciphertext after the ratchet advanced must fail"
        );
    }

    #[test]
    fn session_json_roundtrip_continues_operation() {
        let (alice_session, bob_session) = establish_session_pair();

        let alice_json = alice_session.to_json().expect("serialization must succeed");
        let bob_json = bob_session.to_json().expect("serialization must succeed");

        let mut alice_restored = ChatSession::from_json(&alice_json).expect("must deserialize");
        let mut bob_restored = ChatSession::from_json(&bob_json).expect("must deserialize");

        let message = alice_restored
            .encrypt(b"after restore")
            .expect("encrypt must succeed");
        assert_eq!(
            bob_restored
                .decrypt(&message)
                .expect("decrypt must succeed"),
            b"after restore"
        );

        let reply = bob_restored
            .encrypt(b"still works")
            .expect("encrypt must succeed");
        assert_eq!(
            alice_restored
                .decrypt(&reply)
                .expect("decrypt must succeed"),
            b"still works"
        );
    }
}
