//! Public profiles, display names and avatars.
//!
//! The public directory feature: registering a signed username binding
//! (`register_profile`), prefix-searching usernames/peer IDs (`search_users`),
//! fetching a peer's public profile (`get_profile`) and updating the caller's
//! public display name (`update_profile`). Avatar blobs are decoded,
//! size-checked and content-addressed into the media directory here. Profile
//! operations draw from the per-IP `profile:<ip>` rate bucket.

use std::path::Path;

use base64::{engine::general_purpose::STANDARD, Engine};
use e2ee_core::profile::{validate_username, verify_username_signature};
use sha2::{Digest, Sha256};
use vodozemac::{Curve25519PublicKey, Ed25519PublicKey, Ed25519Signature};

use super::*;

/// Maximum size of an uploaded avatar blob, in bytes (2 MiB). The check runs
/// on the decoded blob so a client cannot smuggle more data than advertised.
const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;

impl Relay {
    /// Update the caller's public display name.
    ///
    /// Like pre-key traffic, profile updates are rate limited per source IP
    /// under the `profile:<ip>` bucket so a client cannot spam renames.
    /// Invalid names are rejected with `invalid_display_name` and leave any
    /// existing name untouched.
    pub(crate) async fn update_profile(&self, peer_id: &str, ip: &str, display_name: &str) {
        if !self.inner.limiter.try_take(&format!("profile:{ip}")) {
            tracing::warn!(ip = %ip, "profile rate limit exceeded");
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

        if !Self::is_valid_display_name(display_name) {
            tracing::warn!(peer = %peer_id, "rejecting invalid display name");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "invalid_display_name".into(),
                    },
                )
                .await;
            return;
        }

