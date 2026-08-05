//! Long-term identity key management.
//!
//! An [`Identity`] wraps a vodozemac [`Account`], which holds the two long-term
//! key pairs every client needs:
//!
//! - an X25519 (Curve25519) key pair used as the DH identity key for the X3DH
//!   handshake, and
//! - an Ed25519 key pair used to sign key material (for example the
//!   [`crate::prekey::PreKeyBundle`]).
//!
//! The peer ID is a short fingerprint (SHA-256 truncated to 16 hex chars)
//! derived from the public X25519 identity key. It is deterministic for a
//! given public key and is used to address peers on the relay.
//!
//! Identities can be serialized to JSON via vodozemac's pickle mechanism so
//! they can be persisted, for example in a SQLCipher database.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use vodozemac::olm::{Account, AccountPickle, IdentityKeys, OneTimeKeyGenerationResult};
use vodozemac::{Curve25519PublicKey, Ed25519PublicKey, Ed25519Signature, KeyId};

/// Number of characters in a peer ID (16 hex chars = 64 bits of entropy).
pub const PEER_ID_LENGTH: usize = 16;

/// Errors that can occur while managing or persisting an [`Identity`].
#[derive(Debug)]
pub enum IdentityError {
    /// The identity could not be serialized.
    Serialize(serde_json::Error),
    /// The identity could not be deserialized.
    Deserialize(serde_json::Error),
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serialize(e) => write!(f, "failed to serialize identity: {e}"),
            Self::Deserialize(e) => write!(f, "failed to deserialize identity: {e}"),
        }
    }
}

impl std::error::Error for IdentityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Serialize(e) | Self::Deserialize(e) => Some(e),
        }
    }
}

/// A self-authenticating hello: the peer ID bound to its public keys and
/// signed with the identity's Ed25519 key.
///
/// The relay can verify the hello without holding any secret material:
/// `peer_id` must be the fingerprint of `curve25519_key` and `signature`
/// must verify under `ed25519_key`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedHello {
    /// Peer ID fingerprint of `curve25519_key`.
    pub peer_id: String,
    /// The public X25519 identity key, base64-encoded (raw 32 bytes).
    pub curve25519_key: String,
    /// The public Ed25519 signing key, base64-encoded (raw 32 bytes).
    pub ed25519_key: String,
    /// Ed25519 signature over the UTF-8 bytes of `peer_id`.
    pub signature: String,
}

/// Errors that can occur while verifying a [`SignedHello`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelloError {
    /// A public key could not be parsed (malformed base64 or wrong size).
    InvalidKey,
    /// The peer ID does not match the fingerprint of `curve25519_key`.
    InvalidPeerId,
    /// The Ed25519 signature does not verify under `ed25519_key`.
    InvalidSignature,
}

impl fmt::Display for HelloError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey => write!(f, "invalid public key in signed hello"),
            Self::InvalidPeerId => write!(f, "peer id does not match the curve25519 key"),
            Self::InvalidSignature => write!(f, "invalid signature in signed hello"),
        }
    }
}

impl std::error::Error for HelloError {}

/// A single device's cryptographic identity.
///
/// New instances are created with [`Identity::new`]. The underlying vodozemac
/// [`Account`] is kept private; all session creation goes through the helpers
/// exposed by the [`crate::session`] module.
pub struct Identity {
    account: Account,
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("peer_id", &self.peer_id())
            .field("curve25519_key", &self.curve25519_key().to_base64())
            .field("ed25519_key", &self.ed25519_key().to_base64())
            .finish_non_exhaustive()
    }
}

impl Identity {
    /// Create a new identity with freshly generated random keys.
    pub fn new() -> Self {
        Self {
            account: Account::new(),
        }
    }

    /// Compute the peer ID for a given public X25519 identity key.
    ///
    /// The ID is the first [`PEER_ID_LENGTH`] hex characters of the SHA-256
    /// digest of the raw public key bytes. It is deterministic: the same
    /// public key always yields the same peer ID.
    pub fn peer_id_from_curve25519(identity_key: &Curve25519PublicKey) -> String {
        let digest = Sha256::digest(identity_key.as_bytes());

        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut peer_id = String::with_capacity(PEER_ID_LENGTH);
        for byte in digest.iter().take(PEER_ID_LENGTH / 2) {
            peer_id.push(HEX[(byte >> 4) as usize] as char);
            peer_id.push(HEX[(byte & 0x0f) as usize] as char);
        }
        peer_id
    }

    /// The peer ID of this identity.
    pub fn peer_id(&self) -> String {
        Self::peer_id_from_curve25519(&self.curve25519_key())
    }

    /// The public identity keys (X25519 and Ed25519) of this identity.
    pub fn identity_keys(&self) -> IdentityKeys {
        self.account.identity_keys()
    }

    /// The public X25519 key used as the DH identity key in the X3DH handshake.
    pub fn curve25519_key(&self) -> Curve25519PublicKey {
        self.account.curve25519_key()
    }

