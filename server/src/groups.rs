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
                tracing::info!(peer = %peer_id, group = %group_id, name = %name, "group created");
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
    ///
    /// Contact gate: the added peer must be the adder's ACCEPTED friend. A
    /// stranger cannot be pulled into a group by anyone — this closes the
    /// group side of the anti-spam boundary.
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

        match self
            .inner
            .store
            .add_group_member(group_id, target, unix_now())
        {
            Ok(()) => {
                tracing::info!(peer = %peer_id, group = %group_id, added = %target, "group member added");
                // Fan the roster change out to every existing member except the
                // newly added peer. The caller's socket is included (it is a
                // member), so this both resolves the requester and lets the
                // other online members update their rosters and — in the
                // multi-sender model — share their own Megolm session key to
                // the newcomer. Offline members learn about the change on
                // their next `get_group_info` round-trip.
                let members = self.inner.store.list_group_members(group_id);
                for member in members {
                    if member == target {
                        continue;
                    }
                    let _ = self
                        .send(
                            &member,
                            ServerMessage::GroupMemberAdded {
                                group_id: group_id.to_string(),
                                peer_id: target.to_string(),
                            },
                        )
                        .await;
                }
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, group = %group_id, "failed to add member: {err}");
            }
        }
    }

    /// Invite `target` to join `group_id`. Unlike [`Self::add_group_member`]
    /// the invitee is NOT added to the roster immediately — they receive a
    /// `group_invite_received` push and decide (accept/decline). Only the
    /// owner or an admin may invite, and only accepted contacts may be
    /// invited.
    pub(crate) async fn group_invite(&self, peer_id: &str, ip: &str, group_id: &str, target: &str) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }
        let role = self.inner.store.get_member_role(group_id, peer_id);
        if !matches!(role.as_deref(), Some("owner") | Some("admin")) {
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
        if self.inner.store.is_group_member(group_id, target) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "already_member".into(),
                    },
                )
                .await;
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
        if self.inner.store.is_group_invited(group_id, target) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "already_invited".into(),
                    },
                )
                .await;
            return;
        }

        match self.inner.store.invite_to_group(group_id, target, peer_id) {
            Ok(()) => {
                tracing::info!(inviter = %peer_id, group = %group_id, target = %target, "group invite sent");
                let _ = self.send(peer_id, ServerMessage::GroupInviteSent).await;
                let group_name = self
                    .inner
                    .store
                    .get_group(group_id)
                    .map(|g| g.name)
                    .unwrap_or_default();
                let _ = self
                    .send(
                        target,
                        ServerMessage::GroupInviteReceived {
                            group_id: group_id.to_string(),
                            group_name,
                            inviter_peer_id: peer_id.to_string(),
                        },
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(inviter = %peer_id, group = %group_id, "failed to store invite: {err}");
            }
        }
    }

    /// Accept a pending invite: the caller joins the roster, the invite row is
    /// removed, the inviter gets a `group_invite_accepted` push and every
    /// other member gets the usual `group_member_added` fan-out (so they share
    /// their Megolm keys with the newcomer).
    pub(crate) async fn group_invite_accept(&self, peer_id: &str, ip: &str, group_id: &str) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }
        let inviter = {
            let invites = self.inner.store.group_invites_for(peer_id);
            invites
                .iter()
                .find(|(gid, _)| gid == group_id)
                .map(|(_, inviter)| inviter.clone())
        };
        let Some(inviter) = inviter else {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_invited".into(),
                    },
                )
                .await;
            return;
        };

        match self
            .inner
            .store
            .add_group_member(group_id, peer_id, unix_now())
        {
            Ok(()) => {
                let _ = self.inner.store.remove_group_invite(group_id, peer_id);
                tracing::info!(peer = %peer_id, group = %group_id, "group invite accepted");
                let _ = self
                    .send(peer_id, ServerMessage::GroupInviteAcceptedOk)
                    .await;
                let _ = self
                    .send(
                        &inviter,
                        ServerMessage::GroupInviteAccepted {
                            group_id: group_id.to_string(),
                            peer_id: peer_id.to_string(),
                        },
                    )
                    .await;
                // Fan out the roster change so existing members share keys.
                let members = self.inner.store.list_group_members(group_id);
                for member in members {
                    if member == peer_id {
                        continue;
                    }
                    let _ = self
                        .send(
                            &member,
                            ServerMessage::GroupMemberAdded {
                                group_id: group_id.to_string(),
                                peer_id: peer_id.to_string(),
                            },
                        )
                        .await;
                }
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, group = %group_id, "failed to accept invite: {err}");
            }
        }
    }

    /// Decline a pending invite: the invite row is removed and the inviter
    /// gets a `group_invite_declined` push.
    pub(crate) async fn group_invite_decline(&self, peer_id: &str, ip: &str, group_id: &str) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }
        let inviter = {
            let invites = self.inner.store.group_invites_for(peer_id);
            invites
                .iter()
                .find(|(gid, _)| gid == group_id)
                .map(|(_, inviter)| inviter.clone())
        };
        let Some(inviter) = inviter else {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "not_invited".into(),
                    },
                )
                .await;
            return;
        };
        match self.inner.store.remove_group_invite(group_id, peer_id) {
            Ok(()) => {
                tracing::info!(peer = %peer_id, group = %group_id, "group invite declined");
                let _ = self
                    .send(peer_id, ServerMessage::GroupInviteDeclinedOk)
                    .await;
                let _ = self
                    .send(
                        &inviter,
                        ServerMessage::GroupInviteDeclined {
                            group_id: group_id.to_string(),
                            peer_id: peer_id.to_string(),
                        },
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, group = %group_id, "failed to decline invite: {err}");
            }
        }
    }

    /// Reply with the caller's pending group invites (group id, name, inviter).
    pub(crate) async fn get_group_invites(&self, peer_id: &str, ip: &str) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }
        let invites = self.inner.store.group_invites_for(peer_id);
        let mut out = Vec::with_capacity(invites.len());
        for (group_id, inviter_peer_id) in invites {
            let group_name = self
                .inner
                .store
                .get_group(&group_id)
                .map(|g| g.name)
                .unwrap_or_default();
            out.push(GroupInviteInfo {
                group_id,
                group_name,
                inviter_peer_id,
            });
        }
        let _ = self
            .send(peer_id, ServerMessage::GroupInvites { invites: out })
            .await;
    }

    /// Get (or create) the group's shareable join link. Any member may ask.
    /// The link is `whisper://join?group=<id>&token=<secret>` — the token
    /// authorizes joining, so anyone with the link can join (like a WhatsApp
    /// group invite).
    pub(crate) async fn get_group_join_link(&self, peer_id: &str, ip: &str, group_id: &str) {
        if !self.take_group_slot(peer_id, ip).await {
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
        let Some(token) = self.inner.store.ensure_join_token(group_id) else {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "group_not_found".into(),
                    },
                )
                .await;
            return;
        };
        let group_name = self
            .inner
            .store
            .get_group(group_id)
            .map(|g| g.name)
            .unwrap_or_default();
        // URL-encode the name so the join dialog can show it before joining.
        let mut encoded = String::new();
        for byte in group_name.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    encoded.push(byte as char)
                }
                _ => encoded.push_str(&format!("%{byte:02X}")),
            }
        }
        let link = format!("whisper://join?group={group_id}&token={token}&name={encoded}");
        let _ = self
            .send(peer_id, ServerMessage::GroupJoinLink { link })
            .await;
    }

    /// Join a group via a shareable join link. The secret token authorizes
    /// the join; on success the caller is added to the roster and every other
    /// member gets the usual `group_member_added` fan-out (so they share their
    /// Megolm keys with the newcomer).
    pub(crate) async fn join_group(&self, peer_id: &str, ip: &str, group_id: &str, token: &str) {
        if !self.take_group_slot(peer_id, ip).await {
            return;
        }
        if !self.inner.store.is_valid_join_token(group_id, token) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "invalid_join_token".into(),
                    },
                )
                .await;
            return;
        }
        if self.inner.store.is_group_member(group_id, peer_id) {
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "already_member".into(),
                    },
                )
                .await;
            return;
        }
        match self
            .inner
            .store
            .add_group_member(group_id, peer_id, unix_now())
        {
            Ok(()) => {
                tracing::info!(peer = %peer_id, group = %group_id, "joined group via link");
                let group_name = self
                    .inner
                    .store
                    .get_group(group_id)
                    .map(|g| g.name)
                    .unwrap_or_default();
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::GroupJoinOk {
                            group_id: group_id.to_string(),
                            group_name,
                        },
                    )
                    .await;
                let members = self.inner.store.list_group_members(group_id);
                for member in members {
                    if member == peer_id {
                        continue;
                    }
                    let _ = self
                        .send(
                            &member,
                            ServerMessage::GroupMemberAdded {
                                group_id: group_id.to_string(),
                                peer_id: peer_id.to_string(),
                            },
                        )
                        .await;
                }
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, group = %group_id, "failed to join via link: {err}");
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
                tracing::info!(peer = %peer_id, group = %group_id, "group member left");
                // Confirm to the leaver...
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::GroupMemberLeft {
                            group_id: group_id.to_string(),
                            peer_id: peer_id.to_string(),
                        },
                    )
                    .await;
                // ...and fan the roster change out to every remaining member so
                // their member counts and rosters stay in sync (mirrors the
                // add/remove fan-out). Offline members catch up on the next
                // `get_group_info` round-trip.
                let members = self.inner.store.list_group_members(group_id);
                for member in members {
                    let _ = self
                        .send(
                            &member,
                            ServerMessage::GroupMemberLeft {
                                group_id: group_id.to_string(),
                                peer_id: peer_id.to_string(),
                            },
                        )
                        .await;
                }
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
                    avatar_url: group.avatar_hash.map(|hash| format!("/media/{hash}")),
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
                tracing::info!(peer = %peer_id, group = %group_id, promoted = %target, "member promoted to admin");
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
                tracing::info!(peer = %peer_id, group = %group_id, demoted = %target, "admin demoted to member");
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
                tracing::info!(peer = %peer_id, group = %group_id, removed = %target, "group member removed");
                // The reply to the owner carries the full roster-change payload.
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::GroupMemberRemoved {
                            group_id: group_id.to_string(),
                            peer_id: target.to_string(),
                        },
                    )
                    .await;
                // Push the same removal to the removed peer (when online) so
                // their client drops the group instead of keeping a stale
                // roster entry. Offline members learn about the removal on
                // their next `get_group_info` round-trip (which rejects
                // non-members), so this MVP push only covers the online case.
                let _ = self
                    .send(
                        target,
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

    /// Transfer group ownership to `new_owner` (`transfer_ownership`).
    ///
    /// Permissions:
    /// - Only the current group owner may transfer. An admin or member gets
    ///   `not_owner`.
    /// - `new_owner` must be a member of the group, else `not_a_member`.
    ///
    /// On success the old owner becomes an admin, `new_owner` becomes the
    /// owner and the group's `owner_peer_id` is updated accordingly.
    pub(crate) async fn transfer_ownership(
        &self,
        peer_id: &str,
        ip: &str,
        group_id: &str,
        new_owner: &str,
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
        if !self.inner.store.is_group_member(group_id, new_owner) {
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

        match self.inner.store.transfer_ownership(group_id, new_owner) {
            Ok(()) => {
                tracing::info!(peer = %peer_id, group = %group_id, new_owner = %new_owner, "group ownership transferred");
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::OwnershipTransferred {
                            group_id: group_id.to_string(),
                            new_owner_peer_id: new_owner.to_string(),
                        },
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, group = %group_id, "failed to transfer ownership: {err}");
            }
        }
    }

    /// Set a group's avatar image (`set_group_avatar`).
    ///
    /// The avatar blob is a base64 image of at most 2 MiB, stored
    /// content-addressed as `media/<sha256>.bin` exactly like profile avatars.
    /// Only the group owner or an admin may change the avatar; a regular
    /// member gets `not_admin`.
    pub(crate) async fn set_group_avatar(
        &self,
        peer_id: &str,
        ip: &str,
        group_id: &str,
        avatar_b64: &str,
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

        // Decode and size-check the avatar before touching the database. The
        // blob itself is only written to disk once the permission checks above
        // passed, so a rejected request never leaves a dangling file.
        let bytes = match Self::decode_avatar(avatar_b64) {
            Ok(bytes) => bytes,
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
        };
        let hash = match Self::store_avatar(&self.inner.media_dir, &bytes) {
            Ok(hash) => hash,
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
        };

        match self.inner.store.set_group_avatar_hash(group_id, &hash) {
            Ok(()) => {
                tracing::info!(peer = %peer_id, group = %group_id, "group avatar set");
                let _ = self
                    .send(
                        peer_id,
                        ServerMessage::GroupAvatarSet {
                            group_id: group_id.to_string(),
                        },
                    )
                    .await;
                // Fan the change out to every other member so their chat list
                // and group header pick up the new photo without waiting for a
                // get_group_info round-trip.
                let members = self.inner.store.list_group_members(group_id);
                for member in members {
                    if member == peer_id {
                        continue;
                    }
                    let _ = self
                        .send(
                            &member,
                            ServerMessage::GroupAvatarSet {
                                group_id: group_id.to_string(),
                            },
                        )
                        .await;
                }
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, group = %group_id, "failed to persist group avatar: {err}");
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
        let mut fanned = 0;
        for member in &members {
            if member == peer_id {
                continue;
            }
            let mut copy = envelope.clone();
            copy.recipient = member.clone();
            self.deliver_one(&copy).await;
            fanned += 1;
        }
        tracing::debug!(sender = %peer_id, group = %group_id, recipients = fanned, "group message fanned out");

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
    use crate::relay::test_utils::{env, make_contacts, online_peer, read_reply};
    use base64::Engine;
    use sha2::{Digest, Sha256};

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
        make_contacts(&relay, &alice.peer_id(), &bob.peer_id());

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
        make_contacts(&relay, &alice.peer_id(), &bob.peer_id());
        make_contacts(&relay, &alice.peer_id(), &carol.peer_id());

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
        // Both members are online, so the member-add fan-out queued a
        // `group_member_added` push on each socket; drain them so the reads
        // below see only the fan-out envelope.
        drain(&mut bob_rx);
        drain(&mut carol_rx);

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
        make_contacts(&relay, &alice.peer_id(), &bob.peer_id());

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
        make_contacts(&relay, &alice.peer_id(), &bob.peer_id());

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
        make_contacts(&relay, &alice.peer_id(), &bob.peer_id());

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
    /// in it; returns the group id. `alice` must already be an accepted contact
    /// of both `bob` and `carol` (the relay refuses to add strangers), so the
    /// contacts are established first.
    async fn role_group(
        relay: &Relay,
        alice_rx: &mut mpsc::UnboundedReceiver<WsMessage>,
        alice: &Identity,
        bob: &Identity,
        carol: &Identity,
    ) -> String {
        make_contacts(relay, &alice.peer_id(), &bob.peer_id());
        make_contacts(relay, &alice.peer_id(), &carol.peer_id());
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

    /// Drop every queued message of a member's channel. The member-add fan-out
    /// pushes `group_member_added` to online members, so tests that then read
    /// a member's socket must first drain those roster pushes.
    fn drain(rx: &mut mpsc::UnboundedReceiver<WsMessage>) {
        while rx.try_recv().is_ok() {}
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
        drain(&mut bob_rx);

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
        drain(&mut bob_rx);

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
        drain(&mut bob_rx);
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
        drain(&mut bob_rx);
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

        relay
            .transfer_ownership(&alice.peer_id(), "127.0.0.1", "ghost", &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("group_not_found"));
    }

    #[tokio::test]
    async fn transfer_ownership_swaps_owner_and_old_owner_becomes_admin() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;
        online_peer(&relay, &carol).await;
        let group_id = role_group(&relay, &mut alice_rx, &alice, &bob, &carol).await;
        drain(&mut bob_rx);

        // The owner transfers ownership to bob.
        relay
            .transfer_ownership(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("ownership_transferred"));
        assert_eq!(reply["group_id"].as_str(), Some(group_id.as_str()));
        assert_eq!(
            reply["new_owner_peer_id"].as_str(),
            Some(bob.peer_id().as_str())
        );

        // Bob is now the owner, alice is an admin, carol stays a member.
        assert_eq!(
            relay
                .inner
                .store
                .get_member_role(&group_id, &bob.peer_id())
                .as_deref(),
            Some("owner")
        );
        assert_eq!(
            relay
                .inner
                .store
                .get_member_role(&group_id, &alice.peer_id())
                .as_deref(),
            Some("admin")
        );
        assert_eq!(
            relay
                .inner
                .store
                .get_member_role(&group_id, &carol.peer_id())
                .as_deref(),
            Some("member")
        );
        assert_eq!(
            relay
                .inner
                .store
                .get_group(&group_id)
                .unwrap()
                .owner_peer_id,
            bob.peer_id()
        );

        // The new owner shows up as the owner in group_info.
        relay
            .get_group_info(&alice.peer_id(), "127.0.0.1", &group_id)
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(
            reply["owner_peer_id"].as_str(),
            Some(bob.peer_id().as_str())
        );

        // The old owner (now an admin) can no longer transfer.
        relay
            .transfer_ownership(&alice.peer_id(), "127.0.0.1", &group_id, &carol.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_owner"));

        // Bob (the new owner) can transfer back.
        relay
            .transfer_ownership(&bob.peer_id(), "127.0.0.1", &group_id, &alice.peer_id())
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("ownership_transferred"));
        assert_eq!(
            relay
                .inner
                .store
                .get_member_role(&group_id, &alice.peer_id())
                .as_deref(),
            Some("owner")
        );
        assert_eq!(
            relay
                .inner
                .store
                .get_member_role(&group_id, &bob.peer_id())
                .as_deref(),
            Some("admin")
        );
    }

    #[tokio::test]
    async fn transfer_ownership_rejects_non_owner_actor() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;
        online_peer(&relay, &carol).await;
        let group_id = role_group(&relay, &mut alice_rx, &alice, &bob, &carol).await;
        drain(&mut bob_rx);

        // A plain member cannot transfer ownership.
        relay
            .transfer_ownership(&bob.peer_id(), "127.0.0.1", &group_id, &carol.peer_id())
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_owner"));
        assert_eq!(
            relay
                .inner
                .store
                .get_member_role(&group_id, &alice.peer_id())
                .as_deref(),
            Some("owner"),
            "a rejected transfer must not touch the roles"
        );
    }

    #[tokio::test]
    async fn transfer_ownership_rejects_non_member_target() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let dave = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        online_peer(&relay, &bob).await;
        online_peer(&relay, &dave).await;
        let group_id = role_group(&relay, &mut alice_rx, &alice, &bob, &Identity::new()).await;

        // dave is not a member of the group.
        relay
            .transfer_ownership(&alice.peer_id(), "127.0.0.1", &group_id, &dave.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_a_member"));
        assert_eq!(
            relay
                .inner
                .store
                .get_member_role(&group_id, &alice.peer_id())
                .as_deref(),
            Some("owner"),
            "a rejected transfer must leave the owner untouched"
        );
    }

    #[tokio::test]
    async fn add_group_member_pushes_to_other_online_members() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;
        let _carol_rx = online_peer(&relay, &carol).await;
        make_contacts(&relay, &alice.peer_id(), &bob.peer_id());
        make_contacts(&relay, &alice.peer_id(), &carol.peer_id());

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();

        // Alice adds bob: the only other member (alice) gets the push.
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("group_member_added"));

        // Alice adds carol: bob (an existing member) receives the push too.
        relay
            .add_group_member(&alice.peer_id(), "127.0.0.1", &group_id, &carol.peer_id())
            .await;
        let alice_reply = read_reply(&mut alice_rx);
        assert_eq!(alice_reply["type"].as_str(), Some("group_member_added"));
        assert_eq!(
            alice_reply["peer_id"].as_str(),
            Some(carol.peer_id().as_str())
        );
        let bob_reply = read_reply(&mut bob_rx);
        assert_eq!(bob_reply["type"].as_str(), Some("group_member_added"));
        assert_eq!(bob_reply["group_id"].as_str(), Some(group_id.as_str()));
        assert_eq!(
            bob_reply["peer_id"].as_str(),
            Some(carol.peer_id().as_str()),
            "the push names the newly added member"
        );
    }

    #[tokio::test]
    async fn remove_member_pushes_to_the_removed_peer() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;
        make_contacts(&relay, &alice.peer_id(), &bob.peer_id());

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

        // Alice removes bob: the owner gets the reply AND bob (online)
        // receives the same group_member_removed push.
        relay
            .remove_member(&alice.peer_id(), "127.0.0.1", &group_id, &bob.peer_id())
            .await;
        let alice_reply = read_reply(&mut alice_rx);
        assert_eq!(alice_reply["type"].as_str(), Some("group_member_removed"));

        let bob_reply = read_reply(&mut bob_rx);
        assert_eq!(bob_reply["type"].as_str(), Some("group_member_removed"));
        assert_eq!(bob_reply["group_id"].as_str(), Some(group_id.as_str()));
        assert_eq!(
            bob_reply["peer_id"].as_str(),
            Some(bob.peer_id().as_str()),
            "the push identifies the removed peer so the client can drop the group"
        );
    }

    #[tokio::test]
    async fn set_group_avatar_roundtrip_exposes_avatar_url() {
        let store = Store::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "whisper-relay-group-media-{}",
            uuid::Uuid::new_v4()
        ));
        let relay = Relay::with_parts(
            store,
            dir.clone(),
            RateLimiter::new(100.0, 0.0),
            RateLimiter::new(100.0, 0.0),
            RateLimiter::new(100.0, 0.0),
        );
        let alice = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;

        relay
            .create_group(&alice.peer_id(), "127.0.0.1", "Squad")
            .await;
        let group_id = read_reply(&mut alice_rx)["group_id"]
            .as_str()
            .expect("group_id must be set")
            .to_string();

        let png: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d,
        ];
        let encoded = base64::engine::general_purpose::STANDARD.encode(png);

        relay
            .set_group_avatar(&alice.peer_id(), "127.0.0.1", &group_id, &encoded)
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("group_avatar_set"));

        let digest = Sha256::digest(png);
        let hash = Relay::hex_encode(&digest);
        assert!(
            dir.join(format!("{hash}.bin")).exists(),
            "the group avatar blob must be written to the media directory"
        );
        assert_eq!(
            relay
                .inner
                .store
                .get_group_avatar_hash(&group_id)
                .as_deref(),
            Some(hash.as_str())
        );

        // get_group_info surfaces the avatar as a public /media/{hash} URL.
        relay
            .get_group_info(&alice.peer_id(), "127.0.0.1", &group_id)
            .await;
        let info = read_reply(&mut alice_rx);
        assert_eq!(info["type"].as_str(), Some("group_info"));
        assert_eq!(
            info["avatar_url"].as_str(),
            Some(format!("/media/{hash}").as_str())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn set_group_avatar_requires_owner_or_admin_and_valid_input() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 100.0, 0.0);
        let alice = Identity::new();
        let bob = Identity::new();
        let carol = Identity::new();
        let mut alice_rx = online_peer(&relay, &alice).await;
        let mut bob_rx = online_peer(&relay, &bob).await;
        let mut carol_rx = online_peer(&relay, &carol).await;
        make_contacts(&relay, &alice.peer_id(), &bob.peer_id());

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

        // A plain member cannot change the avatar.
        relay
            .set_group_avatar(&bob.peer_id(), "127.0.0.1", &group_id, "aGVsbG8=")
            .await;
        let reply = read_reply(&mut bob_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_admin"));

        // Invalid base64 / oversized input is rejected with invalid_avatar.
        relay
            .set_group_avatar(&alice.peer_id(), "127.0.0.1", &group_id, "not base64!")
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("invalid_avatar"));

        // An unknown group is rejected with group_not_found.
        relay
            .set_group_avatar(&alice.peer_id(), "127.0.0.1", "ghost", "aGVsbG8=")
            .await;
        let reply = read_reply(&mut alice_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("group_not_found"));

        // A non-member cannot change the avatar either.
        relay
            .set_group_avatar(&carol.peer_id(), "127.0.0.1", &group_id, "aGVsbG8=")
            .await;
        let reply = read_reply(&mut carol_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("not_a_member"));
        assert_eq!(
            relay.inner.store.get_group_avatar_hash(&group_id),
            None,
            "rejected requests must never persist an avatar"
        );
    }
}