        match self.inner.store.set_display_name(peer_id, display_name) {
            Ok(()) => {
                let _ = self.send(peer_id, ServerMessage::ProfileUpdated).await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, "failed to persist display name: {err}");
            }
        }
    }

    /// Register a signed username binding for the authenticated peer.
    ///
    /// SECURITY: the username is bound to the peer's stored X25519 identity
    /// key by an Ed25519 signature over the canonical bytes
    /// (`username || 0x00 || curve_key_raw`). The signature is re-verified
    /// against the peer's stored public keys before anything is persisted, so
    /// a compromised relay cannot reassign usernames or squat reserved ones.
    ///
    /// The optional `avatar` (base64 image, ≤ 2 MiB) is stored on disk as
    /// `media/<sha256>.bin`; identical content hashes to the same blob, so
    /// re-uploads are idempotent. The blob is written (and the media directory
    /// created) before anything is persisted, and a write failure aborts the
    /// whole registration with `media_error` — the relay never stores an
    /// `avatar_hash` that has no blob behind it.
    ///
    /// Rate limiting: registration is throttled per source IP under the
    /// `profile:<ip>` bucket (default 5/hour; burst/refill overridable via
    /// `WHISPER_PROFILE_RATE_BURST` / `WHISPER_PROFILE_RATE_REFILL`).
    pub(crate) async fn register_profile(
        &self,
        peer_id: &str,
        ip: &str,
        username: &str,
        signature_b64: &str,
        display_name: Option<&str>,
        avatar_b64: Option<&str>,
    ) {
        // 1) Rate limit profile mutations per source IP.
        if !self
            .inner
            .profile_limiter
            .try_take(&format!("profile:{ip}"))
        {
            tracing::warn!(ip = %ip, "profile rate limit exceeded");
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

        // 2) Username shape validation (charset, length, reserved names).
        if !validate_username(username) {
            tracing::warn!(peer = %peer_id, "rejecting invalid username");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "invalid_username".into(),
                    },
                )
                .await;
            return;
        }

        if let Some(name) = display_name {
            if !Self::is_valid_display_name(name) {
                tracing::warn!(peer = %peer_id, "rejecting invalid display name");
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "invalid_display_name".into(),
                        },
                    )
                    .await;
                return;
            }
        }

        // 3) Decode and size-check the avatar early so nothing is persisted
        //    for a request that will be rejected later. The blob itself is
        //    only written to disk after the signature has been verified.
        let decoded_avatar = match avatar_b64 {
            Some(raw) => match Self::decode_avatar(raw) {
                Ok(bytes) => Some(bytes),
                Err(()) => {
                    let _ = self
                        .send(
                            peer_id,
                            ServerMessage::Error {
                                code: "invalid_avatar".into(),
                            },
                        )
                        .await;
                    return;
                }
            },
            None => None,
        };

        // 4) Signature verification: only the peer that owns the stored curve
        //    key can produce a valid binding. The relay is authenticated by
        //    the signed hello (handle_socket), so the peer's keys are present.
        let (curve_b64, ed_b64) = match self.inner.store.get_user_keys(peer_id) {
            Some(keys) => keys,
            None => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "no_profile".into(),
                        },
                    )
                    .await;
                return;
            }
        };
        let parsed = match Self::parse_binding(&curve_b64, &ed_b64, signature_b64) {
            Some(keys) => keys,
            None => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "bad_signature".into(),
                        },
                    )
                    .await;
                return;
            }
        };
        if !verify_username_signature(username, &parsed.0, &parsed.1, &parsed.2) {
            tracing::warn!(peer = %peer_id, username = %username, "username signature verification failed");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "bad_signature".into(),
                    },
                )
                .await;
            return;
        }

        // 5) Persist the avatar blob BEFORE the username binding, so a storage
        //    failure aborts the whole registration with a hard `media_error`
        //    and leaves nothing dangling: no username, no avatar hash pointing
        //    at a blob that is not on disk. The blob is content-addressed, so
        //    a write that is later orphaned (the username turns out to be
        //    taken) is harmless — nothing ever references it.
        let avatar_hash = match decoded_avatar {
            Some(bytes) => match Self::store_avatar(&self.inner.media_dir, &bytes) {
                Ok(hash) => Some(hash),
                Err(()) => {
                    let _ = self
                        .send(
                            peer_id,
                            ServerMessage::Error {
                                code: "media_error".into(),
                            },
                        )
                        .await;
                    return;
                }
            },
            None => None,
        };

        // 6) Uniqueness + persistence of the username binding.
        let now = unix_now();
        match self
            .inner
            .store
            .register_username(peer_id, username, signature_b64, now)
        {
            Err(crate::store::StoreError::UsernameTaken) => {
                tracing::warn!(peer = %peer_id, username = %username, "username already taken");
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "username_taken".into(),
                        },
                    )
                    .await;
                return;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, "failed to persist username: {err}");
                return;
            }
            Ok(()) => {}
        }

        // 7) Persist the profile extras (display name + avatar hash).
        if let Some(name) = display_name {
            if let Err(err) = self.inner.store.set_display_name(peer_id, name) {
                tracing::error!(peer = %peer_id, "failed to persist display name: {err}");
            }
        }
        if let Some(hash) = avatar_hash {
            if let Err(err) = self.inner.store.set_avatar_hash(peer_id, &hash) {
                tracing::error!(peer = %peer_id, "failed to persist avatar hash: {err}");
            }
        }

        let _ = self
            .send(
                peer_id,
                ServerMessage::ProfileRegistered {
                    username: username.to_string(),
                },
            )
            .await;
        tracing::debug!(peer = %peer_id, username = %username, "profile registered");
    }

    /// Prefix-search the public directory by username or peer ID.
    ///
    /// Results are capped at 25 entries (default 10). Like profile
    /// registration, search consumes the `profile:<ip>` rate bucket.
    pub(crate) async fn search_users(
        &self,
        peer_id: &str,
        ip: &str,
        query: &str,
        limit: Option<usize>,
    ) {
        if !self
            .inner
            .profile_limiter
            .try_take(&format!("profile:{ip}"))
        {
            tracing::warn!(ip = %ip, "profile rate limit exceeded");
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

        let query = query.trim();
        let limit = limit.unwrap_or(10).clamp(1, 25);
        let results = if query.is_empty() {
            Vec::new()
        } else {
            self.inner
                .store
                .search_users(query, limit)
                .into_iter()
                .map(|p| SearchResult {
                    username: p.username.unwrap_or_default(),
                    peer_id: p.peer_id,
                    display_name: p.display_name,
                    avatar_url: Self::avatar_url(p.avatar_hash.as_deref()),
                })
                .collect()
        };
        let results_count = results.len();
        let _ = self
            .send(peer_id, ServerMessage::UsersSearch { results })
            .await;
        tracing::debug!(
            peer = %peer_id,
            query = %query,
            results = results_count,
            "users searched"
        );
    }

    /// Fetch another peer's public profile by peer ID, or answer `no_profile`
    /// when the peer has never been seen by the relay. Directory lookups are
    /// rate limited per source IP like every other profile operation.
    pub(crate) async fn get_profile(&self, peer_id: &str, ip: &str, target: &str) {
        if !self
            .inner
            .profile_limiter
            .try_take(&format!("profile:{ip}"))
        {
            tracing::warn!(ip = %ip, "profile rate limit exceeded");
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

        match self.inner.store.get_profile(target) {
            Some(profile) => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Profile {
                            username: profile.username,
                            peer_id: profile.peer_id,
                            display_name: profile.display_name,
                            avatar_url: Self::avatar_url(profile.avatar_hash.as_deref()),
                            curve25519_key: profile.curve25519_key,
                        },
                    )
                    .await;
                tracing::debug!(peer = %peer_id, target = %target, "profile fetched");
            }
            None => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "no_profile".into(),
                        },
                    )
                    .await;
                tracing::debug!(peer = %peer_id, target = %target, "profile not found");
            }
        }
    }

    /// Map a stored avatar hash to the public URL the relay serves it under.
    fn avatar_url(avatar_hash: Option<&str>) -> Option<String> {
        avatar_hash.map(|h| format!("/media/{h}"))
    }

    /// Decode a base64 avatar blob and enforce the size bound. Returns `Err`
    /// when the input is not valid base64, empty or larger than
    /// [`MAX_AVATAR_BYTES`]. Shared by profile and group avatars.
    pub(crate) fn decode_avatar(raw: &str) -> Result<Vec<u8>, ()> {
        let bytes = STANDARD.decode(raw).map_err(|_| ())?;
        if bytes.is_empty() || bytes.len() > MAX_AVATAR_BYTES {
            return Err(());
        }
        Ok(bytes)
    }

    /// Write an avatar blob to `media/<sha256>.bin` and return the hex SHA-256
    /// used as its storage key. Content-addressed: identical blobs share one
    /// file, so re-uploads are idempotent. Shared by profile and group avatars.
    pub(crate) fn store_avatar(media_dir: &Path, bytes: &[u8]) -> Result<String, ()> {
        let digest = Sha256::digest(bytes);
        let hash = Self::hex_encode(&digest);
        let path = media_dir.join(format!("{hash}.bin"));
        if let Some(parent) = path.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                tracing::error!(path = %parent.display(), "failed to create media dir: {err}");
                return Err(());
            }
        }
        if let Err(err) = std::fs::write(&path, bytes) {
            tracing::error!(path = %path.display(), "failed to write avatar blob: {err}");
            return Err(());
        }
        Ok(hash)
    }

    /// Lowercase hex encoding of a byte slice (SHA-256 digests, peer IDs).
    pub(crate) fn hex_encode(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
        out
    }

    /// Parse the stored curve/ed25519 keys and the submitted signature into
    /// verifiable vodozemac types. `None` when any piece is malformed.
    fn parse_binding(
        curve_b64: &str,
        ed_b64: &str,
        sig_b64: &str,
    ) -> Option<(Curve25519PublicKey, Ed25519PublicKey, Ed25519Signature)> {
        let curve = Curve25519PublicKey::from_base64(curve_b64).ok()?;
        let ed = Ed25519PublicKey::from_base64(ed_b64).ok()?;
        let sig = Ed25519Signature::from_base64(sig_b64).ok()?;
        Some((curve, ed, sig))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::test_utils::{online_peer, read_reply, sign_username};

    #[tokio::test]
    async fn update_profile_persists_display_name() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let peer_id = "peer-profile".to_string();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert(peer_id.clone(), out_tx);

        relay
            .update_profile(&peer_id, "127.0.0.1", "Alice Prime")
            .await;
        assert_eq!(
            relay.inner.store.get_display_name(&peer_id).as_deref(),
            Some("Alice Prime")
        );
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["type"].as_str(), Some("profile_updated"));
    }

    #[tokio::test]
    async fn update_profile_rejects_invalid_display_name() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let peer_id = "peer-profile".to_string();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert(peer_id.clone(), out_tx);

        let too_long = "x".repeat(super::super::MAX_DISPLAY_NAME_CHARS + 1);
        relay.update_profile(&peer_id, "127.0.0.1", &too_long).await;
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("invalid_display_name"));
        assert_eq!(
            relay.inner.store.get_display_name(&peer_id),
            None,
            "a rejected name must not touch the stored profile"
        );
    }

    #[tokio::test]
    async fn update_profile_is_rate_limited() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 1.0, 0.0);
        let peer_id = "peer-profile".to_string();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert(peer_id.clone(), out_tx);

        relay.update_profile(&peer_id, "10.0.0.1", "First").await;
        relay.update_profile(&peer_id, "10.0.0.1", "Second").await;

        let first = read_reply(&mut out_rx);
        assert_eq!(first["type"].as_str(), Some("profile_updated"));
        let second = read_reply(&mut out_rx);
        assert_eq!(second["type"].as_str(), Some("error"));
        assert_eq!(second["code"].as_str(), Some("rate_limited"));
        assert_eq!(
            relay.inner.store.get_display_name(&peer_id).as_deref(),
            Some("First"),
            "the rejected rename must not overwrite the accepted one"
        );
    }

    #[tokio::test]
    async fn register_profile_then_get_profile_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                Some("Test Alice"),
                None,
            )
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("profile_registered"));
        assert_eq!(reply["username"].as_str(), Some("alice"));

        relay
            .register_profile(
                &bob.peer_id(),
                "127.0.0.1",
                "bob",
                &sign_username(&bob, "bob"),
                None,
                None,
            )
            .await;
        assert_eq!(
            read_reply(&mut bob_rx)["type"].as_str(),
            Some("profile_registered")
        );

        // Alice looks Bob up by peer ID and sees his full public profile.
        relay
            .get_profile(&alice.peer_id(), "127.0.0.1", &bob.peer_id())
            .await;
        let profile = read_reply(&mut alice_rx);
        assert_eq!(profile["type"].as_str(), Some("profile"));
        assert_eq!(profile["username"].as_str(), Some("bob"));
        assert_eq!(profile["peer_id"].as_str(), Some(bob.peer_id().as_str()));
        assert_eq!(
            profile["curve25519_key"].as_str(),
            Some(bob.curve25519_key().to_base64().as_str())
        );
        assert_eq!(profile["avatar_url"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn register_profile_rejects_username_signed_for_another_name() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        // The signature binds "bob", not the claimed "alice".
        let wrong = sign_username(&alice, "bob");
        relay
            .register_profile(&alice.peer_id(), "127.0.0.1", "alice", &wrong, None, None)
            .await;
        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("bad_signature"));
    }

    #[tokio::test]
    async fn register_profile_rejects_signature_from_another_key() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mallory = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        // Mallory signs for her own username; Alice claims it.
        let forged = sign_username(&mallory, "alice");
        relay
            .register_profile(&alice.peer_id(), "127.0.0.1", "alice", &forged, None, None)
            .await;
        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("bad_signature"));
        assert_eq!(
            relay
                .inner
                .store
                .get_profile(&alice.peer_id())
                .unwrap()
                .username,
            None
        );
    }

    #[tokio::test]
    async fn register_profile_rejects_reserved_username() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "admin",
                &sign_username(&alice, "admin"),
                None,
                None,
            )
            .await;
        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("invalid_username"));
    }

    #[tokio::test]
    async fn register_profile_rejects_invalid_username() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        // Uppercase is not part of the `[a-z0-9_]` charset.
        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "Alice",
                &sign_username(&alice, "Alice"),
                None,
                None,
            )
            .await;
        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("invalid_username"));
    }

    #[tokio::test]
    async fn register_profile_rejects_duplicate_username() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                None,
                None,
            )
            .await;
        assert_eq!(
            read_reply(&mut alice_rx)["type"].as_str(),
            Some("profile_registered")
        );

        // Bob's signature is valid — the uniqueness check is what rejects him.
        relay
            .register_profile(
                &bob.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&bob, "alice"),
                None,
                None,
            )
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("username_taken"));
        assert_eq!(
            relay
                .inner
                .store
                .get_profile(&bob.peer_id())
                .unwrap()
                .username,
            None,
            "the rejected registration must not be persisted"
        );
    }

    #[tokio::test]
    async fn register_profile_is_rate_limited() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 1.0, 0.0);
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        relay
            .register_profile(
                &alice.peer_id(),
                "10.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                None,
                None,
            )
            .await;
        assert_eq!(
            read_reply(&mut rx)["type"].as_str(),
            Some("profile_registered")
        );

        relay
            .register_profile(
                &alice.peer_id(),
                "10.0.0.1",
                "bob",
                &sign_username(&alice, "bob"),
                None,
                None,
            )
            .await;
        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("rate_limited"));
    }

    #[tokio::test]
    async fn register_profile_stores_avatar_blob() {
        let store = Store::open_in_memory().unwrap();
        let dir =
            std::env::temp_dir().join(format!("whisper-relay-media-test-{}", uuid::Uuid::new_v4()));
        let relay = Relay::with_parts(
            store,
            dir.clone(),
            RateLimiter::new(100.0, 0.0),
            RateLimiter::new(100.0, 0.0),
            RateLimiter::new(100.0, 0.0),
            RateLimiter::new(100.0, 0.0),
        );
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        let png: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d,
        ];
        let encoded = STANDARD.encode(png);

        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                None,
                Some(&encoded),
            )
            .await;
        assert_eq!(
            read_reply(&mut rx)["type"].as_str(),
            Some("profile_registered")
        );

        let digest = Sha256::digest(png);
        let hash = Relay::hex_encode(&digest);
        assert!(
            dir.join(format!("{hash}.bin")).exists(),
            "the avatar blob must be written to the media directory"
        );
        let profile = relay.inner.store.get_profile(&alice.peer_id()).unwrap();
        assert_eq!(profile.avatar_hash.as_deref(), Some(hash.as_str()));
        assert_eq!(
            relay
                .inner
                .store
                .get_avatar_hash(&alice.peer_id())
                .as_deref(),
            Some(hash.as_str())
        );

        // A re-upload of identical content is idempotent (same hash, one file).
        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                None,
                Some(&encoded),
            )
            .await;
        assert_eq!(
            read_reply(&mut rx)["type"].as_str(),
            Some("profile_registered")
        );
        assert_eq!(
            relay
                .inner
                .store
                .get_avatar_hash(&alice.peer_id())
                .as_deref(),
            Some(hash.as_str())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn register_profile_reports_media_error_when_avatar_write_fails() {
        let store = Store::open_in_memory().unwrap();
        let dir =
            std::env::temp_dir().join(format!("whisper-relay-media-fail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // A regular FILE squatting on the media path makes create_dir_all
        // fail, so the blob cannot be written.
        let media_dir = dir.join("media");
        std::fs::write(&media_dir, b"not a directory").unwrap();
        let relay = Relay::with_parts(
            store,
            media_dir,
            RateLimiter::new(100.0, 0.0),
            RateLimiter::new(100.0, 0.0),
            RateLimiter::new(100.0, 0.0),
            RateLimiter::new(100.0, 0.0),
        );
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        let png: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        let encoded = STANDARD.encode(png);
        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                None,
                Some(&encoded),
            )
            .await;

        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("media_error"));
        // Nothing must be persisted: a failed blob write aborts the whole
        // registration, so there is no username and no dangling avatar hash.
        let profile = relay.inner.store.get_profile(&alice.peer_id()).unwrap();
        assert_eq!(
            profile.username, None,
            "a failed avatar write must abort the registration"
        );
        assert_eq!(
            relay
                .inner
                .store
                .get_avatar_hash(&alice.peer_id())
                .as_deref(),
            None,
            "an avatar hash must never be stored without its blob"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn register_profile_rejects_oversized_avatar() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        let big = vec![0x00u8; MAX_AVATAR_BYTES + 1];
        let encoded = STANDARD.encode(&big);
        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                None,
                Some(&encoded),
            )
            .await;
        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("invalid_avatar"));
        assert_eq!(
            relay
                .inner
                .store
                .get_profile(&alice.peer_id())
                .unwrap()
                .username,
            None,
            "an oversized avatar must abort the whole registration"
        );
    }

    #[tokio::test]
    async fn search_users_returns_matching_profiles() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .register_profile(
                &alice.peer_id(),
                "127.0.0.1",
                "alice",
                &sign_username(&alice, "alice"),
                None,
                None,
            )
            .await;
        relay
            .register_profile(
                &bob.peer_id(),
                "127.0.0.1",
                "bob",
                &sign_username(&bob, "bob"),
                None,
                None,
            )
            .await;
        read_reply(&mut alice_rx);
        read_reply(&mut bob_rx);

        // Bob searches by username prefix.
        relay
            .search_users(&bob.peer_id(), "127.0.0.1", "ali", Some(10))
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("users_search"));
        let results = reply["results"]
            .as_array()
            .expect("results must be an array");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["username"].as_str(), Some("alice"));
        assert_eq!(
            results[0]["peer_id"].as_str(),
            Some(alice.peer_id().as_str())
        );

        // Alice searches by peer-ID prefix and finds Bob.
        let prefix = &bob.peer_id()[..8];
        relay
            .search_users(&alice.peer_id(), "127.0.0.1", prefix, None)
            .await;
        let reply = read_reply(&mut alice_rx);
        let results = reply["results"]
            .as_array()
            .expect("results must be an array");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["username"].as_str(), Some("bob"));
    }

    #[tokio::test]
    async fn get_profile_returns_no_profile_for_unknown_peer() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut rx = online_peer(&relay, &alice).await;

        let ghost = "000000000000000000000000";
        relay
            .get_profile(&alice.peer_id(), "127.0.0.1", ghost)
            .await;
        let reply = read_reply(&mut rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("no_profile"));
    }
}
