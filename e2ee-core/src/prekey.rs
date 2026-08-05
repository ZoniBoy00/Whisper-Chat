//! Pre-key bundles for the X3DH handshake.
//!
//! A [`PreKeyBundle`] is the set of public keys a client publishes to the
//! relay so that other clients can establish an encrypted session with it:
//!
//! - the X25519 identity key,
//! - the Ed25519 signing key plus a signature over the identity and one-time
//!   keys, and
//! - a list of one-time pre-keys.
//!
//! # Note on the signed pre-key
//!
//! vodozemac's Olm ratchet derives the X3DH shared secret from the recipient's
//! long-term identity key and a single one-time key; it has no Signal-style
//! signed pre-key. To preserve bundle authenticity we instead sign the identity
//! key together with all one-time keys using the Ed25519 signing key, and the
//! initiator verifies that signature before creating a session. This makes
//! tampering with the bundle (for example swapping in attacker-controlled
//! one-time keys) detectable.
//!
//! Serialization uses unpadded base64 for every key field.

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use vodozemac::{Curve25519PublicKey, Ed25519PublicKey, Ed25519Signature};

/// The current bundle format version.
pub const BUNDLE_VERSION: u8 = 1;

/// Errors that can occur while working with a [`PreKeyBundle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreKeyBundleError {
    /// The bundle's signature could not be verified.
    InvalidSignature,
    /// The bundle does not contain any one-time keys.
    NoOneTimeKeys,
    /// A base64-encoded key field inside the bundle was malformed.
    InvalidKey,
}

impl fmt::Display for PreKeyBundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSignature => write!(f, "the pre-key bundle signature is invalid"),
            Self::NoOneTimeKeys => write!(f, "the pre-key bundle has no one-time keys"),
            Self::InvalidKey => write!(f, "the pre-key bundle contains a malformed key"),
        }
    }
}

impl std::error::Error for PreKeyBundleError {}

/// The public key material one participant publishes for the X3DH handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreKeyBundle {
    /// The bundle format version, currently [`BUNDLE_VERSION`].
    pub version: u8,
    /// The X25519 identity key of the bundle owner.
    pub identity_key: Curve25519PublicKey,
    /// The Ed25519 public key used to sign this bundle.
    pub signing_key: Ed25519PublicKey,
    /// Ed25519 signature over the identity key and all one-time keys.
    pub signature: Ed25519Signature,
    /// One-time pre-keys that can be consumed during the handshake.
    pub one_time_keys: Vec<Curve25519PublicKey>,
}

impl PreKeyBundle {
    /// Create a new version-1 bundle.
    pub fn new(
        identity_key: Curve25519PublicKey,
        signing_key: Ed25519PublicKey,
        signature: Ed25519Signature,
        one_time_keys: Vec<Curve25519PublicKey>,
    ) -> Self {
        Self {
            version: BUNDLE_VERSION,
            identity_key,
            signing_key,
            signature,
            one_time_keys,
        }
    }

    /// The canonical bytes that are signed by [`PreKeyBundle::signature`].
    ///
    /// The identity key bytes are followed by every one-time key in ascending
    /// base64 order so the signature is deterministic.
    pub fn signed_bytes(&self) -> Vec<u8> {
        let mut message = Vec::with_capacity(32 * (1 + self.one_time_keys.len()));
        message.extend_from_slice(self.identity_key.as_bytes());
        for key in &self.one_time_keys {
            message.extend_from_slice(key.as_bytes());
        }
        message
    }

    /// Check whether the bundle's signature is valid.
    pub fn verify(&self) -> bool {
        self.signing_key
            .verify(&self.signed_bytes(), &self.signature)
            .is_ok()
    }

    /// Verify the bundle signature and fail with [`PreKeyBundleError`] on any
    /// problem.
    pub fn ensure_valid(&self) -> Result<(), PreKeyBundleError> {
        if !self.verify() {
            Err(PreKeyBundleError::InvalidSignature)
        } else if self.one_time_keys.is_empty() {
            Err(PreKeyBundleError::NoOneTimeKeys)
        } else {
            Ok(())
        }
    }
}

impl Serialize for PreKeyBundle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        PreKeyBundleSerde::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PreKeyBundle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = PreKeyBundleSerde::deserialize(deserializer)?;

        let identity_key = Curve25519PublicKey::from_base64(&raw.identity_key)
            .map_err(|_| de::Error::custom(PreKeyBundleError::InvalidKey))?;
        let signing_key = Ed25519PublicKey::from_base64(&raw.signing_key)
            .map_err(|_| de::Error::custom(PreKeyBundleError::InvalidKey))?;
        let signature = Ed25519Signature::from_base64(&raw.signature)
            .map_err(|_| de::Error::custom(PreKeyBundleError::InvalidKey))?;
        let one_time_keys = raw
            .one_time_keys
            .into_iter()
            .map(|k| {
                Curve25519PublicKey::from_base64(&k)
                    .map_err(|_| de::Error::custom(PreKeyBundleError::InvalidKey))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            version: raw.version,
            identity_key,
            signing_key,
            signature,
            one_time_keys,
        })
    }
}

/// The base64-encoded wire representation of a [`PreKeyBundle`].
#[derive(Serialize, Deserialize)]
struct PreKeyBundleSerde {
    version: u8,
    identity_key: String,
    signing_key: String,
    signature: String,
    one_time_keys: Vec<String>,
}

impl From<&PreKeyBundle> for PreKeyBundleSerde {
    fn from(bundle: &PreKeyBundle) -> Self {
        Self {
            version: bundle.version,
            identity_key: bundle.identity_key.to_base64(),
            signing_key: bundle.signing_key.to_base64(),
            signature: bundle.signature.to_base64(),
            one_time_keys: bundle.one_time_keys.iter().map(|k| k.to_base64()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    fn bundle_for(identity: &mut Identity, count: usize) -> PreKeyBundle {
        identity.pre_key_bundle(count)
    }

    #[test]
    fn bundle_json_roundtrip_preserves_fields() {
        let mut identity = Identity::new();
        let original = bundle_for(&mut identity, 5);

        let json = serde_json::to_string(&original).expect("serialization must succeed");
        let restored: PreKeyBundle =
            serde_json::from_str(&json).expect("deserialization must succeed");

        assert_eq!(restored, original, "round-trip must preserve every field");
        assert_eq!(restored.one_time_keys.len(), 5);
    }

    #[test]
    fn valid_bundle_verifies() {
        let mut identity = Identity::new();
        let bundle = bundle_for(&mut identity, 3);

        assert!(bundle.verify(), "a correctly signed bundle must verify");
        assert!(bundle.ensure_valid().is_ok());
    }

    #[test]
    fn tampered_bundle_fails_verification() {
        let mut identity = Identity::new();
        let mut bundle = bundle_for(&mut identity, 3);

        let foreign_key = Identity::new().curve25519_key();
        bundle.one_time_keys[0] = foreign_key;

        assert!(
            !bundle.verify(),
            "swapping a one-time key must invalidate the signature"
        );
        assert!(matches!(
            bundle.ensure_valid(),
            Err(PreKeyBundleError::InvalidSignature)
        ));
    }

    #[test]
    fn bundle_without_one_time_keys_is_rejected() {
        let mut identity = Identity::new();
        let bundle = bundle_for(&mut identity, 0);

        assert!(bundle.verify(), "the signature itself is still valid");
        assert!(matches!(
            bundle.ensure_valid(),
            Err(PreKeyBundleError::NoOneTimeKeys)
        ));
    }
}
