//! Friend requests and the contact directory.
//!
//! The relay enforces a closed social graph — the server-level anti-spam
//! boundary. A peer may only send 1:1 envelopes to, fetch pre-keys from, or
//! add to a group someone who has ACCEPTED a friend request. Strangers can
//! never route ciphertext, harvest pre-key bundles or join groups.
//!
//! The contact table stores one normalized row per relationship
//! (`peer_a < peer_b`) regardless of who initiated it; a `requester` column
//! records the direction. A request starts as `pending` and flips to
//! `accepted`. Offline recipients discover pending requests through
//! `get_friend_requests` (they are persisted); online recipients get a live
//! push. Friend-request traffic draws from the per-IP `contacts:<ip>` rate
//! bucket.

use super::*;

impl Relay {
    /// Consume one token from the per-IP contact bucket. On exhaustion, send a
    /// `rate_limited` error to `peer_id` and return `false`.
    async fn take_contact_slot(&self, peer_id: &str, ip: &str) -> bool {
        if self
            .inner
            .contacts_limiter
            .try_take(&format!("contacts:{ip}"))
        {
            true
        } else {
            tracing::warn!(ip = %ip, "contact rate limit exceeded");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "rate_limited".into(),
                    },
                )
                .await;
            false
        }
    }

    /// Send a friend request from the caller to `target`.
    ///
    /// The recipient gets a `friend_request_received` push (with the caller's
    /// display name) when online; offline recipients find the request via
    /// `get_friend_requests` on their next connect. Rejected when the caller
    /// targets itself (`cannot_add_self`), re-requests a pending pair
    /// (`already_pending`) or requests an already-accepted contact
    /// (`already_contacts`).
    pub(crate) async fn send_friend_request(&self, peer_id: &str, ip: &str, target: &str) {
        if !self.take_contact_slot(peer_id, ip).await {
            return;
        }

        if peer_id == target {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "cannot_add_self".into(),
                    },
                )
                .await;
            return;
        }
        if self.inner.store.are_contacts(peer_id, target) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "already_contacts".into(),
                    },
                )
                .await;
            return;
        }
        if let Some((status, _)) = self.inner.store.contact_status(peer_id, target) {
            if status == "pending" {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "already_pending".into(),
                        },
                    )
                    .await;
                return;
            }
        }

        match self.inner.store.upsert_friend_request(peer_id, target) {
            Ok(()) => {
                tracing::info!(from = %peer_id, to = %target, "friend request sent");
                let _ = self.send(peer_id, ServerMessage::FriendRequestSent).await;
                // Push to the recipient when online; offline peers discover the
                // request through get_friend_requests.
                let display_name = self.inner.store.get_display_name(peer_id);
                let _ = self
                    .send(
                        target,
                        ServerMessage::FriendRequestReceived {
                            peer_id: peer_id.to_string(),
                            display_name,
                        },
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, target = %target, "failed to persist friend request: {err}");
            }
        }
    }

    /// Accept a pending friend request directed at the caller from `target`.
    ///
    /// Both peers become accepted contacts and receive a
    /// `friend_request_accepted` push naming their new contact; the caller
    /// additionally gets a `friend_request_accepted_ok` reply. Rejected when
    /// the pair is already contacts (`already_contacts`) or when no pending
    /// request from `target` exists (`not_found`).
    pub(crate) async fn accept_friend_request(&self, peer_id: &str, ip: &str, target: &str) {
        if !self.take_contact_slot(peer_id, ip).await {
            return;
        }

        if self.inner.store.are_contacts(peer_id, target) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "already_contacts".into(),
                    },
                )
                .await;
            return;
        }
        // Only the RECIPIENT of a pending request may accept it: the requester
        // column must name `target` (the one who asked).
        match self.inner.store.contact_status(peer_id, target) {
            Some((status, requester)) if status == "pending" && requester == target => {
                match self.inner.store.accept_friend(peer_id, target) {
                    Ok(()) => {
                        tracing::info!(a = %peer_id, b = %target, "friend request accepted");
                        let _ = self
                            .send(peer_id, ServerMessage::FriendRequestAcceptedOk)
                            .await;
                        // Both sides learn they are now contacts.
                        let _ = self
                            .send(
                                peer_id,
                                ServerMessage::FriendRequestAccepted {
                                    peer_id: target.to_string(),
                                },
                            )
                            .await;
                        let _ = self
                            .send(
                                target,
                                ServerMessage::FriendRequestAccepted {
                                    peer_id: peer_id.to_string(),
                                },
                            )
                            .await;
                    }
                    Err(err) => {
                        tracing::error!(peer = %peer_id, target = %target, "failed to accept friend request: {err}");
                    }
                }
            }
            _ => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "not_found".into(),
                        },
                    )
                    .await;
            }
        }
    }

    /// Decline a pending friend request directed at the caller from `target`.
    ///
    /// The request row is removed and the requester gets a
    /// `friend_request_declined` push; the caller gets a
    /// `friend_request_declined_ok` reply. Rejected with `not_found` when no
    /// pending request from `target` exists.
    pub(crate) async fn decline_friend_request(&self, peer_id: &str, ip: &str, target: &str) {
        if !self.take_contact_slot(peer_id, ip).await {
            return;
        }

        match self.inner.store.contact_status(peer_id, target) {
            Some((status, requester)) if status == "pending" && requester == target => {
                match self.inner.store.decline_friend(peer_id, target) {
                    Ok(()) => {
                        tracing::info!(from = %target, by = %peer_id, "friend request declined");
                        let _ = self
                            .send(peer_id, ServerMessage::FriendRequestDeclinedOk)
                            .await;
                        let _ = self
                            .send(
                                target,
                                ServerMessage::FriendRequestDeclined {
                                    peer_id: peer_id.to_string(),
                                },
                            )
                            .await;
                    }
                    Err(err) => {
                        tracing::error!(peer = %peer_id, target = %target, "failed to decline friend request: {err}");
                    }
                }
            }
            _ => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "not_found".into(),
                        },
                    )
                    .await;
            }
        }
    }

    /// Remove `target` from the caller's contacts.
    ///
    /// Both peers receive a `contact_removed` push; the caller additionally
    /// gets a `contact_removed_ok` reply. Rejected with `not_contacts` when
    /// the pair is not in an accepted relationship.
    pub(crate) async fn remove_contact(&self, peer_id: &str, ip: &str, target: &str) {
        if !self.take_contact_slot(peer_id, ip).await {
            return;
        }

        if !self.inner.store.are_contacts(peer_id, target) {
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

        match self.inner.store.remove_contact(peer_id, target) {
            Ok(()) => {
                tracing::info!(a = %peer_id, b = %target, "contact removed");
                let _ = self.send(peer_id, ServerMessage::ContactRemovedOk).await;
                // Both sides learn the relationship is gone.
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::ContactRemoved {
                            peer_id: target.to_string(),
                        },
                    )
                    .await;
                let _ = self
                    .send(
                        target,
                        ServerMessage::ContactRemoved {
                            peer_id: peer_id.to_string(),
                        },
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, target = %target, "failed to remove contact: {err}");
            }
        }
    }

    /// Answer `get_friend_requests`: the caller's pending incoming requests
    /// (requester + display name) and outgoing requests (target peer IDs).
    pub(crate) async fn get_friend_requests(&self, peer_id: &str, ip: &str) {
        if !self.take_contact_slot(peer_id, ip).await {
            return;
        }

        let incoming = self
            .inner
            .store
            .list_incoming(peer_id)
            .into_iter()
            .map(|(peer_id, display_name)| FriendRequestIncoming {
                peer_id,
                display_name,
            })
            .collect();
        let outgoing = self.inner.store.list_outgoing(peer_id);
        let _ = self
            .send(
                peer_id,
                ServerMessage::FriendRequests { incoming, outgoing },
            )
            .await;
    }

    /// Reply with the caller's accepted 1:1 contacts (peer IDs). Clients use
    /// this to rehydrate the local contact list after a database reset or
    /// restore — the relay is the source of truth for friendships.
    pub(crate) async fn list_contacts(&self, peer_id: &str, ip: &str) {
        if !self.take_contact_slot(peer_id, ip).await {
            return;
        }
        let peers = self.inner.store.list_contacts(peer_id);
        let _ = self.send(peer_id, ServerMessage::Contacts { peers }).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::test_utils::{env, make_contacts, online_peer, read_reply};

    #[tokio::test]
    async fn send_friend_request_acks_sender_and_pushes_recipient() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;
        relay
            .inner
            .store
            .set_display_name(&alice.peer_id(), "Test Alice")
            .unwrap();

        relay
            .send_friend_request(&alice.peer_id(), "127.0.0.1", &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("friend_request_sent"));

        // The recipient is pushed the request with the requester's display name.
        let push = read_reply(&mut bob_rx);
        assert_eq!(push["type"].as_str(), Some("friend_request_received"));
        assert_eq!(push["peer_id"].as_str(), Some(alice.peer_id().as_str()));
        assert_eq!(push["display_name"].as_str(), Some("Test Alice"));

        // The request is pending and stored for offline discovery.
        let (status, requester) = relay
            .inner
            .store
            .contact_status(&alice.peer_id(), &bob.peer_id())
            .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(requester, alice.peer_id());
        assert!(!relay
            .inner
            .store
            .are_contacts(&alice.peer_id(), &bob.peer_id()));
    }

    #[tokio::test]
    async fn friend_request_persists_for_offline_recipient() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        // bob is registered but never online.

        relay
            .send_friend_request(&alice.peer_id(), "127.0.0.1", &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("friend_request_sent"));
        // No live push is possible; the request lives in the store and is
        // surfaced through get_friend_requests on the recipient's next connect.
        assert_eq!(relay.inner.store.list_incoming(&bob.peer_id()).len(), 1);
        assert_eq!(
            relay.inner.store.list_outgoing(&alice.peer_id()),
            vec![bob.peer_id()]
        );
    }

    #[tokio::test]
    async fn accept_friend_request_makes_both_contacts_and_pushes_both() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .send_friend_request(&alice.peer_id(), "127.0.0.1", &bob.peer_id())
            .await;
        read_reply(&mut alice_rx);
        read_reply(&mut bob_rx);

        relay
            .accept_friend_request(&bob.peer_id(), "127.0.0.1", &alice.peer_id())
            .await;

        // Bob gets the reply ack, then a push naming his new contact alice.
        let ack = read_reply(&mut bob_rx);
        assert_eq!(ack["type"].as_str(), Some("friend_request_accepted_ok"));
        let push = read_reply(&mut bob_rx);
        assert_eq!(push["type"].as_str(), Some("friend_request_accepted"));
        assert_eq!(push["peer_id"].as_str(), Some(alice.peer_id().as_str()));

        // Alice gets the matching push naming her new contact bob.
        let push = read_reply(&mut alice_rx);
        assert_eq!(push["type"].as_str(), Some("friend_request_accepted"));
        assert_eq!(push["peer_id"].as_str(), Some(bob.peer_id().as_str()));

        assert!(relay
            .inner
            .store
            .are_contacts(&alice.peer_id(), &bob.peer_id()));
    }

    #[tokio::test]
    async fn accept_friend_request_rejects_when_no_request_is_pending() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut bob_rx = online_peer(&relay, &bob).await;

        // No request between them at all -> not_found.
        relay
            .accept_friend_request(&bob.peer_id(), "127.0.0.1", &alice.peer_id())
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_found"));

        // Bob can neither accept his OWN outgoing request (he is the requester,
        // not the recipient) ...
        relay
            .send_friend_request(&bob.peer_id(), "127.0.0.1", &alice.peer_id())
            .await;
        read_reply(&mut bob_rx);
        relay
            .accept_friend_request(&bob.peer_id(), "127.0.0.1", &alice.peer_id())
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["code"].as_str(), Some("not_found"));
    }

    #[tokio::test]
    async fn send_friend_request_rejects_self() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;

        relay
            .send_friend_request(&alice.peer_id(), "127.0.0.1", &alice.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("cannot_add_self"));
    }

    #[tokio::test]
    async fn send_friend_request_rejects_already_pending_and_already_contacts() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .send_friend_request(&alice.peer_id(), "127.0.0.1", &bob.peer_id())
            .await;
        read_reply(&mut alice_rx);
        read_reply(&mut bob_rx);

        // Duplicate pending request -> already_pending.
        relay
            .send_friend_request(&alice.peer_id(), "127.0.0.1", &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["code"].as_str(), Some("already_pending"));

        // After accepting, requesting again -> already_contacts.
        relay
            .accept_friend_request(&bob.peer_id(), "127.0.0.1", &alice.peer_id())
            .await;
        read_reply(&mut bob_rx);
        read_reply(&mut bob_rx);
        read_reply(&mut alice_rx);
        relay
            .send_friend_request(&alice.peer_id(), "127.0.0.1", &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["code"].as_str(), Some("already_contacts"));
    }

    #[tokio::test]
    async fn decline_friend_request_removes_request_and_pushes_requester() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .send_friend_request(&alice.peer_id(), "127.0.0.1", &bob.peer_id())
            .await;
        read_reply(&mut alice_rx);
        read_reply(&mut bob_rx);

        relay
            .decline_friend_request(&bob.peer_id(), "127.0.0.1", &alice.peer_id())
            .await;
        let ack = read_reply(&mut bob_rx);
        assert_eq!(ack["type"].as_str(), Some("friend_request_declined_ok"));
        // The requester (alice) is pushed the decliner's peer id.
        let push = read_reply(&mut alice_rx);
        assert_eq!(push["type"].as_str(), Some("friend_request_declined"));
        assert_eq!(push["peer_id"].as_str(), Some(bob.peer_id().as_str()));

        assert_eq!(
            relay
                .inner
                .store
                .contact_status(&alice.peer_id(), &bob.peer_id()),
            None
        );
    }

    #[tokio::test]
    async fn remove_contact_breaks_relationship_and_pushes_both() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .remove_contact(&alice.peer_id(), "127.0.0.1", &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["code"].as_str(), Some("not_contacts"));

        make_contacts(&relay, &alice.peer_id(), &bob.peer_id());
        relay
            .remove_contact(&alice.peer_id(), "127.0.0.1", &bob.peer_id())
            .await;
        let ack = read_reply(&mut alice_rx);
        assert_eq!(ack["type"].as_str(), Some("contact_removed_ok"));
        // Both sides are pushed the peer they lost.
        let push = read_reply(&mut alice_rx);
        assert_eq!(push["type"].as_str(), Some("contact_removed"));
        assert_eq!(push["peer_id"].as_str(), Some(bob.peer_id().as_str()));
        let push = read_reply(&mut bob_rx);
        assert_eq!(push["type"].as_str(), Some("contact_removed"));
        assert_eq!(push["peer_id"].as_str(), Some(alice.peer_id().as_str()));

        assert!(!relay
            .inner
            .store
            .are_contacts(&alice.peer_id(), &bob.peer_id()));
    }

    #[tokio::test]
    async fn get_friend_requests_lists_incoming_and_outgoing() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        online_peer(&relay, &bob).await;
        online_peer(&relay, &carol).await;
        relay
            .inner
            .store
            .set_display_name(&carol.peer_id(), "Carol Contact")
            .unwrap();

        // bob -> alice and carol -> alice (incoming for alice).
        relay
            .send_friend_request(&bob.peer_id(), "127.0.0.1", &alice.peer_id())
            .await;
        relay
            .send_friend_request(&carol.peer_id(), "127.0.0.1", &alice.peer_id())
            .await;
        // alice -> dave (outgoing for alice).
        let dave = Identity::new();
        relay
            .send_friend_request(&alice.peer_id(), "127.0.0.1", &dave.peer_id())
            .await;

        // Drain the live `friend_request_received` pushes alice got from bob and
        // carol plus her own `friend_request_sent` ack, so the read below sees
        // only the `friend_requests` reply.
        while alice_rx.try_recv().is_ok() {}

        relay
            .get_friend_requests(&alice.peer_id(), "127.0.0.1")
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("friend_requests"));
        let incoming = reply["incoming"]
            .as_array()
            .expect("incoming must be an array");
        let mut pairs: Vec<(String, Option<String>)> = incoming
            .iter()
            .map(|r| {
                (
                    r["peer_id"].as_str().unwrap().to_string(),
                    r["display_name"].as_str().map(|s| s.to_string()),
                )
            })
            .collect();
        pairs.sort();
        let mut expected = vec![
            (bob.peer_id().to_string(), None),
            (
                carol.peer_id().to_string(),
                Some("Carol Contact".to_string()),
            ),
        ];
        expected.sort();
        assert_eq!(pairs, expected);
        let outgoing = reply["outgoing"]
            .as_array()
            .expect("outgoing must be an array");
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].as_str(), Some(dave.peer_id().as_str()));
    }

    #[tokio::test]
    async fn contact_operations_are_rate_limited_per_ip() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 1.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;

        relay
            .send_friend_request(&alice.peer_id(), "10.0.0.1", &bob.peer_id())
            .await;
        assert_eq!(
            read_reply(&mut alice_rx)["type"].as_str(),
            Some("friend_request_sent")
        );

        relay
            .send_friend_request(&alice.peer_id(), "10.0.0.1", &carol.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("rate_limited"));

        // A different IP has its own bucket.
        relay
            .send_friend_request(&alice.peer_id(), "10.0.0.2", &carol.peer_id())
            .await;
        assert_eq!(
            read_reply(&mut alice_rx)["type"].as_str(),
            Some("friend_request_sent")
        );
    }

    #[tokio::test]
    async fn route_rejects_envelope_between_non_contacts() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .route(
                env(&alice.peer_id(), &bob.peer_id(), 1),
                &alice.peer_id(),
                "127.0.0.1",
            )
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_contacts"));
        // Nothing is routed, queued or acked.
        assert!(
            bob_rx.try_recv().is_err(),
            "a non-contact envelope must not reach the recipient"
        );
        assert_eq!(relay.inner.store.count_for(&bob.peer_id()), 0);
    }

    #[tokio::test]
    async fn route_delivers_envelope_between_contacts() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;
        make_contacts(&relay, &alice.peer_id(), &bob.peer_id());

        relay
            .route(
                env(&alice.peer_id(), &bob.peer_id(), 7),
                &alice.peer_id(),
                "127.0.0.1",
            )
            .await;
        let ack = read_reply(&mut alice_rx);
        assert_eq!(ack["type"].as_str(), Some("ack"));
        assert_eq!(ack["seq"].as_u64(), Some(7));
        let envelope = read_reply(&mut bob_rx);
        assert_eq!(envelope["type"].as_str(), Some("envelope"));
        assert_eq!(envelope["envelope"]["seq"].as_u64(), Some(7));
    }

    #[tokio::test]
    async fn fetch_prekeys_requires_accepted_contact() {
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
            .publish_prekeys(&owner_id, "127.0.0.1", owner.pre_key_bundle(2))
            .await;

        let requester = Identity::new();
        let mut requester_rx = online_peer(&relay, &requester).await;

        // Not contacts -> the fetch is refused before any bundle lookup.
        relay
            .fetch_prekeys(&requester.peer_id(), "127.0.0.1", &owner_id)
            .await;
        let reply = read_reply(&mut requester_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_contacts"));

        // Once accepted, the same fetch succeeds.
        make_contacts(&relay, &requester.peer_id(), &owner_id);
        relay
            .fetch_prekeys(&requester.peer_id(), "127.0.0.1", &owner_id)
            .await;
        let reply = read_reply(&mut requester_rx);
        assert_eq!(reply["type"].as_str(), Some("prekeys"));
    }

    #[tokio::test]
    async fn add_group_member_requires_accepted_contact() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        online_peer(&relay, &bob).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();

        // Bob is not alice's accepted contact -> refused before any roster change.
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_contacts"));
        assert!(!relay.inner.store.is_group_member(&group_id, &bob.peer_id()));

        // Once accepted, the same add succeeds.
        make_contacts(&relay, &alice.peer_id(), &bob.peer_id());
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("group_member_added"));
        assert!(relay.inner.store.is_group_member(&group_id, &bob.peer_id()));
    }
}
