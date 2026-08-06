//! Safety numbers and invite links.
//!
//! # Safety numbers
//!
//! A safety number is a human-readable fingerprint that lets two peers verify
//! they are talking to the right keys — Signal-style, without trusting the
//! relay. Both parties derive the *same* value from their two public X25519
//! identity keys; the keys are sorted before hashing so the result is
//! symmetric. The full form is a 60-digit number grouped in blocks of five
//! (for QR/visual comparison); the short form is a compact hex tag for quick
//! verbal comparison.
//!
//! The digest is SHA-256 from the `sha2` crate — no hand-rolled primitives.
//!
//! # Invite links
//!
//! An invite link shares a peer ID (plus optional profile hints) as a
//! `whisper://` URI: `whisper://invite?peer=<24-hex>&name=<..>&user=<..>`. The
//! recipient parses it and can add the peer before the first message.

use sha2::{Digest, Sha256};
use vodozemac::Curve25519PublicKey;

use crate::identity::PEER_ID_LENGTH;

/// How many 5-digit groups the full safety number contains (60 digits).
pub const SAFETY_NUMBER_GROUPS: usize = 12;
/// Digits per group in the full safety number.
pub const SAFETY_NUMBER_DIGITS_PER_GROUP: usize = 5;
/// Hex characters in the short safety number.
pub const SHORT_SAFETY_NUMBER_LENGTH: usize = 8;

/// Compute the 60-digit safety number shared by two identities.
///
/// The two public keys are sorted byte-wise before hashing, so both peers
/// derive the identical value. The digest's bytes are mapped onto decimal
/// digits (two per byte, ten's and one's position), truncated to 60 digits and
/// grouped in blocks of five for readability.
pub fn safety_number(my_key: &Curve25519PublicKey, their_key: &Curve25519PublicKey) -> String {
    let (a, b) = if my_key.as_bytes() <= their_key.as_bytes() {
        (my_key.as_bytes(), their_key.as_bytes())
    } else {
        (their_key.as_bytes(), my_key.as_bytes())
    };
    let mut hasher = Sha256::new();
    hasher.update(a);
    hasher.update(b);
    let digest = hasher.finalize();

    // Map digest bytes onto decimal digits (two per byte: ten's and one's
    // position, both reduced mod 10) until we have 60 digits.
    let mut digits = String::with_capacity(60);
    for byte in digest.iter() {
        if digits.len() >= 60 {
            break;
        }
        digits.push(char::from(b'0' + (byte / 10) % 10));
        if digits.len() < 60 {
            digits.push(char::from(b'0' + byte % 10));
        }
    }
    // Group in blocks of five for readability.
    let mut grouped = String::with_capacity(60 + SAFETY_NUMBER_GROUPS - 1);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && index % SAFETY_NUMBER_DIGITS_PER_GROUP == 0 {
            grouped.push(' ');
        }
        grouped.push(digit);
    }
    grouped
}

/// Compute the short (8 hex char) safety tag shared by two identities. Quick
/// to compare out loud; the full [`safety_number`] remains the authoritative
/// fingerprint.
pub fn short_safety_number(
    my_key: &Curve25519PublicKey,
    their_key: &Curve25519PublicKey,
) -> String {
    let (a, b) = if my_key.as_bytes() <= their_key.as_bytes() {
        (my_key.as_bytes(), their_key.as_bytes())
    } else {
        (their_key.as_bytes(), my_key.as_bytes())
    };
    let mut hasher = Sha256::new();
    hasher.update(a);
    hasher.update(b);
    let digest = hasher.finalize();
    let mut tag = String::with_capacity(SHORT_SAFETY_NUMBER_LENGTH);
    for byte in digest.iter().take(SHORT_SAFETY_NUMBER_LENGTH / 2) {
        tag.push_str(&format!("{byte:02x}"));
    }
    tag
}

/// A parsed `whisper://` invite link: the target peer plus optional profile
/// hints the sender chose to include.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteLink {
    /// The target's 24-hex peer ID (validated on parse).
    pub peer_id: String,
    /// The sender's display name, when they included it.
    pub display_name: Option<String>,
    /// The sender's registered username, when they included it.
    pub username: Option<String>,
}

/// Build a `whisper://invite` link for `peer_id`.
pub fn build_invite_link(
    peer_id: &str,
    display_name: Option<&str>,
    username: Option<&str>,
) -> String {
    let mut link = format!("whisper://invite?peer={peer_id}");
    if let Some(name) = display_name {
        if !name.is_empty() {
            link.push_str("&name=");
            link.push_str(&urlencode(name));
        }
    }
    if let Some(user) = username {
        if !user.is_empty() {
            link.push_str("&user=");
            link.push_str(&urlencode(user));
        }
    }
    link
}

/// Parse and validate a `whisper://` invite link. Returns `None` for anything
/// that is not a well-formed invite (wrong scheme, missing/invalid peer ID).
pub fn parse_invite_link(link: &str) -> Option<InviteLink> {
    let rest = link.strip_prefix("whisper://")?;
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, query),
        None => (rest, ""),
    };
    if path != "invite" {
        return None;
    }
    let mut peer_id = None;
    let mut display_name = None;
    let mut username = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        match key {
            "peer" => peer_id = Some(urldecode(value)),
            "name" => display_name = Some(urldecode(value)),
            "user" => username = Some(urldecode(value)),
            _ => {}
        }
    }
    let peer_id = peer_id?;
    if !is_valid_peer_id(&peer_id) {
        return None;
    }
    Some(InviteLink {
        peer_id,
        display_name: display_name.filter(|n| !n.is_empty()),
        username: username.filter(|u| !u.is_empty()),
    })
}

