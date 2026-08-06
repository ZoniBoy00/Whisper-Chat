//! Group chat: Megolm sessions, roster operations and group envelope handling.
//!
//! Groups use vodozemac's Megolm session protocol. The group creator builds an
//! outbound session, shares its `session_key` to every member over the existing
//! 1:1 Double Ratchet channel, and fans encrypted group envelopes out via the
//! relay's `send_group_message` fan-out.
//!
//! This module owns every group `impl RelayClient` block: roster operations
//! (create/get/promote/demote/remove/leave), Megolm key sharing, group envelope
//! decryption ([`ingest_group`](RelayClient::ingest_group),
//! [`handle_group_key`](RelayClient::handle_group_key)) and the group arms of
//! the inbound server-message dispatch. The shared group state structs
//! [`GroupInfoState`] and [`GroupKeyPayload`] live here too; the relay core
//! (`super`) references them for the in-memory group maps and persistence.

use super::*;

/// Internal, in-memory group state. Not serializable: it owns the (secret)
/// Megolm outbound session used to encrypt this identity's group messages.
pub(crate) struct GroupInfoState {
    /// Public group name.
    pub(crate) name: String,
    /// Cached member roster (server-authoritative snapshot).
    pub(crate) members: Vec<GroupMember>,
    /// This identity's role in the group, when known.
    pub(crate) my_role: Option<String>,
    /// Server avatar path ("/media/{hash}"), when the group has a photo.
    pub(crate) avatar_url: Option<String>,
    /// Our own outbound Megolm session. In the multi-sender model EVERY member
    /// holds one (created automatically when they first receive the group's
    /// key), so each member can send to the group.
    pub(crate) outbound: Option<OutboundGroup>,
    /// Whether the outbound session key has been shared with the other members
    /// during THIS process run. Hydrated sessions (after a restart) have not
    /// shared yet — the recipients may hold a stale/absent inbound session, so
    /// the key is re-shared before the first group send.
    pub(crate) key_shared_this_session: bool,
}

/// The plaintext JSON of a Megolm session-key share. A member encrypts it
/// inside an ordinary 1:1 Double Ratchet message so the relay never sees the
/// key; the recipient parses it and builds an [`InboundGroup`] keyed by the
/// sharing member's peer ID. `sender` defaults to empty for robustness against
/// older shares; the 1:1 envelope's authenticated sender is then used.
#[derive(Debug, Deserialize)]
pub(crate) struct GroupKeyPayload {
    /// Always "group_key"; distinguishes the share from ordinary text.
    pub(crate) kind: String,
    /// The relay-assigned group ID the key belongs to.
    pub(crate) group_id: String,
    /// The base64 Megolm session key (secret key material).
    pub(crate) session_key: String,
    /// The public group name, used to surface the group in the chat list.
    pub(crate) group_name: String,
    /// The member who shared this key (their own outbound session).
    #[serde(default)]
    pub(crate) sender: String,
}

/// Options for recording a group plaintext payload as an outgoing message.
///
/// `display_text` is the human-readable text recorded for the optimistic
/// message — the plaintext itself is a tagged JSON payload and must never be
/// shown raw. `message_id` is the pre-decided id that travels inside the
/// encrypted payload (so recipients store the message under the same id); when
/// absent the `client_id` scheme is used.
pub(crate) struct GroupSend {
    /// Whether the payload is recorded as an ordinary outgoing chat message.
    pub record: bool,
    pub client_id: String,
    pub quote: Option<e2ee_core::Quote>,
    pub message_id: Option<String>,
    pub display_text: Option<String>,
}

impl RelayClient {
    /// Create a group on the relay, build the Megolm outbound session, register
    /// the group locally and share its `session_key` to every member over the
    /// existing 1:1 Double Ratchet channel.
    ///
    /// The outbound session is built and persisted BEFORE the roster is
    /// mutated, so a member-add failure can never leave this identity with a
    /// group that has no outbound session. Returns the relay-assigned group ID.
    /// Member adds and key sharing are both best-effort per member: a member
    /// that is rate-limited, offline or unreachable is skipped so one failure
    /// cannot abort group creation.
    pub async fn create_group(
        &self,
        name: &str,
        member_ids: Vec<String>,
    ) -> Result<String, RelayError> {
        let my_peer_id = self.my_peer_id()?;
        if member_ids.iter().any(|m| m == &my_peer_id) {
            return Err(RelayError::InvalidPeer(my_peer_id));
        }

        // 1) Ask the relay for a fresh group and wait for the group_created
        //    reply carrying the group ID.
        let group_id = {
            let (tx, rx) = oneshot::channel();
            mutex_guard(&self.inner.pending_group_created)?.push_back(tx);
            if let Err(err) = self.send_json(&ClientMessage::CreateGroup {
                name: name.to_string(),
            }) {
                mutex_guard(&self.inner.pending_group_created)?.pop_back();
                return Err(err);
            }
            tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx)
                .await
                .map_err(|_| RelayError::GroupTimeout)?
                .map_err(|_| RelayError::GroupRequestFailed)??
        };

        // 2) Build the Megolm outbound session and register the group locally
        //    BEFORE touching the roster. The relay confirms `group_created`
        //    (and `handle_group_created` caches the group) as soon as the
        //    reply arrives, so if a member add failed afterwards the group
        //    would otherwise linger with `outbound: None` and every send would
        //    fail with "group X has no outbound session". Building and
        //    persisting the pickle up front keeps the owner's state consistent
        //    no matter how the roster mutations go.
        let outbound = OutboundGroup::new();
        let session_key = outbound.session_key();
        let mut members = vec![GroupMember {
            peer_id: my_peer_id.clone(),
            role: "owner".to_string(),
        }];
        members.extend(member_ids.iter().map(|peer_id| GroupMember {
            peer_id: peer_id.clone(),
            role: "member".to_string(),
        }));
        write_guard(&self.inner.groups)?.insert(
            group_id.clone(),
            GroupInfoState {
                name: name.to_string(),
                members: members.clone(),
                my_role: Some("owner".to_string()),
                avatar_url: None,
                outbound: Some(outbound),
                // The key is shared to every member during group creation.
                key_shared_this_session: true,
            },
        );
        // Surface the group in the chat list (the group ID acts as a contact
        // with the group name as its display name).
        self.remember_contact_name(&group_id, name)?;
        self.save_group_sessions()?;

