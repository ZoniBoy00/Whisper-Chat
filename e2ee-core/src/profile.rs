//! Username and public profile helpers.
//!
//! A username is a unique, lowercase alias (`[a-z0-9_]{3,32}`) bound to a
//! peer's X25519 identity key by an Ed25519 signature. The relay stores the
//! signature and re-verifies it on every registration, so a compromised relay
//! cannot reassign usernames, squat reserved aliases or inject its own keys.
//!
//! The signed binding is deliberately self-contained: the canonical bytes
//! cover the username and the public curve key, so a client only needs its
//! own [`Identity`] to register, and the relay needs nothing but the peer's
//! stored public keys to verify.

use vodozemac::{Curve25519PublicKey, Ed25519PublicKey, Ed25519Signature};

use crate::Identity;

/// Minimum username length in characters.
pub const USERNAME_MIN_LENGTH: usize = 3;
/// Maximum username length in characters.
pub const USERNAME_MAX_LENGTH: usize = 32;
/// Separator between the username bytes and the curve key in the canonical
/// signing bytes. It makes the encoding of a variable-length username
/// followed by a fixed-length key unambiguous.
pub const CANONICAL_SEPARATOR: u8 = 0x00;

/// Usernames that can never be registered (service-wide aliases).
pub const RESERVED_USERNAMES: &[&str] = &["admin", "whisper", "support", "mod", "system", "root"];

/// Whether `username` is a valid, registerable username.
///
/// A username must be 3-32 characters long, contain only ASCII lowercase
/// letters, digits and underscores, and not be a reserved alias.
pub fn validate_username(username: &str) -> bool {
    if username.len() < USERNAME_MIN_LENGTH || username.len() > USERNAME_MAX_LENGTH {
        return false;
    }
    if !username
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return false;
    }
    !RESERVED_USERNAMES.contains(&username)
}

/// The canonical bytes signed when binding a username to a public key:
/// `username_utf8_bytes || 0x00 || curve25519_public_key_raw (32 bytes)`.
///
/// The layout is fixed so signatures are portable across clients and
/// implementations.
pub fn canonical_bytes(username: &str, curve25519_key: &Curve25519PublicKey) -> Vec<u8> {
    let mut out = Vec::with_capacity(username.len() + 1 + 32);
    out.extend_from_slice(username.as_bytes());
    out.push(CANONICAL_SEPARATOR);
    out.extend_from_slice(curve25519_key.as_bytes());
    out
}

/// Sign the canonical username binding with the identity's Ed25519 key.
pub fn sign_username(identity: &Identity, username: &str) -> Ed25519Signature {
    identity.sign(canonical_bytes(username, &identity.curve25519_key()))
}

/// Verify an Ed25519 signature over the canonical username binding.
///
/// Returns true only when `sig` was produced by `ed_key` over
/// `canonical_bytes(username, curve)`. Any tampering with the username, the
/// curve key or the signature itself fails verification.
pub fn verify_username_signature(
    username: &str,
    curve: &Curve25519PublicKey,
    ed_key: &Ed25519PublicKey,
    sig: &Ed25519Signature,
) -> bool {
    ed_key
        .verify(&canonical_bytes(username, curve), sig)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Identity;

    #[test]
    fn validate_username_accepts_valid_names() {
        for name in [
            "alice",
            "bob_1",
            "x_0_9",
            "a_b_c",
            "u2345678901234567890123456789012",
        ] {
            assert!(validate_username(name), "{name} should be valid");
        }
    }

    #[test]
    fn validate_username_rejects_wrong_length() {
        assert!(!validate_username("ab"), "two characters is too short");
        assert!(!validate_username("a"), "one character is too short");
        assert!(!validate_username(""), "empty is not a username");
        let thirty_three = "a".repeat(USERNAME_MAX_LENGTH + 1);
        assert!(
            !validate_username(&thirty_three),
            "33 characters is too long"
        );
    }

    #[test]
    fn validate_username_rejects_invalid_characters() {
        for name in [
            "Alice",
            "ALICE",
            "al!ce",
            "al-ice",
            "al ice",
            "al.ice",
            "alice😀",
            "äbä",
        ] {
            assert!(!validate_username(name), "{name} must be rejected");
        }
    }

    #[test]
    fn validate_username_rejects_reserved_names() {
        for name in ["admin", "whisper", "support", "mod", "system", "root"] {
            assert!(!validate_username(name), "{name} is reserved");
        }
    }

    #[test]
    fn canonical_bytes_are_deterministic_and_well_formed() {
        let identity = Identity::new();
        let curve = identity.curve25519_key();

        let a = canonical_bytes("alice", &curve);
        let b = canonical_bytes("alice", &curve);
        assert_eq!(a, b, "canonical bytes must be deterministic");

        assert_eq!(a.len(), 5 + 1 + 32, "username || 0x00 || 32-byte key");
        assert_eq!(&a[..5], b"alice");
        assert_eq!(a[5], CANONICAL_SEPARATOR);
        assert_eq!(&a[6..], curve.as_bytes());
    }

    #[test]
    fn canonical_bytes_differ_across_username_or_key() {
        let identity = Identity::new();
        let curve = identity.curve25519_key();
        let other = Identity::new();

        let base = canonical_bytes("alice", &curve);
        assert_ne!(canonical_bytes("bob", &curve), base);
        assert_ne!(canonical_bytes("alice", &other.curve25519_key()), base);
    }

    #[test]
    fn username_signature_roundtrip_verifies() {
        let identity = Identity::new();
        let sig = sign_username(&identity, "alice");

        assert!(verify_username_signature(
            "alice",
            &identity.curve25519_key(),
            &identity.ed25519_key(),
            &sig,
        ));
    }

    #[test]
    fn username_signature_rejects_signature_from_another_key() {
        let alice = Identity::new();
        let mallory = Identity::new();

        // Mallory signs for the same username with her own key.
        let forged = sign_username(&mallory, "alice");
        assert!(!verify_username_signature(
            "alice",
            &alice.curve25519_key(),
            &alice.ed25519_key(),
            &forged,
        ));
    }

    #[test]
    fn username_signature_rejects_wrong_username() {
        let alice = Identity::new();
        let sig = sign_username(&alice, "alice");

        assert!(!verify_username_signature(
            "bob",
            &alice.curve25519_key(),
            &alice.ed25519_key(),
            &sig,
        ));
    }

    #[test]
    fn username_signature_rejects_wrong_curve_key() {
        let alice = Identity::new();
        let sig = sign_username(&alice, "alice");

        let other = Identity::new();
        assert!(!verify_username_signature(
            "alice",
            &other.curve25519_key(),
            &alice.ed25519_key(),
            &sig,
        ));
    }

    #[test]
    fn username_signature_rejects_wrong_ed_key() {
        let alice = Identity::new();
        let sig = sign_username(&alice, "alice");

        let other = Identity::new();
        assert!(!verify_username_signature(
            "alice",
            &alice.curve25519_key(),
            &other.ed25519_key(),
            &sig,
        ));
    }
}
