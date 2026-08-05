//! Pre-key bundle publishing and fetching.
//!
//! Peers publish their public X3DH pre-key bundle (`publish_prekeys`) so other
//! peers can start an encrypted session; fetching another peer's bundle
//! (`fetch_prekeys`) is a public directory lookup. Bundles are verified
//! (Ed25519 signature via [`PreKeyBundle::ensure_valid`]) and bound to the
//! authenticated peer before being persisted. Pre-key traffic is rate limited
//! per source IP under the `prekey:<ip>` bucket.

use super::*;

use e2ee_core::prekey::PreKeyBundle;
use e2ee_core::Identity;

impl Relay {
    /// Publish a peer's pre-key bundle so other peers can fetch it for the
    /// X3DH handshake. The bundle is only accepted when:
    /// 1. its Ed25519 signature verifies over the identity and one-time keys
    ///    (`ensure_valid`), and
    /// 2. the identity key fingerprints to the authenticated peer ID.
    ///
    /// Pre-key traffic is rate limited per source IP under the `prekey:<ip>`
    /// bucket (see [`crate::relay::ratelimit::RateLimiter`]).
    pub(crate) async fn publish_prekeys(&self, peer_id: &str, ip: &str, bundle: PreKeyBundle) {
        if !self.inner.limiter.try_take(&format!("prekey:{ip}")) {
            tracing::warn!(ip = %ip, "pre-key rate limit exceeded");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "rate_limited".into(),
                    },
                )
                .await;
            return;
        }

        if let Err(err) = bundle.ensure_valid() {
            tracing::warn!(peer = %peer_id, "rejecting invalid pre-key bundle: {err}");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "invalid_bundle".into(),
                    },
                )
                .await;
            return;
        }

        let derived = Identity::peer_id_from_curve25519(&bundle.identity_key);
        if derived != peer_id {
            tracing::warn!(peer = %peer_id, derived = %derived, "pre-key bundle identity mismatch");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "identity_mismatch".into(),
                    },
                )
                .await;
            return;
        }

        match serde_json::to_string(&bundle) {
            Ok(json) => match self.inner.store.set_prekeys(peer_id, &json, unix_now()) {
                Ok(()) => {
                    let _ = self.send(peer_id, ServerMessage::PrekeysPublished).await;
                }
                Err(err) => {
                    tracing::error!(peer = %peer_id, "failed to persist pre-key bundle: {err}");
                }
            },
            Err(err) => {
                tracing::error!(peer = %peer_id, "failed to serialize pre-key bundle: {err}")
            }
        }
    }

    /// Return the pre-key bundle another peer published, or `no_prekeys` when
    /// none is stored. Pre-key fetches are rate limited per source IP under the
    /// `prekey:<ip>` bucket like publishing.
    ///
    /// Contact gate: a bundle is only disclosed to an ACCEPTED contact (or the
    /// owner fetching their own bundle). This closes the directory-lookup side
    /// of the anti-spam boundary — a stranger can never harvest another peer's
    /// public key material.
    pub(crate) async fn fetch_prekeys(&self, peer_id: &str, ip: &str, target: &str) {
        if !self.inner.limiter.try_take(&format!("prekey:{ip}")) {
            tracing::warn!(ip = %ip, "pre-key rate limit exceeded");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "rate_limited".into(),
                    },
                )
                .await;
            return;
        }

        if target != peer_id && !self.inner.store.are_contacts(peer_id, target) {
            tracing::warn!(peer = %peer_id, target = %target, "pre-key fetch between non-contacts refused");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_contacts".into(),
                    },
                )
                .await;
            return;
        }

        match self.inner.store.get_prekeys(target) {
            Some(json) => match serde_json::from_str::<PreKeyBundle>(&json) {
                Ok(bundle) => {
                    let display_name = self.inner.store.get_display_name(target);
                    let _ = self
                        .send(
                            peer_id,
                            ServerMessage::Prekeys {
                                bundle: Box::new(bundle),
                                display_name,
                            },
                        )
                        .await;
                }
                Err(err) => {
                    tracing::error!(peer = %target, "stored pre-key bundle is corrupt: {err}");
                }
            },
            None => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "no_prekeys".into(),
                        },
                    )
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::test_utils::read_reply;

    #[tokio::test]
    async fn publish_and_fetch_prekeys_roundtrip_preserves_bundle() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let mut identity = Identity::new();
        let peer_id = identity.peer_id();
        let bundle = identity.pre_key_bundle(3);

        relay
            .publish_prekeys(&peer_id, "127.0.0.1", bundle.clone())
            .await;
        let json = relay
            .inner
            .store
            .get_prekeys(&peer_id)
            .expect("bundle must be persisted");
        let restored: PreKeyBundle =
            serde_json::from_str(&json).expect("stored bundle must deserialize");
        assert_eq!(restored, bundle);
    }

    #[tokio::test]
    async fn publish_prekeys_rejects_identity_mismatch() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let owner = Identity::new();
        let peer_id = owner.peer_id();
        // A valid bundle owned by a different identity must be rejected: its
        // identity key fingerprints to the other peer, not to `peer_id`.
        let mut other = Identity::new();
        let foreign = other.pre_key_bundle(3);

        relay.publish_prekeys(&peer_id, "127.0.0.1", foreign).await;
        assert_eq!(relay.inner.store.get_prekeys(&peer_id), None);
    }

    #[tokio::test]
    async fn publish_prekeys_rejects_invalid_bundle() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let mut identity = Identity::new();
        let peer_id = identity.peer_id();
        let mut bundle = identity.pre_key_bundle(2);
        // Swapping a one-time key invalidates the signature.
        bundle.one_time_keys[0] = Identity::new().curve25519_key();

        relay.publish_prekeys(&peer_id, "127.0.0.1", bundle).await;
        assert_eq!(relay.inner.store.get_prekeys(&peer_id), None);
    }

    #[tokio::test]
    async fn fetch_prekeys_reply_includes_display_name() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let mut owner = Identity::new();
        let owner_id = owner.peer_id();
        relay
            .inner
            .store
            .register_user_with_keys(
                &owner_id,
                &owner.curve25519_key().to_base64(),
                &owner.ed25519_key().to_base64(),
                unix_now(),
            )
            .unwrap();
        relay
            .inner
            .store
            .set_display_name(&owner_id, "Test Alice")
            .unwrap();
        relay
            .publish_prekeys(&owner_id, "127.0.0.1", owner.pre_key_bundle(2))
            .await;

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert("requester".into(), out_tx);
        // A pre-key fetch is a contact-gated lookup: the requester must be an
        // accepted contact of the owner.
        relay
            .inner
            .store
            .upsert_friend_request("requester", &owner_id)
            .unwrap();
        relay
            .inner
            .store
            .accept_friend("requester", &owner_id)
            .unwrap();

        relay
            .fetch_prekeys("requester", "127.0.0.1", &owner_id)
            .await;
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["type"].as_str(), Some("prekeys"));
        assert_eq!(reply["display_name"].as_str(), Some("Test Alice"));
    }
}
