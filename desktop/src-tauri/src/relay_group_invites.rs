//! Group invites: invite a contact to a group, accept/decline, list.
//!
//! Unlike `add_group_member` (which adds a peer directly), an invite is a
//! pending proposal: the relay stores it, pushes `group_invite_received` to
//! the invitee, and only adds them to the roster once they accept. The
//! inviter learns the outcome via `group_invite_accepted` / `group_invite_declined`.

use super::*;

impl RelayClient {
    /// Invite `peer_id` to `group_id` (owner/admin only). The invitee is NOT
    /// added to the roster until they accept.
    pub async fn send_group_invite(&self, group_id: &str, peer_id: &str) -> Result<(), RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_group_invite_op)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::GroupInvite {
            group_id: group_id.to_string(),
            peer_id: peer_id.to_string(),
        }) {
            mutex_guard(&self.inner.pending_group_invite_op)?.pop_back();
            return Err(err);
        }
        tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::GroupTimeout)?
            .map_err(|_| RelayError::GroupRequestFailed)?
    }

    /// Accept a pending invite to `group_id`: the relay adds us to the roster.
    pub async fn accept_group_invite(&self, group_id: &str) -> Result<(), RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_group_invite_op)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::GroupInviteAccept {
            group_id: group_id.to_string(),
        }) {
            mutex_guard(&self.inner.pending_group_invite_op)?.pop_back();
            return Err(err);
        }
        tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::GroupTimeout)?
            .map_err(|_| RelayError::GroupRequestFailed)??;
        // We were not a member while the invite was pending, so the group is
        // not in our local roster yet — fetch it so the chat list shows it
        // with the correct member count.
        let _ = self.get_group_info(group_id).await;
        Ok(())
    }

    /// Decline a pending invite to `group_id`.
    pub async fn decline_group_invite(&self, group_id: &str) -> Result<(), RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_group_invite_op)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::GroupInviteDecline {
            group_id: group_id.to_string(),
        }) {
            mutex_guard(&self.inner.pending_group_invite_op)?.pop_back();
            return Err(err);
        }
        tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::GroupTimeout)?
            .map_err(|_| RelayError::GroupRequestFailed)?
    }

    /// Fetch the pending group invites for this identity.
    pub async fn get_group_invites(&self) -> Result<Vec<GroupInviteInfo>, RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_group_invites_list)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::GetGroupInvites) {
            mutex_guard(&self.inner.pending_group_invites_list)?.pop_back();
            return Err(err);
        }
        tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::GroupTimeout)?
            .map_err(|_| RelayError::GroupRequestFailed)?
    }

    /// Get (or create) the group's shareable join link (`whisper://join?..`).
    /// Any member may ask; the link lets anyone join.
    pub async fn get_group_join_link(&self, group_id: &str) -> Result<String, RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_group_join_link)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::GetGroupJoinLink {
            group_id: group_id.to_string(),
        }) {
            mutex_guard(&self.inner.pending_group_join_link)?.pop_back();
            return Err(err);
        }
        tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::GroupTimeout)?
            .map_err(|_| RelayError::GroupRequestFailed)?
    }

    /// Join a group via its shareable join link.
    pub async fn join_group(&self, group_id: &str, token: &str) -> Result<(), RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_group_join_op)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::JoinGroup {
            group_id: group_id.to_string(),
            token: token.to_string(),
        }) {
            mutex_guard(&self.inner.pending_group_join_op)?.pop_back();
            return Err(err);
        }
        tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::GroupTimeout)?
            .map_err(|_| RelayError::GroupRequestFailed)??;
        // We were not a member, so the group is not in the local roster yet —
        // fetch it so the chat list shows it with the correct member count.
        let _ = self.get_group_info(group_id).await;
        Ok(())
    }

    /// A `group_invite_received` push: remember the invite and surface it to
    /// the UI (toast + the Sidebar Invites section).
    pub(crate) fn handle_group_invite_received(
        &self,
        group_id: &str,
        group_name: &str,
        inviter_peer_id: &str,
    ) -> Result<(), RelayError> {
        let invite = GroupInviteInfo {
            group_id: group_id.to_string(),
            group_name: group_name.to_string(),
            inviter_peer_id: inviter_peer_id.to_string(),
        };
        {
            let mut invites = write_guard(&self.inner.group_invites_incoming)?;
            if !invites.iter().any(|i| i.group_id == group_id) {
                invites.push(invite.clone());
            }
        }
        let _ = self.inner.app.emit("group-invite-received", invite);
        Ok(())
    }

    /// A `group_invites` snapshot: resolve the in-flight request and cache the
    /// list for the UI.
    pub(crate) fn handle_group_invites(
        &self,
        invites: Vec<GroupInviteInfo>,
    ) -> Result<(), RelayError> {
        if let Some(tx) = mutex_guard(&self.inner.pending_group_invites_list)?.pop_front() {
            let _ = tx.send(Ok(invites.clone()));
        }
        *write_guard(&self.inner.group_invites_incoming)? = invites;
        Ok(())
    }

    /// A `group_invite_accepted` / `group_invite_declined` / sent/ok ack for
    /// the INVITER side: resolve the in-flight command.
    pub(crate) fn handle_group_op_ack(&self) -> Result<(), RelayError> {
        if let Some(tx) = mutex_guard(&self.inner.pending_group_invite_op)?.pop_front() {
            let _ = tx.send(Ok(()));
        }
        Ok(())
    }

    /// The invitee accepted our invite: refresh the roster so the new member
    /// shows up in the group panel.
    pub(crate) fn handle_group_invite_accepted(
        &self,
        group_id: &str,
        peer_id: &str,
    ) -> Result<(), RelayError> {
        self.handle_group_op_ack()?;
        self.spawn_group_info_refresh(group_id);
        let _ = self.inner.app.emit(
            "group-invite-outcome",
            GroupInviteOutcomeEvent {
                group_id: group_id.to_string(),
                peer_id: peer_id.to_string(),
                accepted: true,
            },
        );
        Ok(())
    }

    /// The invitee declined our invite.
    pub(crate) fn handle_group_invite_declined(
        &self,
        group_id: &str,
        peer_id: &str,
    ) -> Result<(), RelayError> {
        self.handle_group_op_ack()?;
        let _ = self.inner.app.emit(
            "group-invite-outcome",
            GroupInviteOutcomeEvent {
                group_id: group_id.to_string(),
                peer_id: peer_id.to_string(),
                accepted: false,
            },
        );
        Ok(())
    }

    /// A `group_join_link` reply: resolve the in-flight request.
    pub(crate) fn handle_group_join_link(&self, link: String) -> Result<(), RelayError> {
        if let Some(tx) = mutex_guard(&self.inner.pending_group_join_link)?.pop_front() {
            let _ = tx.send(Ok(link));
        }
        Ok(())
    }

    /// A `group_join_ok` reply: resolve the in-flight join and pull the group
    /// metadata so the chat list shows it.
    pub(crate) fn handle_group_join_ok(
        &self,
        group_id: &str,
        group_name: &str,
    ) -> Result<(), RelayError> {
        if let Some(tx) = mutex_guard(&self.inner.pending_group_join_op)?.pop_front() {
            let _ = tx.send(Ok(()));
        }
        self.spawn_group_info_refresh(group_id);
        let _ = group_name;
        Ok(())
    }
}

/// Payload of the `group-invite-outcome` event (the inviter learns the result).
#[derive(Debug, Clone, Serialize)]
pub struct GroupInviteOutcomeEvent {
    pub group_id: String,
    pub peer_id: String,
    pub accepted: bool,
}