        // 3) Add every member to the roster (owner or member may add, and the
        //    creator is the owner). Best-effort: a member whose add request is
        //    rate-limited, times out or otherwise fails must not abort group
        //    creation — the group is already created and fully functional for
        //    the owner, and the member can be added again later. A timed-out
        //    waiter is removed so a request that never receives a reply cannot
        //    misalign the FIFO pending queue for the remaining members.
        for member in &member_ids {
            let result = async {
                let (tx, rx) = oneshot::channel();
                mutex_guard(&self.inner.pending_group_member_added)?.push_back(tx);
                if let Err(err) = self.send_json(&ClientMessage::AddGroupMember {
                    group_id: group_id.clone(),
                    peer_id: member.clone(),
                }) {
                    mutex_guard(&self.inner.pending_group_member_added)?.pop_back();
                    return Err(err);
                }
                match tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx).await {
                    Err(_) => {
                        mutex_guard(&self.inner.pending_group_member_added)?.pop_front();
                        Err(RelayError::GroupTimeout)
                    }
                    Ok(inner) => inner
                        .map_err(|_| RelayError::GroupRequestFailed)
                        .and_then(|reply| reply),
                }
            }
            .await;
            if let Err(err) = result {
                tracing::warn!(peer = %member, group = %group_id, error = %err, "failed to add member to group");
            }
        }

        // 4) Share the session key to every member over 1:1 sessions. Each
        //    member starts with `start_chat` (which also sends the greeting)
        //    when no session exists yet.
        for member in &member_ids {
            let result = async {
                if !mutex_guard(&self.inner.sessions)?.contains_key(member) {
                    self.start_chat(member).await?;
                }
                self.send_group_key(member, &group_id, &session_key, name)
            }
            .await;
            if let Err(err) = result {
                tracing::warn!(peer = %member, group = %group_id, error = %err, "failed to share group key");
            }
        }

        // 5) Refresh the roster from the relay so the member count reflects
        //    exactly the members the server accepted (best-effort adds may
        //    have failed above and are not actually in the group).
        self.spawn_group_info_refresh(&group_id);

        Ok(group_id)
    }

    /// Fetch a group's public metadata and member roster from the relay.
    pub async fn get_group_info(&self, group_id: &str) -> Result<GroupInfo, RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_group_info)?.push_back((group_id.to_string(), tx));
        if let Err(err) = self.send_json(&ClientMessage::GetGroupInfo {
            group_id: group_id.to_string(),
        }) {
            mutex_guard(&self.inner.pending_group_info)?.retain(|(gid, _)| gid != group_id);
            return Err(err);
        }

        let info = match tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx).await {
            Err(_) => {
                // The reply never arrived. Drop this group's waiter so a late
                // reply can never misroute a later request for another group.
                mutex_guard(&self.inner.pending_group_info)?.retain(|(gid, _)| gid != group_id);
                return Err(RelayError::GroupTimeout);
            }
            Ok(inner) => inner.map_err(|_| RelayError::GroupRequestFailed)??,
        };

        // Cache the fresh roster locally so the chat list and group panel stay
        // consistent without a full state refresh.
        let my_peer_id = self.my_peer_id()?;
        let my_role = info
            .members
            .iter()
            .find(|m| m.peer_id == my_peer_id)
            .map(|m| m.role.clone());
        {
            let mut groups = write_guard(&self.inner.groups)?;
            if let Some(group) = groups.get_mut(group_id) {
                group.name = info.name.clone();
                group.members = info.members.clone();
                group.my_role = my_role.clone();
                group.avatar_url = info.avatar_url.clone();
            }
        }
        // Persist the display metadata (name, avatar) so it survives restarts.
        self.persist_group_meta(group_id, &info.name, info.avatar_url.as_deref());
        Ok(GroupInfo { my_role, ..info })
    }

    /// Add an existing peer to a group's roster AFTER creation. Only the group
    /// owner or an admin may add members.
    ///
    /// The relay fans a `group_member_added` push to every existing member; the
    /// inbound handler ([`RelayClient::handle_group_member_added`]) updates the
    /// roster and shares this identity's own Megolm outbound session key to the
    /// newcomer over a 1:1 Double Ratchet channel, so the multi-sender model
    /// keeps working (every member shares its own key to every new member).
    /// The cached roster is refreshed so roles and the member count update
    /// immediately.
    pub async fn add_group_member(&self, group_id: &str, peer_id: &str) -> Result<(), RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_group_member_added)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::AddGroupMember {
            group_id: group_id.to_string(),
            peer_id: peer_id.to_string(),
        }) {
            mutex_guard(&self.inner.pending_group_member_added)?.pop_back();
            return Err(err);
        }
        match tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx).await {
            Err(_) => {
                // The reply never arrived. Drop the waiter so a late reply can
                // never misalign the FIFO pending queue for later requests.
                mutex_guard(&self.inner.pending_group_member_added)?.pop_front();
                return Err(RelayError::GroupTimeout);
            }
            Ok(inner) => inner.map_err(|_| RelayError::GroupRequestFailed)??,
        }
        // The roster push already appended the member locally; refresh roles
        // from the server in the background.
        self.spawn_group_info_refresh(group_id);
        Ok(())
    }

    /// Set a group's avatar image (`avatar_b64`, base64, ≤2 MB). The relay
    /// stores the blob content-addressed and exposes it as `avatar_url` in the
    /// group metadata, so a `get_group_info` refresh re-renders the photo.
    /// Only the owner or an admin may change the avatar.
    pub async fn set_group_avatar(
        &self,
        group_id: &str,
        avatar_b64: &str,
    ) -> Result<(), RelayError> {
        self.group_op(ClientMessage::SetGroupAvatar {
            group_id: group_id.to_string(),
            avatar: avatar_b64.to_string(),
        })
        .await
    }

    /// Promote `peer_id` to a group admin. The relay only allows this for the
    /// group owner or an existing admin.
    pub async fn promote_member(&self, group_id: &str, peer_id: &str) -> Result<(), RelayError> {
        self.group_op(ClientMessage::PromoteMember {
            group_id: group_id.to_string(),
            peer_id: peer_id.to_string(),
        })
        .await
    }

    /// Demote `peer_id` from admin back to a member. The relay only allows the
    /// owner to demote, and never the owner themselves.
    pub async fn demote_member(&self, group_id: &str, peer_id: &str) -> Result<(), RelayError> {
        self.group_op(ClientMessage::DemoteMember {
            group_id: group_id.to_string(),
            peer_id: peer_id.to_string(),
        })
        .await
    }

    /// Remove `peer_id` from a group. The relay only allows the owner to
    /// remove members, and never the owner themselves.
    pub async fn remove_member(&self, group_id: &str, peer_id: &str) -> Result<(), RelayError> {
        self.group_op(ClientMessage::RemoveMember {
            group_id: group_id.to_string(),
            peer_id: peer_id.to_string(),
        })
        .await
    }

    /// Remove the caller from a group's roster.
    pub async fn leave_group(&self, group_id: &str) -> Result<(), RelayError> {
        self.group_op(ClientMessage::LeaveGroup {
            group_id: group_id.to_string(),
        })
        .await
    }

    /// Transfer group ownership to `peer_id`. The relay only allows the
    /// current owner to transfer; on success the old owner becomes an admin
    /// and `peer_id` takes over the owner role.
    pub async fn transfer_ownership(
        &self,
        group_id: &str,
        peer_id: &str,
    ) -> Result<(), RelayError> {
        self.group_op(ClientMessage::TransferOwnership {
            group_id: group_id.to_string(),
            new_owner_peer_id: peer_id.to_string(),
        })
        .await
    }

    /// Rename the group (owner/admin). The relay fans the new name to every
    /// member; refresh the roster so the chat list updates immediately.
    pub async fn rename_group(&self, group_id: &str, name: &str) -> Result<(), RelayError> {
        self.group_op(ClientMessage::RenameGroup {
            group_id: group_id.to_string(),
            name: name.to_string(),
        })
        .await
    }

    /// Send a promote/demote/remove/leave request and wait for its
    /// confirmation, then refresh the cached roster so the UI reflects the new
    /// membership immediately.
    async fn group_op(&self, message: ClientMessage) -> Result<(), RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_group_op)?.push_back(tx);
        if let Err(err) = self.send_json(&message) {
            mutex_guard(&self.inner.pending_group_op)?.pop_back();
            return Err(err);
        }
        tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::GroupTimeout)?
            .map_err(|_| RelayError::GroupRequestFailed)??;
        // Refresh the cached roster (best-effort: the group may no longer
        // exist from our point of view after a leave).
        if let ClientMessage::LeaveGroup { group_id } = message {
            self.forget_group(&group_id);
        } else if let ClientMessage::RemoveMember { group_id, .. } = message {
            // The removed member is no longer in the roster; refresh the rest.
            let _ = self.get_group_info(&group_id).await;
        } else if let ClientMessage::PromoteMember { group_id, .. }
        | ClientMessage::DemoteMember { group_id, .. } = message
        {
            let _ = self.get_group_info(&group_id).await;
        } else if let ClientMessage::TransferOwnership { group_id, .. } = message {
            // The roster (and our own role) changed: refresh so the chat list
            // and group panel reflect the new owner immediately.
            let _ = self.get_group_info(&group_id).await;
        } else if let ClientMessage::SetGroupAvatar { group_id, .. } = message {
            // The avatar changed: refresh so the chat list and header show the
            // new group photo.
            let _ = self.get_group_info(&group_id).await;
        }
        Ok(())
    }

    /// Drop all local group state after leaving a group: the outbound/inbound
    /// sessions and the contact-list entry. Messages stay in history but the
    /// group no longer appears as a conversation.
    fn forget_group(&self, group_id: &str) {
        if let Ok(mut groups) = write_guard(&self.inner.groups) {
            groups.remove(group_id);
        }
        if let Ok(mut inbound) = mutex_guard(&self.inner.inbound_groups) {
            // Multi-sender inbound sessions live in a nested map keyed by
            // (group_id, sender): dropping the outer key removes every
            // sender's session for this group at once.
            inbound.remove(group_id);
        }
        if let Ok(mut contacts) = write_guard(&self.inner.contacts) {
            contacts.retain(|c| c != group_id);
        }
        if let Ok(store) = self.store_guard() {
            if let Some(store) = store.as_ref() {
                let _ = store.delete_contact(group_id);
            }
        }
        let _ = self.save_group_sessions();
        // The UI keeps its own copy of the group in React state: without an
        // event it would keep showing the group (and every action on it would
        // fail with not_a_member). Emit so the UI drops it and can toast.
        let _ = self.inner.app.emit(
            "group-removed",
            GroupRemovedEvent {
                group_id: group_id.to_string(),
            },
        );
    }

    /// Kick off a best-effort background `get_group_info` for `group_id` so
    /// the roster (and therefore the member count shown by the chat list)
    /// refreshes without waiting for the user to open the group info panel.
    pub(crate) fn spawn_group_info_refresh(&self, group_id: &str) {
        let client = self.clone();
        let id = group_id.to_string();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = client.get_group_info(&id).await {
                tracing::warn!(group = %id, error = %err, "failed to refresh group roster");
            }
        });
    }

    /// Background-refresh the member roster of every known group. Called after
    /// every successful connect so groups restored from the store (whose
    /// rosters are empty until the first `get_group_info` round-trip) show a
    /// real member count shortly after startup or reconnect.
    ///
    /// While the roster is being refreshed the task also heals legacy groups:
    /// a group this identity is a member of but that has no outbound session
    /// (created before the multi-sender model, or joined before the join-time
    /// setup landed) gets an outbound session established in the background,
    /// so it is ready to send before the first message.
    pub(crate) fn refresh_group_rosters(&self) {
        let mut group_ids: Vec<String> = match read_guard(&self.inner.groups) {
            Ok(groups) => groups.keys().cloned().collect(),
            Err(_) => Vec::new(),
        };
        // Legacy groups may linger ONLY in the contact list: a group row in
        // the store without any persisted Megolm sessions is hydrated as a
        // contact but never enters the groups map, so the map-based loop below
        // would skip it and a group we left (or were removed from) would stay
        // visible forever. Group IDs are UUIDs (contain a dash), peer IDs are
        // 24 hex chars — include any dashed contact too.
        if let Ok(contacts) = read_guard(&self.inner.contacts) {
            for peer in contacts.iter() {
                if peer.contains('-') && !group_ids.contains(peer) {
                    group_ids.push(peer.clone());
                }
            }
        }
        for group_id in group_ids {
            let client = self.clone();
            let id = group_id;
            tauri::async_runtime::spawn(async move {
                if let Err(err) = client.refresh_group_roster_and_outbound(&id).await {
                    tracing::warn!(group = %id, error = %err, "failed to refresh group roster");
                }
            });
        }
    }

    /// Refresh one group's roster and, when this identity is a member with no
    /// outbound session yet (a legacy pre-multi-sender group, or a join whose
    /// setup never finished), establish the outbound session in the background
    /// so the group is sendable before the first message. Establishing only
    /// happens for actual roster members (`my_role != None`) so a stale local
    /// entry for a group we left is never resurrected and key shares are never
    /// spammed.
    async fn refresh_group_roster_and_outbound(&self, group_id: &str) -> Result<(), RelayError> {
        // The roster refresh populates `my_role`, which `needs_outbound_session`
        // depends on to decide whether this identity may establish.
        match self.get_group_info(group_id).await {
            Ok(_) => {}
            Err(RelayError::Relay(code)) if code == "not_a_member" || code == "group_not_found" => {
                // We are no longer a member of this group (left it, or were
                // removed while offline). Drop the stale local entry so a
                // re-hydrated legacy group does not linger in the chat list —
                // this also cleans up groups that predate the fix where
                // `forget_group` did not wipe the persisted inbound sessions.
                self.forget_group(group_id);
                return Ok(());
            }
            Err(err) => return Err(err),
        }
        if self.needs_outbound_session(group_id)? {
            let group_name = read_guard(&self.inner.groups)?
                .get(group_id)
                .map(|group| group.name.clone())
                .unwrap_or_default();
            self.establish_outbound_and_share(group_id, &group_name)
                .await?;
        }
        Ok(())
    }

    /// Whether `group_id` lacks an outbound session while this identity is a
    /// roster member — the exact condition a background establish heals.
    fn needs_outbound_session(&self, group_id: &str) -> Result<bool, RelayError> {
        let groups = read_guard(&self.inner.groups)?;
        Ok(match groups.get(group_id) {
            Some(group) => group.outbound.is_none() && group.my_role.is_some(),
            None => false,
        })
    }

    /// Megolm-encrypt an emoji reaction and fan it out to every group member.
    /// Reactions are not chat messages: they are not recorded as optimistic
    /// outgoing messages and do not carry an ack mapping (best-effort, like
    /// typing indicators).
    pub(crate) fn send_group_reaction(
        &self,
        group_id: &str,
        message_id: &str,
        emoji: &str,
        active: bool,
    ) -> Result<(), RelayError> {
        let payload = e2ee_core::ChatPayload::Reaction(e2ee_core::ReactionPayload::new(
            message_id, emoji, active,
        ));
        let bytes = serde_json::to_vec(&payload)?;
        self.send_group_payload(
            group_id,
            &bytes,
            GroupSend {
                record: false,
                client_id: String::new(),
                quote: None,
                message_id: None,
                display_text: None,
            },
        )
    }

    /// Megolm-encrypt a group typing indicator and fan it out to every member.
    /// The Megolm envelope is attributed to this identity, so recipients know
    /// exactly who is composing. Best-effort, never recorded.
    pub(crate) fn send_group_typing(
        &self,
        group_id: &str,
        is_typing: bool,
    ) -> Result<(), RelayError> {
        let payload = e2ee_core::ChatPayload::Typing(e2ee_core::TypingPayload::new(is_typing));
        let bytes = serde_json::to_vec(&payload)?;
        tracing::info!(group = %group_id, typing = %is_typing, "sending group typing indicator");
        self.send_group_payload(
            group_id,
            &bytes,
            GroupSend {
                record: false,
                client_id: String::new(),
                quote: None,
                message_id: None,
                display_text: None,
            },
        )
    }

    /// Megolm-encrypt a serialized plaintext payload and fan it out to every
    /// member. When `record` is set the payload is treated as an ordinary
    /// outgoing chat message (optimistic insertion, ack mapping, rollback on
    /// send failure); otherwise it is a best-effort control signal (reaction,
    /// typing) that is never recorded in the thread.
    pub(crate) fn send_group_payload(
        &self,
        group_id: &str,
        plaintext: &[u8],
        options: GroupSend,
    ) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        // Encrypt with our own outbound Megolm session. Every member owns one
        // in the multi-sender model (created on first group-key receipt or
        // healed by `ensure_outbound_session` before the first send), so the
        // `NoOutboundGroup` error here is only a defensive safety net.
        let ciphertext = {
            let mut groups = write_guard(&self.inner.groups)?;
            let group = groups
                .get_mut(group_id)
                .ok_or_else(|| RelayError::NoOutboundGroup(group_id.to_string()))?;
            let outbound = group
                .outbound
                .as_mut()
                .ok_or_else(|| RelayError::NoOutboundGroup(group_id.to_string()))?;
            outbound.encrypt(plaintext)
        };
        // The ratchet advanced, so persist the outbound session.
        self.save_group_sessions()?;

        let wire = Envelope::new(
            my_peer_id.clone(),
            group_id.to_string(),
            EnvelopeContent::Group {
                group_id: group_id.to_string(),
                ciphertext,
            },
        );
        let payload = BASE64.encode(serde_json::to_vec(&wire)?);
        let relay_envelope = RelayEnvelope {
            sender: my_peer_id.clone(),
            recipient: group_id.to_string(),
            payload,
            seq: 0, // replaced below with the allocated seq
        };

        let seq = self.next_seq();
        let recorded = if options.record {
            // Record the HUMAN-READABLE text (not the tagged JSON plaintext)
            // so the sender's own optimistic message renders normally.
            let shown_text = options
                .display_text
                .unwrap_or_else(|| String::from_utf8_lossy(plaintext).into_owned());
            let msg = match options.message_id {
                Some(id) => {
                    self.record_outgoing_with_id(group_id, id, &shown_text, options.quote)?
                }
                None => {
                    self.record_outgoing(group_id, &shown_text, &options.client_id, options.quote)?
                }
            };
            self.record_pending_ack(seq, &msg.id)?;
            Some(msg)
        } else {
            None
        };

        let mut envelope = relay_envelope;
        envelope.seq = seq;
        if let Err(err) = self.send_json(&ClientMessage::SendGroupMessage {
            group_id: group_id.to_string(),
            envelope,
        }) {
            let _ = mutex_guard(&self.inner.pending_acks)?.remove(&seq);
            if let Some(msg) = &recorded {
                if let Ok(mut messages) = write_guard(&self.inner.messages) {
                    if let Some(msgs) = messages.get_mut(group_id) {
                        msgs.retain(|m| m.id != msg.id);
                    }
                }
                if let Ok(store) = self.store_guard() {
                    if let Some(store) = store.as_ref() {
                        let _ = store.delete_message(&msg.id);
                    }
                }
            }
            return Err(err);
        }

        if let Some(msg) = recorded {
            let _ = self.inner.app.emit(
                "chat-message",
                ChatMessageEvent {
                    peer_id: group_id.to_string(),
                    message: msg,
                },
            );
        }
        Ok(())
    }

    /// Encrypt a `group_key` share (the Megolm session key + group name) inside
    /// the 1:1 session with `peer_id` and send it as an ordinary message. The
    /// recipient recognises the plaintext JSON and stores the inbound session
    /// (keyed by this identity's peer ID) instead of rendering it as a chat
    /// message.
    fn send_group_key(
        &self,
        peer_id: &str,
        group_id: &str,
        session_key: &str,
        group_name: &str,
    ) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        let payload = serde_json::json!({
            "kind": "group_key",
            "group_id": group_id,
            "session_key": session_key,
            "group_name": group_name,
            "sender": my_peer_id,
        });
        let (olm, session_id) = {
            let mut sessions = mutex_guard(&self.inner.sessions)?;
            let session = sessions
                .get_mut(peer_id)
                .ok_or_else(|| RelayError::NoSession(peer_id.to_string()))?;
            let session_id = session.session_id();
            let olm = session.encrypt(serde_json::to_vec(&payload)?)?;
            (olm, session_id)
        };
        self.save_sessions()?;
        let wire = Envelope::new(
            my_peer_id.clone(),
            peer_id.to_string(),
            EnvelopeContent::Message(Message::new(my_peer_id, session_id, olm)),
        );
        let seq = self.next_seq();
        self.send_wire(&wire, seq)
    }

    // ---------------------------------------------------------------------
    // Inbound group handling
    // ---------------------------------------------------------------------

    /// Decrypt a Megolm group envelope with the sender's inbound session and
    /// record it in the group's message thread. Envelopes from a sender whose
    /// session key we have not received yet are skipped defensively.
    pub(crate) fn ingest_group(
        &self,
        wire: &Envelope,
        group_id: &str,
    ) -> Result<Option<UIMessage>, RelayError> {
        let ciphertext = match &wire.content {
            EnvelopeContent::Group { ciphertext, .. } => ciphertext.clone(),
            _ => return Ok(None),
        };
        let plaintext = match self.decrypt_group(group_id, &wire.sender_peer_id, &ciphertext) {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        // The inbound Megolm ratchet advanced — persist it now, or a restart
        // would restore the session to a stale index and every later group
        // message (text, typing, reactions) would be rejected as out of order.
        self.save_group_sessions()?;
        // Group payloads use the same tagged envelope as 1:1 messages: an
        // emoji reaction is applied to the target message, a typing indicator
        // is surfaced with the writer's identity, anything else is a (possibly
        // quoting) text message. Legacy raw text parses as plain text too.
        match e2ee_core::parse_plaintext(&plaintext) {
            e2ee_core::ParsedPayload::Reaction(reaction) => {
                // No end-to-end read receipts for group messages in the MVP
                // model; reactions work exactly like their 1:1 counterparts.
                self.handle_reaction(
                    group_id,
                    &reaction.message_id,
                    &wire.sender_peer_id,
                    &reaction.emoji,
                    reaction.active,
                )?;
                Ok(None)
            }
            e2ee_core::ParsedPayload::Typing(typing) => {
                // The typing indicator carries the WRITER's peer id so the UI
                // can render "ZoniBoy typing…" (or "3 members typing…").
                tracing::info!(group = %group_id, sender = %wire.sender_peer_id, typing = %typing.active, "group typing received");
                let _ = self.inner.app.emit(
                    "typing",
                    TypingEvent {
                        peer_id: group_id.to_string(),
                        is_typing: typing.active,
                        sender: Some(wire.sender_peer_id.clone()),
                    },
                );
                Ok(None)
            }
            e2ee_core::ParsedPayload::Read(read) => {
                // A member read one of our (or another member's) group
                // messages: count them so the sender's tick turns blue.
                self.apply_read_by(group_id, &read.message_id, &wire.sender_peer_id)?;
                Ok(None)
            }
            e2ee_core::ParsedPayload::Text(text) => {
                // No read receipt here: the UI sends it (via mark_read) once
                // the message is actually visible on screen — never merely on
                // receipt, so "read" matches what the user has opened.
                tracing::info!(
                    group = %group_id,
                    sender = %wire.sender_peer_id,
                    msg_id = ?text.message_id,
                    len = text.text.chars().count(),
                    "group message decrypted, recording"
                );
                Ok(Some(self.record_incoming(
                    group_id,
                    text.text,
                    text.quote,
                    text.message_id,
                )?))
            }
        }
    }

    /// Megolm-encrypt a group read receipt (we read `message_id`) and fan it
    /// out to every member. Best-effort, never recorded.
    pub(crate) fn send_group_read_receipt(
        &self,
        group_id: &str,
        message_id: &str,
    ) -> Result<(), RelayError> {
        let payload = e2ee_core::ChatPayload::Read(e2ee_core::ReadPayload::new(message_id));
        let bytes = serde_json::to_vec(&payload)?;
        self.send_group_payload(
            group_id,
            &bytes,
            GroupSend {
                record: false,
                client_id: String::new(),
                quote: None,
                message_id: None,
                display_text: None,
            },
        )
    }

    /// Record that `reader` has read the group message `message_id` and notify
    /// the UI (which flips the tick blue once every other member has read it).
    fn apply_read_by(
        &self,
        group_id: &str,
        message_id: &str,
        reader: &str,
    ) -> Result<(), RelayError> {
        let count = {
            let mut messages = write_guard(&self.inner.messages)?;
            let thread = messages.entry(group_id.to_string()).or_default();
            let message = thread.iter_mut().find(|m| m.id == message_id);
            match message {
                Some(message)
                    if message.outgoing && !message.read_by.iter().any(|r| r == reader) =>
                {
                    message.read_by.push(reader.to_string());
                    message.read_by.len()
                }
                _ => 0,
            }
        };
        if count > 0 {
            let _ = self.inner.app.emit(
                "message-read-by",
                MessageReadByEvent {
                    group_id: group_id.to_string(),
                    message_id: message_id.to_string(),
                    read_by_count: count,
                },
            );
        }
        Ok(())
    }

    /// Decrypt a Megolm ciphertext with the inbound session built from that
    /// sender's shared session key. Returns `None` (and logs) when no session
    /// exists for this (group_id, sender) pair or the ratchet rejects the
    /// message — a missing session key must never break the inbound pump. A
    /// legacy fallback tries the empty-sender session (migrated from the old
    /// single-sender shape) so pre-multi-sender groups keep decrypting.
    fn decrypt_group(&self, group_id: &str, sender: &str, ciphertext: &str) -> Option<Vec<u8>> {
        let mut inbound = match mutex_guard(&self.inner.inbound_groups) {
            Ok(g) => g,
            Err(_) => return None,
        };
        let senders = match inbound.get_mut(group_id) {
            Some(senders) => senders,
            None => {
                tracing::warn!(group = %group_id, "no inbound group session");
                return None;
            }
        };
        let session = match senders.get_mut(sender) {
            Some(session) => session,
            None => match senders.get_mut("") {
                Some(legacy) => legacy,
                None => {
                    tracing::warn!(group = %group_id, sender = %sender, "no inbound group session");
                    return None;
                }
            },
        };
        match session.decrypt(ciphertext) {
            Ok(bytes) => Some(bytes),
            Err(err) => {
                tracing::warn!(group = %group_id, error = %err, "failed to decrypt group message");
                None
            }
        }
    }

    /// Store a received Megolm `session_key` share as an inbound session keyed
    /// by the sharing member (who is identified by the `sender` field in the
    /// payload, or by the authenticated 1:1 envelope sender when the field is
    /// absent) and surface the group in the chat list under its name.
    ///
    /// The FIRST key received for a group marks our join: besides keeping the
    /// sender's inbound session we set up our own side so we can send too —
    /// fetch the roster, create our own outbound session and share its key to
    /// every other member over 1:1 sessions. That setup runs in the background
    /// (it needs a `get_group_info` round-trip) so the inbound pump is never
    /// blocked.
    pub(crate) fn handle_group_key(&self, payload: &GroupKeyPayload) -> Result<(), RelayError> {
        // The caller (`ingest`) backfills the authenticated 1:1 sender into
        // `payload.sender`; an empty sender here is a defensive fallback that
        // stores the key under the empty "legacy" key used by `decrypt_group`.
        let sender = payload.sender.clone();
        let inbound = InboundGroup::new(&payload.session_key)?;
        // First join: this group was unknown before the key arrived.
        let is_first_join = {
            let groups = read_guard(&self.inner.groups)?;
            !groups.contains_key(&payload.group_id)
        };
        {
            let mut inbound_groups = mutex_guard(&self.inner.inbound_groups)?;
            inbound_groups
                .entry(payload.group_id.clone())
                .or_default()
                .insert(sender, inbound);
        }
        // Register the group so the chat list and header render it by name.
        write_guard(&self.inner.groups)?
            .entry(payload.group_id.clone())
            .or_insert(GroupInfoState {
                name: payload.group_name.clone(),
                members: Vec::new(),
                my_role: None,
                avatar_url: None,
                outbound: None,
                key_shared_this_session: false,
            });
        // The group ID acts as a contact whose display name is the group name.
        self.remember_contact_name(&payload.group_id, &payload.group_name)?;
        self.save_group_sessions()?;
        // The roster is unknown until a get_group_info round-trip; fetch it in
        // the background so the chat list shows a real member count.
        self.spawn_group_info_refresh(&payload.group_id);
        // On the very first key for a group we join the multi-sender setup:
        // create our own outbound and share it with the other members.
        if is_first_join {
            let client = self.clone();
            let group_id = payload.group_id.clone();
            let group_name = payload.group_name.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = client
                    .establish_outbound_and_share(&group_id, &group_name)
                    .await
                {
                    tracing::warn!(group = %group_id, error = %err, "failed to set up outbound group session");
                }
            });
        }
        Ok(())
    }

    /// Make sure this identity owns an outbound Megolm session for `group_id`,
    /// establishing one (roster fetch + key share to the other members) on
    /// first use.
    ///
    /// Groups that predate the multi-sender model — and joins whose setup
    /// never finished — have `outbound: None`, so a send would otherwise fail
    /// with "has no outbound session yet". Calling this before the first send
    /// heals those groups lazily, with no user action required.
    pub(crate) async fn ensure_outbound_session(&self, group_id: &str) -> Result<(), RelayError> {
        // Fast path: we already own an outbound session AND have shared its
        // key during this process run.
        {
            let groups = read_guard(&self.inner.groups)?;
            if let Some(group) = groups.get(group_id) {
                if group.outbound.is_some() && group.key_shared_this_session {
                    return Ok(());
                }
            }
        }
        let group_name = read_guard(&self.inner.groups)?
            .get(group_id)
            .map(|group| group.name.clone())
            .ok_or_else(|| RelayError::NoOutboundGroup(group_id.to_string()))?;
        // Hydrated outbound session (survived a restart) but the key has not
        // been shared this run: re-share it so recipients with a stale inbound
        // session can decrypt again — this is the "restart breaks group
        // messages" heal.
        let needs_reshare = {
            let groups = read_guard(&self.inner.groups)?;
            match groups.get(group_id) {
                Some(group) => group.outbound.is_some() && !group.key_shared_this_session,
                None => false,
            }
        };
        if needs_reshare {
            self.share_existing_key(group_id, &group_name).await?;
            if let Ok(mut groups) = write_guard(&self.inner.groups) {
                if let Some(group) = groups.get_mut(group_id) {
                    group.key_shared_this_session = true;
                }
            }
            return Ok(());
        }
        self.establish_outbound_and_share(group_id, &group_name)
            .await
    }

    /// Join-time multi-sender setup for a group we just received a key for:
    /// fetch the roster, create our OWN outbound Megolm session and share its
    /// key to every other member over 1:1 sessions (each share identifies us
    /// as the sender). Runs once per group, off the inbound pump.
    async fn establish_outbound_and_share(
        &self,
        group_id: &str,
        group_name: &str,
    ) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        // (a) The roster tells us who else is in the group (the server only
        //     exposes it to members, and we were just added). Only a member
        //     may (or should) build the outbound session — a stale local entry
        //     for a group we left must not resurrect a session.
        let info = self.get_group_info(group_id).await?;
        if info.my_role.is_none() {
            return Err(RelayError::NotInGroup(group_id.to_string()));
        }
        // (b) Create our own outbound session and (d) store it with the fresh
        //     roster, so sends work immediately after this returns. The
        //     `outbound.is_none()` guard keeps a racing establish (send path +
        //     background refresh, or a reconnect racing a first send) from
        //     stamping a second session over the first — and from sharing a
        //     key that does not match the session actually stored.
        let outbound = OutboundGroup::new();
        let session_key = outbound.session_key();
        let stored = {
            let mut groups = write_guard(&self.inner.groups)?;
            match groups.get_mut(group_id) {
                Some(group) if group.outbound.is_none() => {
                    group.members = info.members.clone();
                    group.my_role = info.my_role.clone();
                    group.outbound = Some(outbound);
                    group.key_shared_this_session = false; // shared below
                    true
                }
                _ => false,
            }
        };
        if !stored {
            // A racing establish already stored a fresh session; our duplicate
            // is dropped and nothing (mismatched) is shared.
            return Ok(());
        }
        self.save_group_sessions()?;
        // (c) Share our own session key to every OTHER member over 1:1
        //     sessions. Best-effort per member, like the creator's share.
        for member in &info.members {
            if member.peer_id == my_peer_id {
                continue;
            }
            let result = async {
                if !mutex_guard(&self.inner.sessions)?.contains_key(&member.peer_id) {
                    self.start_chat(&member.peer_id).await?;
                }
                self.send_group_key(&member.peer_id, group_id, &session_key, group_name)
            }
            .await;
            if let Err(err) = result {
                tracing::warn!(peer = %member.peer_id, group = %group_id, error = %err, "failed to share group key");
            }
        }
        if let Ok(mut groups) = write_guard(&self.inner.groups) {
            if let Some(group) = groups.get_mut(group_id) {
                group.key_shared_this_session = true;
            }
        }
        Ok(())
    }

    /// Re-share the EXISTING outbound session key with every other member.
    /// Called before the first group send of a process run when the outbound
    /// session was hydrated from the store: recipients may hold a stale or
    /// absent inbound session (their persisted ratchet can lag or predate a
    /// crash), so re-sharing heals the group without requiring a 1:1 message.
    async fn share_existing_key(&self, group_id: &str, group_name: &str) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        let (session_key, members) = {
            let groups = read_guard(&self.inner.groups)?;
            let group = groups
                .get(group_id)
                .ok_or_else(|| RelayError::NoOutboundGroup(group_id.to_string()))?;
            let session_key = group
                .outbound
                .as_ref()
                .map(|outbound| outbound.session_key())
                .ok_or_else(|| RelayError::NoOutboundGroup(group_id.to_string()))?;
            (session_key, group.members.clone())
        };
        tracing::info!(group = %group_id, members = %members.len(), "re-sharing group key after restart");
        for member in &members {
            if member.peer_id == my_peer_id {
                continue;
            }
            let result = async {
                if !mutex_guard(&self.inner.sessions)?.contains_key(&member.peer_id) {
                    tracing::info!(peer = %member.peer_id, "no 1:1 session — starting chat to share key");
                    self.start_chat(&member.peer_id).await?;
                }
                self.send_group_key(&member.peer_id, group_id, &session_key, group_name)
            }
            .await;
            match &result {
                Ok(()) => {
                    tracing::info!(peer = %member.peer_id, group = %group_id, "group key re-shared")
                }
                Err(err) => {
                    tracing::warn!(peer = %member.peer_id, group = %group_id, error = %err, "failed to re-share group key")
                }
            }
        }
        Ok(())
    }

    /// A `group_created` reply resolved an in-flight `create_group` request and
    /// caches the roster we already know (the creator + every member we added)
    /// so the chat list renders the group immediately.
    pub(crate) fn handle_group_created(
        &self,
        group_id: String,
        name: String,
        members: Vec<String>,
    ) -> Result<(), RelayError> {
        if let Some(tx) = mutex_guard(&self.inner.pending_group_created)?.pop_front() {
            let _ = tx.send(Ok(group_id.clone()));
        }
        let my_peer_id = self.my_peer_id()?;
        let roster = members
            .into_iter()
            .map(|peer_id| GroupMember {
                role: if peer_id == my_peer_id {
                    "owner".to_string()
                } else {
                    "member".to_string()
                },
                peer_id,
            })
            .collect::<Vec<_>>();
        write_guard(&self.inner.groups)?
            .entry(group_id.clone())
            .or_insert(GroupInfoState {
                name,
                members: roster,
                my_role: Some("owner".to_string()),
                avatar_url: None,
                outbound: None,
                key_shared_this_session: false,
            });
        Ok(())
    }

    /// A `group_member_added` reply resolves the in-flight request, keeps the
    /// local roster in sync by appending the member and — in the multi-sender
    /// model — shares this identity's own Megolm outbound session key to the
    /// newly added member, so the newcomer can decrypt this identity's group
    /// messages. Every existing member does the same (each receives the same
    /// roster push), so the newcomer ends up holding every member's stream.
    ///
    /// The key share runs in the background because it may need a 1:1
    /// `start_chat` round-trip (a fresh member has no established session yet).
    pub(crate) fn handle_group_member_added(
        &self,
        group_id: &str,
        peer_id: &str,
    ) -> Result<(), RelayError> {
        if let Some(tx) = mutex_guard(&self.inner.pending_group_member_added)?.pop_front() {
            let _ = tx.send(Ok(()));
        }
        if let Ok(mut groups) = write_guard(&self.inner.groups) {
            if let Some(group) = groups.get_mut(group_id) {
                if !group.members.iter().any(|m| m.peer_id == peer_id) {
                    group.members.push(GroupMember {
                        peer_id: peer_id.to_string(),
                        role: "member".to_string(),
                    });
                }
            }
        }
        let client = self.clone();
        let group = group_id.to_string();
        let member = peer_id.to_string();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = client.share_group_key_to_member(&group, &member).await {
                tracing::warn!(group = %group, member = %member, error = %err, "failed to share group key to new member");
            }
        });
        // WhatsApp-style system message: "X joined the group".
        if peer_id != self.my_peer_id()? {
            self.add_system_message(group_id, "joined", peer_id)?;
        }
        self.emit_group_updated(group_id);
        Ok(())
    }

    /// Record a WhatsApp-style system event (member joined/left) into the
    /// group's thread and surface it to the UI.
    fn add_system_message(
        &self,
        group_id: &str,
        kind: &str,
        peer_id: &str,
    ) -> Result<(), RelayError> {
        let message = UIMessage {
            id: format!(
                "sys-{}",
                self.inner.next_msg_id.fetch_add(1, Ordering::SeqCst)
            ),
            text: String::new(),
            outgoing: false,
            timestamp: now_millis(),
            status: "delivered".to_string(),
            quote: None,
            reactions: Vec::new(),
            system: Some(SystemInfo {
                kind: kind.to_string(),
                peer_id: peer_id.to_string(),
            }),
            read_by: Vec::new(),
        };
        write_guard(&self.inner.messages)?
            .entry(group_id.to_string())
            .or_default()
            .push(message.clone());
        self.persist_message(group_id, &message, None)?;
        self.persist_next_msg_id()?;
        let _ = self.inner.app.emit(
            "chat-message",
            ChatMessageEvent {
                peer_id: group_id.to_string(),
                message,
            },
        );
        Ok(())
    }

    /// Share this identity's own outbound Megolm session key to `member` over
    /// a 1:1 Double Ratchet session, so the member can decrypt this identity's
    /// group messages. Establishes the outbound group session and the 1:1
    /// session on demand. Best-effort per member: a member that is unreachable
    /// (rate-limited, offline without published pre-keys) is skipped so one
    /// failure never aborts the add flow.
    async fn share_group_key_to_member(
        &self,
        group_id: &str,
        member: &str,
    ) -> Result<(), RelayError> {
        // A member never shares a key to itself.
        if member == self.my_peer_id()? {
            return Ok(());
        }
        // We must be a roster member to hold (and share) an outbound session;
        // this also establishes one lazily for legacy groups.
        self.ensure_outbound_session(group_id).await?;
        let (session_key, group_name) = {
            let groups = read_guard(&self.inner.groups)?;
            let group = groups
                .get(group_id)
                .ok_or_else(|| RelayError::NoOutboundGroup(group_id.to_string()))?;
            let outbound = group
                .outbound
                .as_ref()
                .ok_or_else(|| RelayError::NoOutboundGroup(group_id.to_string()))?;
            (outbound.session_key(), group.name.clone())
        };
        if !mutex_guard(&self.inner.sessions)?.contains_key(member) {
            self.start_chat(member).await?;
        }
        self.send_group_key(member, group_id, &session_key, &group_name)
    }

    /// A `group_member_left` push: the leaver gets the confirmation (resolving
    /// the in-flight leave request); every other member gets the same push and
    /// drops the leaver from the cached roster, so member counts stay in sync.
    pub(crate) fn handle_group_member_left(
        &self,
        group_id: &str,
        peer_id: &str,
    ) -> Result<(), RelayError> {
        if let Ok(mut groups) = write_guard(&self.inner.groups) {
            if let Some(group) = groups.get_mut(group_id) {
                group.members.retain(|m| m.peer_id != peer_id);
            }
        }
        if let Some(tx) = mutex_guard(&self.inner.pending_group_op)?.pop_front() {
            let _ = tx.send(Ok(()));
        }
        // WhatsApp-style system message: "X left the group". The leaver itself
        // is dropping the group, so only other members get the event.
        if peer_id != self.my_peer_id()? {
            self.add_system_message(group_id, "left", peer_id)?;
        }
        self.emit_group_updated(group_id);
        Ok(())
    }

    /// A `group_renamed` push: update the cached name (and the persisted
    /// metadata, keeping the avatar), resolve the in-flight request and wake
    /// the UI so the chat list and header show the new name.
    pub(crate) fn handle_group_renamed(
        &self,
        group_id: &str,
        name: &str,
    ) -> Result<(), RelayError> {
        let avatar = {
            let groups = read_guard(&self.inner.groups)?;
            groups.get(group_id).and_then(|g| g.avatar_url.clone())
        };
        if let Ok(mut groups) = write_guard(&self.inner.groups) {
            if let Some(group) = groups.get_mut(group_id) {
                group.name = name.to_string();
            }
        }
        self.persist_group_meta(group_id, name, avatar.as_deref());
        if let Some(tx) = mutex_guard(&self.inner.pending_group_op)?.pop_front() {
            let _ = tx.send(Ok(()));
        }
        self.emit_group_updated(group_id);
        Ok(())
    }

    /// Notify the UI that a group's roster changed (member count/roles), so it
    /// can refresh the chat list without waiting for the next full refresh.
    fn emit_group_updated(&self, group_id: &str) {
        let _ = self.inner.app.emit(
            "group-updated",
            GroupUpdatedEvent {
                group_id: group_id.to_string(),
            },
        );
    }

    /// Persist a group's public display metadata (name, avatar path) so the
    /// chat list can render it immediately after a restart.
    fn persist_group_meta(&self, group_id: &str, name: &str, avatar_url: Option<&str>) {
        if let Ok(store_guard) = self.store_guard() {
            if let Some(store) = store_guard.as_ref() {
                let _ = store.set_group_meta(group_id, name, avatar_url);
            }
        }
    }

    /// A `group_info` reply caches the fresh roster for the chat list / group
    /// panel and resolves the in-flight `get_group_info` request.
    pub(crate) fn handle_group_info(
        &self,
        group_id: String,
        name: String,
        owner_peer_id: String,
        avatar_url: Option<String>,
        members: Vec<GroupMember>,
    ) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        let my_role = members
            .iter()
            .find(|m| m.peer_id == my_peer_id)
            .map(|m| m.role.clone());
        if let Ok(mut groups) = write_guard(&self.inner.groups) {
            match groups.get_mut(&group_id) {
                // Existing group: refresh name, roster, our role and avatar so
                // pushed/refreshed metadata always lands (e.g. the group photo
                // set by another member).
                Some(group) => {
                    group.name = name.clone();
                    group.members = members.clone();
                    group.my_role = my_role.clone();
                    group.avatar_url = avatar_url.clone();
                }
                None => {
                    groups.insert(
                        group_id.clone(),
                        GroupInfoState {
                            name: name.clone(),
                            members: members.clone(),
                            my_role: my_role.clone(),
                            avatar_url: avatar_url.clone(),
                            outbound: None,
                            key_shared_this_session: false,
                        },
                    );
                }
            }
        }
        // Persist the display metadata (name, avatar) so it survives restarts.
        self.persist_group_meta(&group_id, &name, avatar_url.as_deref());
        // The roster/metadata changed: wake the UI (member count, avatar).
        self.emit_group_updated(&group_id);
        // Resolve the matching request by group ID (concurrent lookups may be
        // answered out of order, so FIFO would misroute them).
        {
            let mut pending = mutex_guard(&self.inner.pending_group_info)?;
            if let Some(pos) = pending.iter().position(|(gid, _)| gid == &group_id) {
                let (_, tx) = pending.remove(pos).expect("position must be in bounds");
                let _ = tx.send(Ok(GroupInfo {
                    group_id,
                    name,
                    owner_peer_id,
                    avatar_url,
                    members,
                    my_role,
                }));
            }
        }
        Ok(())
    }

    /// A `ownership_transferred` reply mirrors the role swap into the cached
    /// roster (old owner becomes an admin, the new owner takes over) and
    /// resolves the in-flight request.
    pub(crate) fn handle_ownership_transferred(
        &self,
        group_id: &str,
        new_owner_peer_id: &str,
    ) -> Result<(), RelayError> {
        if let Ok(mut groups) = write_guard(&self.inner.groups) {
            if let Some(group) = groups.get_mut(group_id) {
                let old_owner: Option<String> = group
                    .members
                    .iter()
                    .find(|m| m.role == "owner")
                    .map(|m| m.peer_id.clone());
                for member in group.members.iter_mut() {
                    if Some(member.peer_id.as_str()) == old_owner.as_deref() {
                        member.role = "admin".to_string();
                    }
                    if member.peer_id == new_owner_peer_id {
                        member.role = "owner".to_string();
                    }
                }
                if let Ok(my_peer_id) = self.my_peer_id() {
                    group.my_role = group
                        .members
                        .iter()
                        .find(|m| m.peer_id == my_peer_id)
                        .map(|m| m.role.clone());
                }
            }
        }
        if let Some(tx) = mutex_guard(&self.inner.pending_group_op)?.pop_front() {
            let _ = tx.send(Ok(()));
        }
        self.emit_group_updated(group_id);
        Ok(())
    }

    /// A `group_member_promoted` reply mirrors the role change into the cached
    /// roster and resolves the in-flight request.
    pub(crate) fn handle_group_member_promoted(
        &self,
        group_id: &str,
        peer_id: &str,
    ) -> Result<(), RelayError> {
        self.apply_group_role(group_id, peer_id, "admin")?;
        if let Some(tx) = mutex_guard(&self.inner.pending_group_op)?.pop_front() {
            let _ = tx.send(Ok(()));
        }
        self.emit_group_updated(group_id);
        Ok(())
    }

    /// A `group_member_demoted` reply mirrors the role change into the cached
    /// roster and resolves the in-flight request.
    pub(crate) fn handle_group_member_demoted(
        &self,
        group_id: &str,
        peer_id: &str,
    ) -> Result<(), RelayError> {
        self.apply_group_role(group_id, peer_id, "member")?;
        if let Some(tx) = mutex_guard(&self.inner.pending_group_op)?.pop_front() {
            let _ = tx.send(Ok(()));
        }
        self.emit_group_updated(group_id);
        Ok(())
    }

    /// A `group_member_removed` reply drops the member from the cached roster
    /// and resolves the in-flight request.
    ///
    /// When the removed peer is THIS identity (the owner removed us), the
    /// whole group is dropped locally — roster, Megolm sessions, contact entry
    /// and history — and a `group-removed` event is emitted so the UI can show
    /// a toast and close the conversation. This MVP push only covers online
    /// members: an offline member learns about the removal on its next
    /// `get_group_info` round-trip, which the relay answers with
    /// `not_a_member` (documented limitation — offline group eviction is a
    /// later improvement).
    pub(crate) fn handle_group_member_removed(
        &self,
        group_id: &str,
        peer_id: &str,
    ) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        if peer_id == my_peer_id {
            self.forget_group(group_id);
            write_guard(&self.inner.messages)?.remove(group_id);
            let _ = self.inner.app.emit(
                "group-removed",
                GroupRemovedEvent {
                    group_id: group_id.to_string(),
                },
            );
        } else if let Ok(mut groups) = write_guard(&self.inner.groups) {
            if let Some(group) = groups.get_mut(group_id) {
                group.members.retain(|m| m.peer_id != peer_id);
            }
        }
        if let Some(tx) = mutex_guard(&self.inner.pending_group_op)?.pop_front() {
            let _ = tx.send(Ok(()));
        }
        self.emit_group_updated(group_id);
        Ok(())
    }

    /// A `group_avatar_set` reply resolves the in-flight `set_group_avatar`
    /// request. The group-op path refreshes the metadata afterwards so the new
    /// photo renders in the chat list and header.
    pub(crate) fn handle_group_avatar_set(&self, group_id: &str) -> Result<(), RelayError> {
        if let Some(tx) = mutex_guard(&self.inner.pending_group_op)?.pop_front() {
            let _ = tx.send(Ok(()));
        }
        self.spawn_group_info_refresh(group_id);
        self.emit_group_updated(group_id);
        Ok(())
    }

    /// Mirror a member's role change into the cached group roster (used when a
    /// `group_member_promoted` / `group_member_demoted` reply arrives).
    fn apply_group_role(
        &self,
        group_id: &str,
        peer_id: &str,
        role: &str,
    ) -> Result<(), RelayError> {
        if let Ok(mut groups) = write_guard(&self.inner.groups) {
            if let Some(group) = groups.get_mut(group_id) {
                if let Some(member) = group.members.iter_mut().find(|m| m.peer_id == peer_id) {
                    member.role = role.to_string();
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Group wire format ----------------------------------------------------

    #[test]
    fn group_client_messages_serialize_to_expected_wire_shape() {
        let create = serde_json::to_value(ClientMessage::CreateGroup {
            name: "Squad".into(),
        })
        .expect("serialize");
        assert_eq!(create["type"], "create_group");
        assert_eq!(create["name"], "Squad");

        let add = serde_json::to_value(ClientMessage::AddGroupMember {
            group_id: "g-1".into(),
            peer_id: "bob".into(),
        })
        .expect("serialize");
        assert_eq!(add["type"], "add_group_member");
        assert_eq!(add["group_id"], "g-1");
        assert_eq!(add["peer_id"], "bob");

        let info = serde_json::to_value(ClientMessage::GetGroupInfo {
            group_id: "g-1".into(),
        })
        .expect("serialize");
        assert_eq!(info["type"], "get_group_info");

        let promote = serde_json::to_value(ClientMessage::PromoteMember {
            group_id: "g-1".into(),
            peer_id: "bob".into(),
        })
        .expect("serialize");
        assert_eq!(promote["type"], "promote_member");

        let demote = serde_json::to_value(ClientMessage::DemoteMember {
            group_id: "g-1".into(),
            peer_id: "bob".into(),
        })
        .expect("serialize");
        assert_eq!(demote["type"], "demote_member");

        let remove = serde_json::to_value(ClientMessage::RemoveMember {
            group_id: "g-1".into(),
            peer_id: "bob".into(),
        })
        .expect("serialize");
        assert_eq!(remove["type"], "remove_member");

        let leave = serde_json::to_value(ClientMessage::LeaveGroup {
            group_id: "g-1".into(),
        })
        .expect("serialize");
        assert_eq!(leave["type"], "leave_group");

        let transfer = serde_json::to_value(ClientMessage::TransferOwnership {
            group_id: "g-1".into(),
            new_owner_peer_id: "bob".into(),
        })
        .expect("serialize");
        assert_eq!(transfer["type"], "transfer_ownership");
        assert_eq!(transfer["group_id"], "g-1");
        assert_eq!(transfer["new_owner_peer_id"], "bob");
    }

    #[test]
    fn group_info_server_message_parses_with_roles() {
        let text = r#"{
            "type":"group_info",
            "group_id":"g-1",
            "name":"Squad",
            "owner_peer_id":"alice",
            "members":[
                {"peer_id":"alice","role":"owner"},
                {"peer_id":"bob","role":"admin"}
            ]
        }"#;
        match serde_json::from_str::<ServerMessage>(text).expect("parse") {
            ServerMessage::GroupInfo {
                group_id,
                name,
                owner_peer_id,
                avatar_url,
                members,
            } => {
                assert_eq!(group_id, "g-1");
                assert_eq!(name, "Squad");
                assert_eq!(owner_peer_id, "alice");
                assert_eq!(
                    avatar_url, None,
                    "a group without a photo has no avatar_url"
                );
                assert_eq!(members.len(), 2);
                assert_eq!(members[0].peer_id, "alice");
                assert_eq!(members[0].role, "owner");
                assert_eq!(members[1].peer_id, "bob");
                assert_eq!(members[1].role, "admin");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn group_created_and_role_confirmation_messages_parse() {
        let created: ServerMessage = serde_json::from_str(
            r#"{"type":"group_created","group_id":"g-1","name":"Squad","members":["alice"]}"#,
        )
        .expect("parse");
        match created {
            ServerMessage::GroupCreated {
                group_id,
                name,
                members,
                ..
            } => {
                assert_eq!(group_id, "g-1");
                assert_eq!(name, "Squad");
                assert_eq!(members, vec!["alice".to_string()]);
            }
            other => panic!("unexpected variant: {other:?}"),
        }

        let promoted: ServerMessage = serde_json::from_str(
            r#"{"type":"group_member_promoted","group_id":"g-1","peer_id":"bob"}"#,
        )
        .expect("parse");
        match promoted {
            ServerMessage::GroupMemberPromoted { group_id, peer_id } => {
                assert_eq!(group_id, "g-1");
                assert_eq!(peer_id, "bob");
            }
            other => panic!("unexpected variant: {other:?}"),
        }

        let removed: ServerMessage = serde_json::from_str(
            r#"{"type":"group_member_removed","group_id":"g-1","peer_id":"bob"}"#,
        )
        .expect("parse");
        assert!(matches!(
            removed,
            ServerMessage::GroupMemberRemoved { group_id, peer_id }
                if group_id == "g-1" && peer_id == "bob"
        ));

        let transferred: ServerMessage = serde_json::from_str(
            r#"{"type":"ownership_transferred","group_id":"g-1","new_owner_peer_id":"bob"}"#,
        )
        .expect("parse");
        match transferred {
            ServerMessage::OwnershipTransferred {
                group_id,
                new_owner_peer_id,
            } => {
                assert_eq!(group_id, "g-1");
                assert_eq!(new_owner_peer_id, "bob");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn group_key_payload_parses() {
        let text = r#"{"kind":"group_key","group_id":"g-1","session_key":"abc","group_name":"Squad","sender":"alice"}"#;
        let payload: GroupKeyPayload = serde_json::from_str(text).expect("parse");
        assert_eq!(payload.kind, "group_key");
        assert_eq!(payload.group_id, "g-1");
        assert_eq!(payload.session_key, "abc");
        assert_eq!(payload.group_name, "Squad");
        assert_eq!(payload.sender, "alice");

        // A share without a `sender` field (older clients) still parses; the
        // inbound pump backfills the authenticated 1:1 sender afterwards.
        let legacy =
            r#"{"kind":"group_key","group_id":"g-1","session_key":"abc","group_name":"Squad"}"#;
        let payload: GroupKeyPayload =
            serde_json::from_str(legacy).expect("legacy share must parse");
        assert_eq!(payload.sender, "");
    }

    #[test]
    fn group_envelope_roundtrips_via_megolm_on_the_wire() {
        // The creator builds the outbound session and shares its session key
        // over an end-to-end channel; the recipient rebuilds the inbound side.
        let mut outbound = OutboundGroup::new();
        let session_key = outbound.session_key();
        let mut inbound = InboundGroup::new(&session_key).expect("key must parse");
        assert_eq!(outbound.session_id(), inbound.session_id());

        // A group envelope carries the Megolm ciphertext keyed by group id,
        // exactly as `send_group_message` emits it.
        let ciphertext = outbound.encrypt(b"hello group");
        let content = EnvelopeContent::Group {
            group_id: "g-1".to_string(),
            ciphertext: ciphertext.clone(),
        };
        let json = serde_json::to_string(&content).expect("serialize");
        let restored: EnvelopeContent = serde_json::from_str(&json).expect("deserialize");
        let restored_cipher = match restored {
            EnvelopeContent::Group { ciphertext, .. } => ciphertext,
            other => panic!("unexpected content: {other:?}"),
        };
        assert_eq!(restored_cipher, ciphertext);

        // The recipient decrypts with the inbound session built from the key.
        let plaintext = inbound.decrypt(&restored_cipher).expect("decrypt");
        assert_eq!(plaintext, b"hello group");

        // A group-key share travels as an ordinary encrypted Message whose
        // plaintext is the JSON payload. The share identifies the member that
        // owns the outbound session, so the recipient keys its inbound session
        // by that member's peer id.
        let payload = serde_json::json!({
            "kind": "group_key",
            "group_id": "g-1",
            "session_key": session_key,
            "group_name": "Squad",
            "sender": "alice",
        });
        let parsed: GroupKeyPayload = serde_json::from_value(payload).expect("parse");
        assert_eq!(parsed.sender, "alice");
        let mut member_inbound =
            InboundGroup::new(&parsed.session_key).expect("member key must parse");
        assert_eq!(member_inbound.session_id(), outbound.session_id());
        let next = outbound.encrypt(b"second message");
        assert_eq!(
            member_inbound.decrypt(&next).expect("decrypt"),
            b"second message"
        );
    }

    #[test]
    fn multi_sender_members_decrypt_each_others_streams() {
        // Two members of the same group. Each holds its OWN outbound Megolm
        // session and shares its session key to the other, who builds an
        // inbound session from it. Both can send with their own stream and
        // decrypt the other's messages with the correct inbound session — a
        // session from the wrong sender must never open another's ciphertext.
        let mut alice_outbound = OutboundGroup::new();
        let mut bob_outbound = OutboundGroup::new();
        let alice_key = alice_outbound.session_key();
        let bob_key = bob_outbound.session_key();

        // Each peer keeps the OTHER's session key as its inbound session.
        let mut alice_inbound_for_bob = InboundGroup::new(&bob_key).expect("bob's key must parse");
        let mut bob_inbound_for_alice =
            InboundGroup::new(&alice_key).expect("alice's key must parse");

        // The two senders have distinct session ids, so their streams never
        // collide under the (group_id, sender) key.
        assert_ne!(alice_outbound.session_id(), bob_outbound.session_id());
        assert_eq!(
            alice_outbound.session_id(),
            bob_inbound_for_alice.session_id()
        );
        assert_eq!(
            bob_outbound.session_id(),
            alice_inbound_for_bob.session_id()
        );

        // Alice encrypts with her own stream; only bob's inbound (built from
        // alice's key) decrypts it.
        let a1 = alice_outbound.encrypt(b"alice says hi");
        assert_eq!(
            bob_inbound_for_alice
                .decrypt(&a1)
                .expect("bob decrypts alice"),
            b"alice says hi"
        );
        assert!(
            alice_inbound_for_bob.decrypt(&a1).is_err(),
            "bob's stream must not open alice's ciphertext"
        );

        // Bob encrypts with his own stream; only alice's inbound (built from
        // bob's key) decrypts it.
        let b1 = bob_outbound.encrypt(b"bob replies");
        assert_eq!(
            alice_inbound_for_bob
                .decrypt(&b1)
                .expect("alice decrypts bob"),
            b"bob replies"
        );
        assert!(
            bob_inbound_for_alice.decrypt(&b1).is_err(),
            "alice's stream must not open bob's ciphertext"
        );

        // Both streams keep working independently across further messages.
        let a2 = alice_outbound.encrypt(b"alice again");
        let b2 = bob_outbound.encrypt(b"bob again");
        assert_eq!(bob_inbound_for_alice.decrypt(&a2).unwrap(), b"alice again");
        assert_eq!(alice_inbound_for_bob.decrypt(&b2).unwrap(), b"bob again");
    }
}
