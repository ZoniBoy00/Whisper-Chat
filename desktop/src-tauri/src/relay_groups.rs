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
    /// Our own outbound Megolm session. In the multi-sender model EVERY member
    /// holds one (created automatically when they first receive the group's
    /// key), so each member can send to the group.
    pub(crate) outbound: Option<OutboundGroup>,
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
                outbound: Some(outbound),
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
                eprintln!("whisper desktop: failed to add {member} to group {group_id}: {err}");
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
                eprintln!("whisper desktop: failed to share group key to {member}: {err}");
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
        mutex_guard(&self.inner.pending_group_info)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::GetGroupInfo {
            group_id: group_id.to_string(),
        }) {
            mutex_guard(&self.inner.pending_group_info)?.pop_back();
            return Err(err);
        }

        let info = match tokio::time::timeout(PREKEY_FETCH_TIMEOUT, rx).await {
            Err(_) => {
                // The reply never arrived. Drop the waiter so a late reply can
                // never misalign the FIFO pending queue for later requests.
                mutex_guard(&self.inner.pending_group_info)?.pop_front();
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
            }
        }
        Ok(GroupInfo { my_role, ..info })
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
            peer_id: peer_id.to_string(),
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
    }

    /// Kick off a best-effort background `get_group_info` for `group_id` so
    /// the roster (and therefore the member count shown by the chat list)
    /// refreshes without waiting for the user to open the group info panel.
    fn spawn_group_info_refresh(&self, group_id: &str) {
        let client = self.clone();
        let id = group_id.to_string();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = client.get_group_info(&id).await {
                eprintln!("whisper desktop: failed to refresh roster for group {id}: {err}");
            }
        });
    }

    /// Background-refresh the member roster of every known group. Called after
    /// every successful connect so groups restored from the store (whose
    /// rosters are empty until the first `get_group_info` round-trip) show a
    /// real member count shortly after startup or reconnect.
    pub(crate) fn refresh_group_rosters(&self) {
        let group_ids: Vec<String> = match read_guard(&self.inner.groups) {
            Ok(groups) => groups.keys().cloned().collect(),
            Err(_) => return,
        };
        for group_id in group_ids {
            self.spawn_group_info_refresh(&group_id);
        }
    }

    /// Megolm-encrypt `text` with the group's outbound session and fan it out
    /// to every member via the relay's `send_group_message`.
    pub(crate) fn send_group_message(
        &self,
        group_id: &str,
        text: &str,
        client_id: &str,
    ) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        // Encrypt with our own outbound Megolm session. Every member owns one
        // in the multi-sender model (created on first group-key receipt), so
        // only a member that joined but has not finished the join-time setup
        // yet can hit `NoOutboundGroup` here.
        let ciphertext = {
            let mut groups = write_guard(&self.inner.groups)?;
            let group = groups
                .get_mut(group_id)
                .ok_or_else(|| RelayError::NoOutboundGroup(group_id.to_string()))?;
            let outbound = group
                .outbound
                .as_mut()
                .ok_or_else(|| RelayError::NoOutboundGroup(group_id.to_string()))?;
            outbound.encrypt(text)
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
        let msg = self.record_outgoing(group_id, text, client_id)?;
        self.record_pending_ack(seq, &msg.id)?;

        let mut envelope = relay_envelope;
        envelope.seq = seq;
        if let Err(err) = self.send_json(&ClientMessage::SendGroupMessage {
            group_id: group_id.to_string(),
            envelope,
        }) {
            let _ = mutex_guard(&self.inner.pending_acks)?.remove(&seq);
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
            return Err(err);
        }

        let _ = self.inner.app.emit(
            "chat-message",
            ChatMessageEvent {
                peer_id: group_id.to_string(),
                message: msg,
            },
        );
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
        let text = String::from_utf8_lossy(&plaintext).to_string();
        // No end-to-end read receipts for group messages in the MVP model.
        Ok(Some(self.record_incoming(group_id, text)?))
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
                eprintln!("whisper desktop: no inbound group session for {group_id}");
                return None;
            }
        };
        let session = match senders.get_mut(sender) {
            Some(session) => session,
            None => match senders.get_mut("") {
                Some(legacy) => legacy,
                None => {
                    eprintln!("whisper desktop: no inbound session for {group_id} from {sender}");
                    return None;
                }
            },
        };
        match session.decrypt(ciphertext) {
            Ok(bytes) => Some(bytes),
            Err(err) => {
                eprintln!("whisper desktop: failed to decrypt group message: {err}");
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
                outbound: None,
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
                    eprintln!(
                        "whisper desktop: failed to set up outbound session for group {group_id}: {err}"
                    );
                }
            });
        }
        Ok(())
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
        //     exposes it to members, and we were just added).
        let info = self.get_group_info(group_id).await?;
        // (b) Create our own outbound session and (d) store it with the
        //     fresh roster, so sends work immediately after this returns.
        let outbound = OutboundGroup::new();
        let session_key = outbound.session_key();
        {
            let mut groups = write_guard(&self.inner.groups)?;
            if let Some(group) = groups.get_mut(group_id) {
                group.members = info.members.clone();
                group.my_role = info.my_role.clone();
                group.outbound = Some(outbound);
            }
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
                eprintln!(
                    "whisper desktop: failed to share group key to {}: {err}",
                    member.peer_id
                );
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
                outbound: None,
            });
        Ok(())
    }

    /// A `group_member_added` reply resolves the in-flight request and keeps
    /// the local roster in sync by appending the member.
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
        Ok(())
    }

    /// A `group_member_left` reply resolves the in-flight leave request.
    pub(crate) fn handle_group_member_left(&self) -> Result<(), RelayError> {
        if let Some(tx) = mutex_guard(&self.inner.pending_group_op)?.pop_front() {
            let _ = tx.send(Ok(()));
        }
        Ok(())
    }

    /// A `group_info` reply caches the fresh roster for the chat list / group
    /// panel and resolves the in-flight `get_group_info` request.
    pub(crate) fn handle_group_info(
        &self,
        group_id: String,
        name: String,
        owner_peer_id: String,
        members: Vec<GroupMember>,
    ) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        let my_role = members
            .iter()
            .find(|m| m.peer_id == my_peer_id)
            .map(|m| m.role.clone());
        if let Ok(mut groups) = write_guard(&self.inner.groups) {
            groups.entry(group_id.clone()).or_insert(GroupInfoState {
                name: name.clone(),
                members: members.clone(),
                my_role: my_role.clone(),
                outbound: None,
            });
        }
        if let Some(tx) = mutex_guard(&self.inner.pending_group_info)?.pop_front() {
            let _ = tx.send(Ok(GroupInfo {
                group_id,
                name,
                owner_peer_id,
                members,
                my_role,
            }));
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
        Ok(())
    }

    /// A `group_member_removed` reply drops the member from the cached roster
    /// and resolves the in-flight request.
    pub(crate) fn handle_group_member_removed(
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
            peer_id: "bob".into(),
        })
        .expect("serialize");
        assert_eq!(transfer["type"], "transfer_ownership");
        assert_eq!(transfer["group_id"], "g-1");
        assert_eq!(transfer["peer_id"], "bob");
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
                members,
            } => {
                assert_eq!(group_id, "g-1");
                assert_eq!(name, "Squad");
                assert_eq!(owner_peer_id, "alice");
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
