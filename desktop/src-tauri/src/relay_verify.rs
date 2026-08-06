//! Safety numbers, contact verification and invite links.
//!
//! Safety numbers are derived purely from the two identities' public X25519
//! keys (see [`e2ee_core::safety_number`]), so neither party needs the relay
//! to compute them — the peer's key is learned and persisted from pre-key
//! bundles, handshakes or profile lookups. Verification ("mark as verified")
//! is a purely local flag on the contact row.
//!
//! Invite links are `whisper://invite?peer=..&name=..&user=..` URIs built from
//! our own identity and profile; the UI shares them via the clipboard.

use super::*;
use vodozemac::Curve25519PublicKey;

/// Safety number info for one contact, returned to the UI.
#[derive(Debug, Clone, Serialize)]
pub struct SafetyNumberInfo {
    /// The 60-digit grouped safety number shared with the peer.
    pub safety_number: String,
    /// The compact 8-hex tag, for quick verbal comparison.
    pub short: String,
    /// Whether we have marked this contact as verified.
    pub verified: bool,
}

impl RelayClient {
    /// Build a `whisper://invite` link for this identity, including our
    /// display name and username as hints when they are registered.
    pub fn get_invite_link(&self) -> Result<String, RelayError> {
        let my_peer_id = self.my_peer_id()?;
        let (name, username) = {
            let profiles = read_guard(&self.inner.profiles)?;
            (
                profiles.my_display_name.clone(),
                profiles.my_username.clone(),
            )
        };
        Ok(e2ee_core::build_invite_link(
            &my_peer_id,
            name.as_deref(),
            username.as_deref(),
        ))
    }

    /// Compute the safety number shared with `peer_id` and our verification
    /// state for them. Fails with [`RelayError::PeerKeyUnknown`] until the
    /// peer's identity key has been learned.
    pub fn get_safety_number(&self, peer_id: &str) -> Result<SafetyNumberInfo, RelayError> {
        let their_key = self.peer_curve25519_key(peer_id)?;
        let identity = mutex_guard(&self.inner.identity)?;
        let identity = identity.as_ref().ok_or(RelayError::NoIdentity)?;
        let my_key = identity.curve25519_key();
        Ok(SafetyNumberInfo {
            safety_number: e2ee_core::safety_number(&my_key, &their_key),
            short: e2ee_core::short_safety_number(&my_key, &their_key),
            verified: self.contact_verified(peer_id)?,
        })
    }

    /// Set (or clear) the verified flag on a contact. Purely local: the relay
    /// never learns about verification.
    pub fn set_contact_verified(&self, peer_id: &str, verified: bool) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        let mut row = self
            .store_guard()?
            .as_ref()
            .ok_or(RelayError::StoreNotOpen)?
            .get_contact(peer_id)?
            .ok_or(RelayError::PeerKeyUnknown)?;
        row.verified = verified;
        self.store_guard()?
            .as_ref()
            .ok_or(RelayError::StoreNotOpen)?
            .upsert_contact(&row)?;
        Ok(())
    }

    /// Remember a peer's public identity key (from a pre-key bundle, handshake
    /// or profile) so safety numbers can be computed without a live relay
    /// round-trip. Best-effort: a missing contact row is created for the key.
    pub(crate) fn remember_peer_key(
        &self,
        peer_id: &str,
        key: Curve25519PublicKey,
    ) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        let mut row = match self
            .store_guard()?
            .as_ref()
            .ok_or(RelayError::StoreNotOpen)?
            .get_contact(peer_id)?
        {
            Some(row) => row,
            None => ContactRow {
                peer_id: peer_id.to_string(),
                display_name: None,
                username: None,
                avatar_url: None,
                last_seen: None,
                curve25519_key: None,
                verified: false,
            },
        };
        row.curve25519_key = Some(key.to_base64());
        self.store_guard()?
            .as_ref()
            .ok_or(RelayError::StoreNotOpen)?
            .upsert_contact(&row)?;
        Ok(())
    }

    /// The peer's stored public X25519 identity key, if we have learned it.
    fn peer_curve25519_key(&self, peer_id: &str) -> Result<Curve25519PublicKey, RelayError> {
        let key = match self.store_guard() {
            Ok(store_guard) => match store_guard.as_ref() {
                Some(store) => match store.get_contact(peer_id)? {
                    Some(row) => row.curve25519_key,
                    None => None,
                },
                None => None,
            },
            Err(_) => None,
        };
        let key = key.ok_or(RelayError::PeerKeyUnknown)?;
        Curve25519PublicKey::from_base64(&key).map_err(|_| RelayError::PeerKeyUnknown)
    }

    /// The stored verified flag for a contact (false when unknown).
    fn contact_verified(&self, peer_id: &str) -> Result<bool, RelayError> {
        match self.store_guard() {
            Ok(store_guard) => match store_guard.as_ref() {
                Some(store) => Ok(store
                    .get_contact(peer_id)?
                    .map(|row| row.verified)
                    .unwrap_or(false)),
                None => Ok(false),
            },
            Err(_) => Ok(false),
        }
    }
}