/// Whether `peer_id` looks like a valid Whisper peer ID (24 lowercase hex).
pub fn is_valid_peer_id(peer_id: &str) -> bool {
    peer_id.len() == PEER_ID_LENGTH && peer_id.chars().all(|c| c.is_ascii_hexdigit())
}

/// Minimal percent-encoding for the link query values (keeps names with
/// spaces/unicode intact on the wire).
fn urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Minimal percent-decoding for the link query values.
fn urldecode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_val(bytes[index + 1]), hex_val(bytes[index + 2]))
            {
                out.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    #[test]
    fn safety_number_is_symmetric_for_both_parties() {
        let alice = Identity::new();
        let bob = Identity::new();
        let alice_view = safety_number(&alice.curve25519_key(), &bob.curve25519_key());
        let bob_view = safety_number(&bob.curve25519_key(), &alice.curve25519_key());
        assert_eq!(
            alice_view, bob_view,
            "both parties must derive the same value"
        );
    }

    #[test]
    fn safety_number_is_deterministic() {
        let alice = Identity::new();
        let bob = Identity::new();
        let first = safety_number(&alice.curve25519_key(), &bob.curve25519_key());
        let second = safety_number(&alice.curve25519_key(), &bob.curve25519_key());
        assert_eq!(first, second);
    }

    #[test]
    fn safety_number_differs_between_distinct_peers() {
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let with_bob = safety_number(&alice.curve25519_key(), &bob.curve25519_key());
        let with_carol = safety_number(&alice.curve25519_key(), &carol.curve25519_key());
        assert_ne!(with_bob, with_carol);
    }

    #[test]
    fn safety_number_has_expected_shape() {
        let alice = Identity::new();
        let bob = Identity::new();
        let number = safety_number(&alice.curve25519_key(), &bob.curve25519_key());
        let groups: Vec<&str> = number.split(' ').collect();
        assert_eq!(groups.len(), SAFETY_NUMBER_GROUPS);
        for group in &groups {
            assert_eq!(group.len(), SAFETY_NUMBER_DIGITS_PER_GROUP);
            assert!(group.chars().all(|c| c.is_ascii_digit()));
        }
        assert_eq!(number.len(), 60 + SAFETY_NUMBER_GROUPS - 1);
    }

    #[test]
    fn short_safety_number_is_compact_and_symmetric() {
        let alice = Identity::new();
        let bob = Identity::new();
        let a = short_safety_number(&alice.curve25519_key(), &bob.curve25519_key());
        let b = short_safety_number(&bob.curve25519_key(), &alice.curve25519_key());
        assert_eq!(a, b);
        assert_eq!(a.len(), SHORT_SAFETY_NUMBER_LENGTH);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        let carol = Identity::new();
        assert_ne!(
            a,
            short_safety_number(&alice.curve25519_key(), &carol.curve25519_key())
        );
    }

    #[test]
    fn invite_link_roundtrips_with_all_fields() {
        let link = build_invite_link("a1b2c3d4e5f6a7b8c9d0e1f2", Some("Tersika"), Some("tersika"));
        let parsed = parse_invite_link(&link).expect("parse");
        assert_eq!(parsed.peer_id, "a1b2c3d4e5f6a7b8c9d0e1f2");
        assert_eq!(parsed.display_name.as_deref(), Some("Tersika"));
        assert_eq!(parsed.username.as_deref(), Some("tersika"));
    }

    #[test]
    fn invite_link_roundtrips_with_unicode_and_spaces() {
        let link = build_invite_link("a1b2c3d4e5f6a7b8c9d0e1f2", Some("Matti Meikäläinen"), None);
        let parsed = parse_invite_link(&link).expect("parse");
        assert_eq!(parsed.display_name.as_deref(), Some("Matti Meikäläinen"));
        assert_eq!(parsed.username, None);
    }

    #[test]
    fn invite_link_minimal_form_parses() {
        let parsed =
            parse_invite_link("whisper://invite?peer=a1b2c3d4e5f6a7b8c9d0e1f2").expect("parse");
        assert_eq!(parsed.peer_id, "a1b2c3d4e5f6a7b8c9d0e1f2");
        assert_eq!(parsed.display_name, None);
        assert_eq!(parsed.username, None);
    }

    #[test]
    fn invite_link_rejects_bad_input() {
        assert!(parse_invite_link("").is_none());
        assert!(parse_invite_link("https://example.com").is_none());
        assert!(parse_invite_link("whisper://other?peer=abc").is_none());
        assert!(parse_invite_link("whisper://invite?name=only").is_none());
        assert!(parse_invite_link("whisper://invite?peer=tooshort").is_none());
        assert!(parse_invite_link("whisper://invite?peer=ZZZZZZZZZZZZZZZZZZZZZZZZ").is_none());
    }

    #[test]
    fn peer_id_validation() {
        assert!(is_valid_peer_id("a1b2c3d4e5f6a7b8c9d0e1f2"));
        assert!(!is_valid_peer_id(""));
        assert!(!is_valid_peer_id("short"));
        assert!(!is_valid_peer_id("ZZZZZZZZZZZZZZZZZZZZZZZZ"));
    }
}