    /// The public Ed25519 key used to sign key material.
    pub fn ed25519_key(&self) -> Ed25519PublicKey {
        self.account.ed25519_key()
    }

    /// Sign a message with the identity's Ed25519 signing key.
    pub fn sign(&self, message: impl AsRef<[u8]>) -> Ed25519Signature {
        self.account.sign(message.as_ref())
    }

    /// Verify that `signature` is valid for `message` under this identity's
    /// Ed25519 public key.
    pub fn verify(&self, message: &[u8], signature: &Ed25519Signature) -> bool {
        self.ed25519_key().verify(message, signature).is_ok()
    }

    /// Build a self-authenticating [`SignedHello`] for this identity.
    ///
    /// The peer ID is signed with the identity's Ed25519 key, binding the
    /// peer ID to both public keys so a relay can authenticate the socket
    /// without any secret material.
    pub fn signed_hello(&self) -> SignedHello {
        let peer_id = self.peer_id();
        let signature = self.sign(peer_id.as_bytes());
        SignedHello {
            peer_id,
            curve25519_key: self.curve25519_key().to_base64(),
            ed25519_key: self.ed25519_key().to_base64(),
            signature: signature.to_base64(),
        }
    }

    /// Verify a [`SignedHello`] against its embedded public keys.
    ///
    /// Checks, in order:
    /// 1. both public keys and the signature decode to valid keys,
    /// 2. `peer_id` equals the fingerprint of `curve25519_key`, and
    /// 3. the Ed25519 signature verifies over the `peer_id` bytes.
    ///
    /// No secret material is involved: any party can authenticate the hello.
    pub fn verify_signed_hello(hello: &SignedHello) -> Result<(), HelloError> {
        let curve_key = Curve25519PublicKey::from_base64(&hello.curve25519_key)
            .map_err(|_| HelloError::InvalidKey)?;
        let ed_key = Ed25519PublicKey::from_base64(&hello.ed25519_key)
            .map_err(|_| HelloError::InvalidKey)?;
        let signature = Ed25519Signature::from_base64(&hello.signature)
            .map_err(|_| HelloError::InvalidSignature)?;

        if hello.peer_id != Self::peer_id_from_curve25519(&curve_key) {
            return Err(HelloError::InvalidPeerId);
        }

        ed_key
            .verify(hello.peer_id.as_bytes(), &signature)
            .map_err(|_| HelloError::InvalidSignature)
    }

    /// Build a signed [`PreKeyBundle`] containing `count` freshly generated
    /// one-time keys.
    ///
    /// The bundle's signature authenticates the identity key together with all
    /// one-time keys, so recipients can detect tampering before starting a
    /// session.
    pub fn pre_key_bundle(&mut self, count: usize) -> crate::prekey::PreKeyBundle {
        self.generate_one_time_keys(count);
        let one_time_keys: Vec<Curve25519PublicKey> = self
            .one_time_keys()
            .into_iter()
            .map(|(_, key)| key)
            .collect();

        let mut bundle = crate::prekey::PreKeyBundle::new(
            self.curve25519_key(),
            self.ed25519_key(),
            Ed25519Signature::from_slice(&[0u8; 64]).expect("64 bytes is a valid signature size"),
            one_time_keys,
        );
        bundle.signature = self.sign(bundle.signed_bytes());
        bundle
    }

    /// Generate new one-time pre-keys. Returns the result so callers can see
    /// how many keys were created.
    pub fn generate_one_time_keys(&mut self, count: usize) -> OneTimeKeyGenerationResult {
        self.account.generate_one_time_keys(count)
    }

    /// The unpublished one-time pre-keys of this identity, sorted by their
    /// base64 encoding for deterministic ordering.
    pub fn one_time_keys(&self) -> Vec<(KeyId, Curve25519PublicKey)> {
        let mut keys: Vec<_> = self.account.one_time_keys().into_iter().collect();
        keys.sort_by_key(|(_, key)| key.to_base64());
        keys
    }

    /// Number of one-time pre-keys currently stored locally.
    pub fn stored_one_time_key_count(&self) -> usize {
        self.account.stored_one_time_key_count()
    }

    /// Mark all currently unpublished one-time keys as published.
    pub fn mark_keys_as_published(&mut self) {
        self.account.mark_keys_as_published();
    }

    /// Serialize this identity (including all private key material) to JSON.
    ///
    /// The output is vodozemac's pickle format and is suitable for storing in
    /// SQLCipher.
    pub fn to_json(&self) -> Result<String, IdentityError> {
        let pickle = self.account.pickle();
        serde_json::to_string(&pickle).map_err(IdentityError::Serialize)
    }

    /// Load an identity previously stored with [`Identity::to_json`].
    pub fn from_json(json: &str) -> Result<Self, IdentityError> {
        let pickle: AccountPickle =
            serde_json::from_str(json).map_err(IdentityError::Deserialize)?;
        Ok(Self {
            account: Account::from_pickle(pickle),
        })
    }

    /// Internal access to the vodozemac account for session creation.
    pub(crate) fn account(&self) -> &Account {
        &self.account
    }

