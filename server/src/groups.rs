//! Group metadata, rosters and roles.
//!
//! Groups are metadata only: the relay stores the roster and fans
//! `send_group_message` envelopes out to every member. The Megolm session key
//! is SECRET and is never seen or stored by the relay — it travels end-to-end
//! between members inside Double Ratchet envelopes. Roster mutations
//! (create/add/leave/remove) and the owner/admin role promotion/demotion all
//! draw from the per-IP `group:<ip>` rate bucket.

use super::*;

impl Relay {
    /// Create a group: generate a unique group ID, persist the public metadata
    /// and register the caller as the owner/first member.
    ///
    /// The Megolm session key is deliberately NOT part of this flow. It is
    /// secret and is shared to members end-to-end over an encrypted envelope
    /// by the desktop client; the relay never sees it.
    pub(crate) async fn create_group(&self, peer_id: &str, ip: &str, name: &str) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }

        if !Self::is_valid_group_name(name) {
            tracing::warn!(peer = %peer_id, "rejecting invalid group name");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "invalid_group_name".into(),
                    },
                )
                .await;
            return;
        }

        let group_id = uuid::Uuid::new_v4().to_string();
        match self
            .inner
            .store
            .create_group(&group_id, name, peer_id, unix_now())
        {
            Ok(()) => {
                let members = self.inner.store.list_group_members(&group_id);
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::GroupCreated {
                            group_id,
                            name: name.to_string(),
                            members,
                        },
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, "failed to persist group: {err}");
            }
        }
    }

    /// Add `target` to a group's roster. Only the owner or an existing member
    /// may add members.
    pub(crate) async fn add_group_member(
        &self,
        peer_id: &str,
        ip: &str,
        group_id: &str,
        target: &str,
    ) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }

        if self.inner.store.get_group(group_id).is_none() {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "group_not_found".into(),
                    },
                )
                .await;
            return;
        }
        if !self.inner.store.is_group_member(group_id, peer_id) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_a_member".into(),
                    },
                )
                .await;
            return;
        }

        match self
            .inner
            .store
            .add_group_member(group_id, target, unix_now())
        {
            Ok(()) => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::GroupMemberAdded {
                            group_id: group_id.to_string(),
                            peer_id: target.to_string(),
                        },
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, group = %group_id, "failed to add member: {err}");
            }
        }
    }

    /// Remove the caller from a group's roster.
    pub(crate) async fn leave_group(&self, peer_id: &str, ip: &str, group_id: &str) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }

        if self.inner.store.get_group(group_id).is_none() {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "group_not_found".into(),
                    },
                )
                .await;
            return;
        }
        if !self.inner.store.is_group_member(group_id, peer_id) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_a_member".into(),
                    },
                )
                .await;
            return;
        }

        match self.inner.store.remove_group_member(group_id, peer_id) {
            Ok(()) => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::GroupMemberLeft {
                            group_id: group_id.to_string(),
                            peer_id: peer_id.to_string(),
                        },
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, group = %group_id, "failed to remove member: {err}");
            }
        }
    }

    /// Reply with a group's public metadata and member roster including each
    /// member's role. The roster is only visible to current members.
    pub(crate) async fn get_group_info(&self, peer_id: &str, ip: &str, group_id: &str) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }

        let group = match self.inner.store.get_group(group_id) {
            Some(group) => group,
            None => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::Error {
                            code: "group_not_found".into(),
                        },
                    )
                    .await;
                return;
            }
        };
        if !self.inner.store.is_group_member(group_id, peer_id) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_a_member".into(),
                    },
                )
                .await;
            return;
        }

        let members = self
            .inner
            .store
            .members_with_roles(group_id)
            .into_iter()
            .map(|(peer_id, role)| GroupMember { peer_id, role })
            .collect();
        let _ = self
            .send(
                peer_id,
                ServerMessage::GroupInfo {
                    group_id: group.id,
                    name: group.name,
                    owner_peer_id: group.owner_peer_id,
                    members,
                },
            )
            .await;
    }

    /// Promote `target` to a group admin (`promote_member`).
    ///
    /// Permissions:
    /// - Only the group owner or an existing admin may promote. A regular
    ///   member gets `not_admin`.
    /// - Promoting a member makes them an admin; promoting an admin is a
    ///   no-op. The owner is never demoted or re-roling: promoting the owner
    ///   is a no-op.
    /// - `target` must be a member of the group, else `not_a_member`.
    pub(crate) async fn promote_member(
        &self,
        peer_id: &str,
        ip: &str,
        group_id: &str,
        target: &str,
    ) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }

        if self.inner.store.get_group(group_id).is_none() {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "group_not_found".into(),
                    },
                )
                .await;
            return;
        }
        if !self.inner.store.is_group_member(group_id, peer_id) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_a_member".into(),
                    },
                )
                .await;
            return;
        }
        let actor_role = self.inner.store.get_member_role(group_id, peer_id);
        if !matches!(actor_role.as_deref(), Some("owner") | Some("admin")) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_admin".into(),
                    },
                )
                .await;
            return;
        }
        if !self.inner.store.is_group_member(group_id, target) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_a_member".into(),
                    },
                )
                .await;
            return;
        }

        // owner->admin is the only real promotion; an already-admin target is
        // a no-op and the owner stays owner.
        let target_role = self.inner.store.get_member_role(group_id, target);
        if matches!(target_role.as_deref(), Some("owner") | Some("admin")) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::GroupMemberPromoted {
                        group_id: group_id.to_string(),
                        peer_id: target.to_string(),
                    },
                )
                .await;
            return;
        }

        match self.inner.store.set_member_role(group_id, target, "admin") {
            Ok(()) => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::GroupMemberPromoted {
                            group_id: group_id.to_string(),
                            peer_id: target.to_string(),
                        },
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, group = %group_id, "failed to promote member: {err}");
            }
        }
    }

    /// Demote `target` from admin to a regular member (`demote_member`).
    ///
    /// Permissions:
    /// - Only the group owner may demote. An admin or member gets `not_owner`.
    /// - The owner cannot demote themselves (or any owner): demoting the owner
    ///   yields `not_owner`.
    /// - `target` must be a member of the group, else `not_a_member`.
    pub(crate) async fn demote_member(
        &self,
        peer_id: &str,
        ip: &str,
        group_id: &str,
        target: &str,
    ) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }

        if self.inner.store.get_group(group_id).is_none() {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "group_not_found".into(),
                    },
                )
                .await;
            return;
        }
        if !self.inner.store.is_group_member(group_id, peer_id) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_a_member".into(),
                    },
                )
                .await;
            return;
        }
        let actor_role = self.inner.store.get_member_role(group_id, peer_id);
        if actor_role.as_deref() != Some("owner") {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_owner".into(),
                    },
                )
                .await;
            return;
        }
        if !self.inner.store.is_group_member(group_id, target) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_a_member".into(),
                    },
                )
                .await;
            return;
        }
        // The owner cannot be demoted (this also guards self-demotion: the
        // only owner is the actor, and an owner is never an admin).
        let target_role = self.inner.store.get_member_role(group_id, target);
        if target_role.as_deref() == Some("owner") {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_owner".into(),
                    },
                )
                .await;
            return;
        }

        match self.inner.store.set_member_role(group_id, target, "member") {
            Ok(()) => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::GroupMemberDemoted {
                            group_id: group_id.to_string(),
                            peer_id: target.to_string(),
                        },
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, group = %group_id, "failed to demote member: {err}");
            }
        }
    }

    /// Remove `target` from a group's roster (`remove_member`).
    ///
    /// Permissions:
    /// - Only the group owner may remove a member. An admin or member gets
    ///   `not_owner`.
    /// - The owner cannot remove themselves: that would leave the group with
    ///   no owner, so it yields `not_owner`.
    /// - `target` must be a member of the group, else `not_a_member`.
    pub(crate) async fn remove_member(
        &self,
        peer_id: &str,
        ip: &str,
        group_id: &str,
        target: &str,
    ) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }

        if self.inner.store.get_group(group_id).is_none() {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "group_not_found".into(),
                    },
                )
                .await;
            return;
        }
        if !self.inner.store.is_group_member(group_id, peer_id) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_a_member".into(),
                    },
                )
                .await;
            return;
        }
        let actor_role = self.inner.store.get_member_role(group_id, peer_id);
        if actor_role.as_deref() != Some("owner") {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_owner".into(),
                    },
                )
                .await;
            return;
        }
        if !self.inner.store.is_group_member(group_id, target) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_a_member".into(),
                    },
                )
                .await;
            return;
        }
        // Admins may be removed, but the owner cannot be removed by anyone
        // (including themselves).
        if target == peer_id
            || self
                .inner
                .store
                .get_member_role(group_id, target)
                .as_deref()
                == Some("owner")
        {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_owner".into(),
                    },
                )
                .await;
            return;
        }

        match self.inner.store.remove_group_member(group_id, target) {
            Ok(()) => {
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::GroupMemberRemoved {
                            group_id: group_id.to_string(),
                            peer_id: target.to_string(),
                        },
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, group = %group_id, "failed to remove member: {err}");
            }
        }
    }

    /// Fan out one client-encrypted envelope to every group member except the
    /// sender.
    ///
    /// The relay rewrites `recipient` per member and reuses the standard
    /// live/offline delivery path, so the ciphertext stays opaque and members
    /// who are offline get the copy on their next fetch. Group sends draw from
    /// the per-IP `group:<ip>` rate bucket.
    pub(crate) async fn send_group_message(
        &self,
        peer_id: &str,
        ip: &str,
        group_id: &str,
        envelope: Envelope,
    ) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }

        if self.inner.store.get_group(group_id).is_none() {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "group_not_found".into(),
                    },
                )
                .await;
            return;
        }
        if !self.inner.store.is_group_member(group_id, peer_id) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_a_member".into(),
                    },
                )
                .await;
            return;
        }

        // Spoofing guard and size cap mirror the 1:1 routing path.
        if envelope.sender != peer_id {
            tracing::warn!(
                claimed = %envelope.sender,
                authenticated = %peer_id,
                "group envelope sender does not match the authenticated peer"
            );
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "sender_mismatch".into(),
                    },
                )
                .await;
            return;
        }
        if !envelope.within_limits() {
            tracing::warn!(sender = %envelope.sender, group = %group_id, "dropping oversized group envelope");
            return;
        }

        let seq = envelope.seq;
        let members = self.inner.store.list_group_members(group_id);
        for member in &members {
            if member == peer_id {
                continue;
            }
            let mut copy = envelope.clone();
            copy.recipient = member.clone();
            self.deliver_one(&copy).await;
        }

        // Single delivery confirmation to the sender; the fan-out copies share
        // the same client `seq`.
        let _ = self
            .send(peer_id, ServerMessage::Acknowledged { seq })
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::test_utils::{env, online_peer, read_reply};

    #[tokio::test]
    async fn create_group_replies_with_owner_membership() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Ghost Squad")
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("group_created"));
        let group_id = reply["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();
        assert_eq!(reply["name"].as_str(), Some("Ghost Squad"));
        let members = reply["members"]
            .as_array()
            .expect("members must be an array");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].as_str(), Some(alice.peer_id().as_str()));
        assert!(
            relay
                .inner
                .store
                .is_group_member(&group_id, &alice.peer_id()),
            "the owner must be a member"
        );
    }

    #[tokio::test]
    async fn create_group_rejects_invalid_name() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;

        relay.create_group(&alice.peer_id(), "127.0.0.1", "").await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("invalid_group_name"));

        let too_long = "x".repeat(super::super::MAX_GROUP_NAME_CHARS + 1);
        relay
            .create_group(&alice.peer_id(), "127.0.0.1", &too_long)
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("invalid_group_name"));
    }

    #[tokio::test]
    async fn add_group_member_and_get_group_info_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();

        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("group_member_added"));
        assert_eq!(reply["group_id"].as_str(), Some(group_id.as_str()));
        assert_eq!(reply["peer_id"].as_str(), Some(bob.peer_id().as_str()));

        relay
            .get_group_info(&alice.peer_id(), "127.0.0.1", &group_id)
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("group_info"));
        assert_eq!(reply["name"].as_str(), Some("Squad"));
        assert_eq!(
            reply["owner_peer_id"].as_str(),
            Some(alice.peer_id().as_str())
        );
        let members = reply["members"]
            .as_array()
            .expect("members must be an array");
        let member_ids: Vec<&str> = members
            .iter()
            .filter_map(|m| m["peer_id"].as_str())
            .collect();
        assert!(member_ids.contains(&alice.peer_id().as_str()));
        assert!(member_ids.contains(&bob.peer_id().as_str()));
        // Roles: the creator owns the group, the added member is a member.
        let roles: Vec<&str> = members.iter().filter_map(|m| m["role"].as_str()).collect();
        assert!(roles.contains(&"owner"));
        assert!(roles.contains(&"member"));

        // Bob (a member) may also read the info.
        relay
            .get_group_info(&bob.peer_id(), "127.0.0.1", &group_id)
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("group_info"));
    }

    #[tokio::test]
    async fn add_group_member_rejects_non_member_and_unknown_group() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut carol_rx = online_peer(&relay, &carol).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();

        // Carol is not a member and cannot add anyone.
        relay
            .add_group_member(&carol.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        let reply = read_reply(&mut carol_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_a_member"));

        // An unknown group id.
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", "ghost", &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("group_not_found"));
    }

    #[tokio::test]
    async fn send_group_message_fans_out_to_members_except_sender() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;
        let mut carol_rx = online_peer(&relay, &carol).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        read_reply(&mut alice_rx);
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &carol.peer_id())
            .await;
        read_reply(&mut alice_rx);

        relay
            .send_group_message(
                &alice.peer_id(),
                "127.0.0.1",
                &group_id,
                env(&alice.peer_id(), "ignored", 42),
            )
            .await;

        // Alice gets a single ack (and no envelope copy for herself).
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("ack"));
        assert_eq!(reply["seq"].as_u64(), Some(42));
        assert!(
            alice_rx.try_recv().is_err(),
            "the sender must not receive its own group copy"
        );

        // Bob and carol each get a copy with the recipient rewritten.
        let bob_msg = read_reply(&mut bob_rx);
        assert_eq!(bob_msg["type"].as_str(), Some("envelope"));
        assert_eq!(
            bob_msg["envelope"]["recipient"].as_str(),
            Some(bob.peer_id().as_str())
        );
        assert_eq!(
            bob_msg["envelope"]["sender"].as_str(),
            Some(alice.peer_id().as_str())
        );
        let carol_msg = read_reply(&mut carol_rx);
        assert_eq!(carol_msg["type"].as_str(), Some("envelope"));
        assert_eq!(
            carol_msg["envelope"]["recipient"].as_str(),
            Some(carol.peer_id().as_str())
        );
    }

    #[tokio::test]
    async fn send_group_message_queues_for_offline_members() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new(); // registered in the store but never online
        relay
            .inner
            .store
            .register_user_with_keys(
                &bob.peer_id(),
                &bob.curve25519_key().to_base64(),
                &bob.ed25519_key().to_base64(),
                unix_now(),
            )
            .unwrap();
        let mut alice_rx = online_peer(&relay, &alice).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        read_reply(&mut alice_rx);

        relay
            .send_group_message(
                &alice.peer_id(),
                "127.0.0.1",
                &group_id,
                env(&alice.peer_id(), "any", 7),
            )
            .await;
        assert_eq!(read_reply(&mut alice_rx)["type"].as_str(), Some("ack"));

        // Bob is offline, so his copy lands in the SQLite queue for him.
        assert_eq!(relay.inner.store.count_for(&bob.peer_id()), 1);
        let queued = relay.inner.store.list_for(&bob.peer_id(), unix_now());
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].recipient, bob.peer_id());
        assert_eq!(queued[0].sender, alice.peer_id());
    }

    #[tokio::test]
    async fn send_group_message_rejects_non_member() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut carol_rx = online_peer(&relay, &carol).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();

        relay
            .send_group_message(
                &carol.peer_id(),
                "127.0.0.1",
                &group_id,
                env(&carol.peer_id(), "x", 1),
            )
            .await;
        let reply = read_reply(&mut carol_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_a_member"));

        // Unknown group.
        relay
            .send_group_message(
                &alice.peer_id(),
                "127.0.0.1",
                "ghost",
                env(&alice.peer_id(), "x", 2),
            )
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("group_not_found"));
    }

    #[tokio::test]
    async fn send_group_message_rejects_spoofed_sender() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        read_reply(&mut alice_rx);

        // Alice claims to be Bob inside a group envelope.
        relay
            .send_group_message(
                &alice.peer_id(),
                "127.0.0.1",
                &group_id,
                env(&bob.peer_id(), "spoofed", 99),
            )
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("sender_mismatch"));
        assert!(
            bob_rx.try_recv().is_err(),
            "a spoofed group envelope must not be delivered"
        );
    }

    #[tokio::test]
    async fn leave_group_removes_member_and_revokes_send() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        read_reply(&mut alice_rx);

        relay
            .leave_group(&bob.peer_id(), "127.0.0.1", &group_id)
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("group_member_left"));
        assert_eq!(reply["group_id"].as_str(), Some(group_id.as_str()));
        assert!(!relay.inner.store.is_group_member(&group_id, &bob.peer_id()));

        // Bob can no longer send to the group.
        relay
            .send_group_message(
                &bob.peer_id(),
                "127.0.0.1",
                &group_id,
                env(&bob.peer_id(), "x", 1),
            )
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_a_member"));
    }

    #[tokio::test]
    async fn get_group_info_requires_membership() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut carol_rx = online_peer(&relay, &carol).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();

        relay
            .get_group_info(&carol.peer_id(), "127.0.0.1", &group_id)
            .await;
        let reply = read_reply(&mut carol_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_a_member"));
    }

    #[tokio::test]
    async fn group_operations_are_rate_limited_per_ip() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 1.0, 0.0);
        let alice = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;

        relay
            .create_group(&alice.peer_id(), "10.0.0.1", "First")
            .await;
        assert_eq!(
            read_reply(&mut alice_rx)["type"].as_str(),
            Some("group_created")
        );

        relay
            .create_group(&alice.peer_id(), "10.0.0.1", "Second")
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("rate_limited"));

        // A different IP has its own group bucket.
        relay
            .create_group(&alice.peer_id(), "10.0.0.2", "Third")
            .await;
        assert_eq!(
            read_reply(&mut alice_rx)["type"].as_str(),
            Some("group_created")
        );
    }

    // -- Group roles (promote / demote / remove) ------------------------------

    /// Build a group owned by `alice` with `bob` (member) and `carol` (member)
    /// in it; returns the group id.
    async fn role_group(
        relay: &Relay,
        alice_rx: &mut mpsc::UnboundedReceiver<WsMessage>,
        alice: &Identity,
        bob: &Identity,
        carol: &Identity,
    ) -> String {
        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Role Squad")
            .await;
        let group_id = read_reply(alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        read_reply(alice_rx);
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &carol.peer_id())
            .await;
        read_reply(alice_rx);
        group_id
    }

    #[tokio::test]
    async fn group_info_lists_members_with_roles() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        online_peer(&relay, &bob).await;
        online_peer(&relay, &carol).await;
        let group_id = role_group(&relay, &mut alice_rx, &alice, &bob, &carol).await;

        relay
            .get_group_info(&alice.peer_id(), "127.0.0.1", &group_id)
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("group_info"));
        let members = reply["members"]
            .as_array()
            .expect("members must be an array");
        assert_eq!(members.len(), 3);
        for member in members {
            assert!(
                member["peer_id"].is_string(),
                "each member must carry a peer_id"
            );
            assert!(
                matches!(member["role"].as_str(), Some("owner" | "admin" | "member")),
                "each member must carry a role"
            );
        }
        // The creator is the only owner.
        let owner: Vec<&str> = members
            .iter()
            .filter(|m| m["role"].as_str() == Some("owner"))
            .filter_map(|m| m["peer_id"].as_str())
            .collect();
        assert_eq!(owner, vec![alice.peer_id().as_str()]);
    }

    #[tokio::test]
    async fn promote_member_makes_a_member_an_admin() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;
        online_peer(&relay, &carol).await;
        let group_id = role_group(&relay, &mut alice_rx, &alice, &bob, &carol).await;

        // The owner promotes bob.
        relay
            .promote_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("group_member_promoted"));
        assert_eq!(reply["peer_id"].as_str(), Some(bob.peer_id().as_str()));
        assert_eq!(
            relay
                .inner
                .store
                .get_member_role(&group_id, &bob.peer_id())
                .as_deref(),
            Some("admin")
        );

        // A promoted admin can promote another member too.
        relay
            .promote_member(&bob.peer_id(), "127.0.0.1", &group_id, &carol.peer_id())
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("group_member_promoted"));
        assert_eq!(
            relay
                .inner
                .store
                .get_member_role(&group_id, &carol.peer_id())
                .as_deref(),
            Some("admin")
        );
    }

    #[tokio::test]
    async fn promote_member_rejects_regular_member_actor() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;
        online_peer(&relay, &carol).await;
        let group_id = role_group(&relay, &mut alice_rx, &alice, &bob, &carol).await;

        // Bob is a plain member: he cannot promote anyone.
        relay
            .promote_member(&bob.peer_id(), "127.0.0.1", &group_id, &carol.peer_id())
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_admin"));
        assert_eq!(
            relay
                .inner
                .store
                .get_member_role(&group_id, &carol.peer_id())
                .as_deref(),
            Some("member"),
            "a rejected promotion must not touch the target's role"
        );

        // Promoting a non-member also fails.
        let dave = Identity::new();
        relay
            .promote_member(&alice.peer_id(), "127.0.0.1", &group_id, &dave.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_a_member"));
    }

    #[tokio::test]
    async fn demote_member_requires_owner_and_lowers_admin() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;
        online_peer(&relay, &carol).await;
        let group_id = role_group(&relay, &mut alice_rx, &alice, &bob, &carol).await;
        relay
            .promote_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        read_reply(&mut alice_rx);

        // The owner demotes bob back to a member.
        relay
            .demote_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("group_member_demoted"));
        assert_eq!(reply["peer_id"].as_str(), Some(bob.peer_id().as_str()));
        assert_eq!(
            relay
                .inner
                .store
                .get_member_role(&group_id, &bob.peer_id())
                .as_deref(),
            Some("member")
        );

        // An admin (bob is again a member here, so promote him first) cannot
        // demote: demote is owner-only.
        relay
            .promote_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        read_reply(&mut alice_rx);
        relay
            .demote_member(&bob.peer_id(), "127.0.0.1", &group_id, &carol.peer_id())
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_owner"));
    }

    #[tokio::test]
    async fn owner_cannot_demote_themselves() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        online_peer(&relay, &bob).await;
        online_peer(&relay, &carol).await;
        let group_id = role_group(&relay, &mut alice_rx, &alice, &bob, &carol).await;

        // The owner tries to demote themselves: the owner role is not an
        // admin, so the demote must be rejected.
        relay
            .demote_member(&alice.peer_id(), "127.0.0.1", &group_id, &alice.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_owner"));
        assert_eq!(
            relay
                .inner
                .store
                .get_member_role(&group_id, &alice.peer_id())
                .as_deref(),
            Some("owner"),
            "the owner's role must never change"
        );
    }

    #[tokio::test]
    async fn remove_member_requires_owner_and_removes_admins() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;
        online_peer(&relay, &carol).await;
        let group_id = role_group(&relay, &mut alice_rx, &alice, &bob, &carol).await;
        relay
            .promote_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        read_reply(&mut alice_rx);

        // An admin cannot remove anyone: remove is owner-only.
        relay
            .remove_member(&bob.peer_id(), "127.0.0.1", &group_id, &carol.peer_id())
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_owner"));

        // The owner removes bob (an admin) from the roster.
        relay
            .remove_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("group_member_removed"));
        assert_eq!(reply["peer_id"].as_str(), Some(bob.peer_id().as_str()));
        assert!(!relay.inner.store.is_group_member(&group_id, &bob.peer_id()));
    }

    #[tokio::test]
    async fn owner_cannot_remove_themselves() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        online_peer(&relay, &bob).await;
        online_peer(&relay, &carol).await;
        let group_id = role_group(&relay, &mut alice_rx, &alice, &bob, &carol).await;

        relay
            .remove_member(&alice.peer_id(), "127.0.0.1", &group_id, &alice.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_owner"));
        assert!(relay
            .inner
            .store
            .is_group_member(&group_id, &alice.peer_id()));
    }

    #[tokio::test]
    async fn role_operations_reject_unknown_groups() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        online_peer(&relay, &bob).await;

        relay
            .promote_member(&alice.peer_id(), "127.0.0.1", "ghost", &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("group_not_found"));

        relay
            .demote_member(&alice.peer_id(), "127.0.0.1", "ghost", &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("group_not_found"));

        relay
            .remove_member(&alice.peer_id(), "127.0.0.1", "ghost", &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("group_not_found"));
    }
}