    /// Internal mutable access to the vodozemac account for session creation.
    pub(crate) fn account_mut(&mut self) -> &mut Account {
        &mut self.account
    }
}

impl Default for Identity {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_generation_produces_distinct_identities() {
        let alice = Identity::new();
        let bob = Identity::new();

        assert_ne!(alice.curve25519_key(), bob.curve25519_key());
        assert_ne!(alice.ed25519_key(), bob.ed25519_key());
    }

    #[test]
    fn peer_id_is_deterministic_for_the_same_public_key() {
        let identity = Identity::new();

        let id_a = Identity::peer_id_from_curve25519(&identity.curve25519_key());
        let id_b = Identity::peer_id_from_curve25519(&identity.curve25519_key());

        assert_eq!(id_a, id_b, "peer ID must be deterministic");
        assert_eq!(
            id_a.len(),
            PEER_ID_LENGTH,
            "peer ID must have the configured length"
        );
        assert_eq!(
            identity.peer_id(),
            id_a,
            "instance helper must agree with the free function"
        );
    }

    #[test]
    fn peer_ids_differ_between_distinct_keys() {
        let alice = Identity::new();
        let bob = Identity::new();

        assert_ne!(alice.peer_id(), bob.peer_id());
    }

    #[test]
    fn identity_json_roundtrip_preserves_keys_and_peer_id() {
        let identity = Identity::new();
        let json = identity.to_json().expect("serialization must succeed");

        let restored = Identity::from_json(&json).expect("deserialization must succeed");

        assert_eq!(restored.curve25519_key(), identity.curve25519_key());
        assert_eq!(restored.ed25519_key(), identity.ed25519_key());
        assert_eq!(restored.peer_id(), identity.peer_id());
        assert_eq!(restored.identity_keys(), identity.identity_keys());
    }

    #[test]
    fn identity_can_sign_and_verify() {
        let identity = Identity::new();
        let message = b"operation ghost";

        let signature = identity.sign(message);
        assert!(identity.verify(message, &signature));

        let other = Identity::new();
        assert!(
            !other.verify(message, &signature),
            "a different key must not verify"
        );
    }

    #[test]
    fn signed_hello_round_trip_verifies() {
        let identity = Identity::new();
        let hello = identity.signed_hello();

        assert_eq!(hello.peer_id, identity.peer_id());
        assert_eq!(hello.curve25519_key, identity.curve25519_key().to_base64());
        assert_eq!(hello.ed25519_key, identity.ed25519_key().to_base64());
        assert!(
            Identity::verify_signed_hello(&hello).is_ok(),
            "a freshly signed hello must verify"
        );
    }

    #[test]
    fn signed_hello_json_roundtrip_preserves_fields() {
        let identity = Identity::new();
        let hello = identity.signed_hello();

        let json = serde_json::to_string(&hello).expect("serialization must succeed");
        let restored: SignedHello =
            serde_json::from_str(&json).expect("deserialization must succeed");

        assert_eq!(restored, hello);
        assert!(
            Identity::verify_signed_hello(&restored).is_ok(),
            "a hello restored from JSON must still verify"
        );
    }

    #[test]
    fn signed_hello_rejects_wrong_signature() {
        let identity = Identity::new();
        let mut hello = identity.signed_hello();

        // A signature produced by a different identity must not verify.
        let other = Identity::new();
        hello.signature = other.sign(hello.peer_id.as_bytes()).to_base64();

        assert_eq!(
            Identity::verify_signed_hello(&hello),
            Err(HelloError::InvalidSignature)
        );
    }

    #[test]
    fn signed_hello_rejects_wrong_peer_id() {
        let identity = Identity::new();
        let mut hello = identity.signed_hello();

        // Rebind the hello to a peer id the curve25519 key does not imply.
        let other = Identity::new();
        hello.peer_id = other.peer_id();

        assert_eq!(
            Identity::verify_signed_hello(&hello),
            Err(HelloError::InvalidPeerId)
        );
    }

    #[test]
    fn signed_hello_rejects_tampered_curve_key() {
        let identity = Identity::new();
        let mut hello = identity.signed_hello();

        // Swapping in another identity's curve key changes the derived peer id.
        let other = Identity::new();
        hello.curve25519_key = other.curve25519_key().to_base64();

        assert_eq!(
            Identity::verify_signed_hello(&hello),
            Err(HelloError::InvalidPeerId)
        );
    }

    #[test]
    fn signed_hello_rejects_malformed_keys() {
        let identity = Identity::new();

        let mut bad_curve = identity.signed_hello();
        bad_curve.curve25519_key = "not-base64!".into();
        assert_eq!(
            Identity::verify_signed_hello(&bad_curve),
            Err(HelloError::InvalidKey)
        );

        let mut bad_ed = identity.signed_hello();
        bad_ed.ed25519_key = "not-base64!".into();
        assert_eq!(
            Identity::verify_signed_hello(&bad_ed),
            Err(HelloError::InvalidKey)
        );
    }
}
