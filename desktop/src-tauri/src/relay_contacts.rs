//! Friend requests and the accepted-contact model.
//!
//! The relay is the authority on who is friends with whom: it stores pending
//! requests and accepted relationships, and ENFORCES them (1:1 routing, pre-key
//! fetches and group-member adds all require an accepted friendship). This
//! module owns the client side of that contract:
//!
//! - Command methods ([`RelayClient::send_friend_request`],
//!   [`RelayClient::accept_friend_request`],
//!   [`RelayClient::decline_friend_request`],
//!   [`RelayClient::get_friend_requests`] and the server-level
//!   [`RelayClient::remove_contact`]).
//! - The inbound push handlers (`friend_request_received`,
//!   `friend_request_accepted`, `friend_request_declined`, `contact_removed`
//!   and the `friend_requests` snapshot) plus the reply acks.
//!
//! # Wire contract
//!
//! The relay answers each friend-request command with its own ack
//! (`friend_request_sent`, `friend_request_accepted_ok`,
//! `friend_request_declined_ok`, `contact_removed_ok`) or a `friend_requests`
//! snapshot (`get_friend_requests`). Failures come back as an `error` code
//! (`not_contacts`, `already_pending`, `already_contacts`, `cannot_add_self`,
//! `not_found`, `rate_limited`). The relay answers in order, so a FIFO pending
//! queue stays aligned, exactly like the pre-key and group-request queues.
//! Lifecycle pushes are separate: `friend_request_received` reaches the target
//! of a new request; `friend_request_accepted` reaches BOTH peers (each adds
//! the other to its contact list); `friend_request_declined` reaches the
//! requester; `contact_removed` reaches BOTH peers after a removal.

use super::*;

/// How long to wait for a friend-request command reply.
const CONTACT_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl RelayClient {
    // ---------------------------------------------------------------------
    // Friend-request commands
    // ---------------------------------------------------------------------

    /// Send a friend request to `peer_id`. The peer becomes an accepted
    /// contact once they accept. Fails locally with `cannot_add_self` when
    /// `peer_id` is our own identity; other failures come back as relay error
    /// codes (`already_pending`, `already_contacts`, `not_found`,
    /// `rate_limited`, ...).
    pub async fn send_friend_request(&self, peer_id: &str) -> Result<(), RelayError> {
        if peer_id == self.my_peer_id()? {
            return Err(RelayError::Relay("cannot_add_self".to_string()));
        }
        self.contact_op(ClientMessage::SendFriendRequest {
            peer_id: peer_id.to_string(),
        })
        .await
        .map(|_| ())
    }

    /// Fetch the caller's accepted 1:1 contacts (peer IDs) from the relay.
    /// Used on connect to rehydrate the local contact list after a database
    /// reset or restore — the relay is the source of truth for friendships.
    pub async fn list_contacts(&self) -> Result<Vec<String>, RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_contacts_list)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::ListContacts) {
            mutex_guard(&self.inner.pending_contacts_list)?.pop_back();
            return Err(err);
        }
        match tokio::time::timeout(CONTACT_FETCH_TIMEOUT, rx).await {
            Err(_) => {
                mutex_guard(&self.inner.pending_contacts_list)?.pop_front();
                Err(RelayError::ContactTimeout)
            }
            Ok(inner) => inner.map_err(|_| RelayError::ContactRequestFailed)?,
        }
    }

    /// The `contacts` reply arrived: resolve the waiter, then merge the
    /// server's contact list into the local one (memory + store) so a reset or
    /// restore rehydrates the contact list from the relay.
    pub(crate) fn handle_contacts_list(&self, peers: Vec<String>) -> Result<(), RelayError> {
        if let Some(tx) = mutex_guard(&self.inner.pending_contacts_list)?.pop_front() {
            let _ = tx.send(Ok(peers.clone()));
        }
        let mut added = Vec::new();
        {
            let mut contacts = write_guard(&self.inner.contacts)?;
            for peer in &peers {
                if !contacts.iter().any(|known| known == peer) {
                    contacts.push(peer.clone());
                    added.push(peer.clone());
                }
            }
        }
        if !added.is_empty() {
            let store_guard = self.store_guard()?;
            let store = store_guard.as_ref().ok_or(RelayError::StoreNotOpen)?;
            for peer in &added {
                let _ = store.upsert_contact(&ContactRow {
                    peer_id: peer.clone(),
                    display_name: None,
                    username: None,
                    avatar_url: None,
                    last_seen: None,
                    curve25519_key: None,
                    verified: false,
                });
            }
            // Tell the UI to resync its chat list — without the "you are now
            // contacts" toast that `contact-added` implies.
            let _ = self.inner.app.emit(
                "contacts-rehydrated",
                ContactsRehydratedEvent {
                    peer_ids: added.clone(),
                },
            );
        }
        Ok(())
    }

    /// Accept a pending incoming friend request from `peer_id`. Both sides
    /// become accepted contacts; the relay answers with
    /// `friend_request_accepted_ok` and then pushes `friend_request_accepted`
    /// to BOTH peers, so this side also surfaces the peer in the chat list
    /// right away (the `handle_friend_request_accepted` push handler does it
    /// again idempotently).
    pub async fn accept_friend_request(&self, peer_id: &str) -> Result<(), RelayError> {
        self.contact_op(ClientMessage::AcceptFriendRequest {
            peer_id: peer_id.to_string(),
        })
        .await?;
        self.ensure_contact(peer_id)?;
        Ok(())
    }

    /// Decline a pending incoming friend request from `peer_id`. The relay
    /// answers with `friend_request_declined_ok` and pushes
    /// `friend_request_declined` to the requester.
    pub async fn decline_friend_request(&self, peer_id: &str) -> Result<(), RelayError> {
        self.contact_op(ClientMessage::DeclineFriendRequest {
            peer_id: peer_id.to_string(),
        })
        .await
        .map(|_| ())
    }

    /// Fetch the full friend-request snapshot (incoming + outgoing). Seeded
    /// after every connect so the Requests section renders without waiting for
    /// a live push.
    pub async fn get_friend_requests(&self) -> Result<FriendRequests, RelayError> {
        self.contact_op(ClientMessage::GetFriendRequests).await
    }

    /// Remove the accepted contact relationship with `peer_id` on both sides.
    ///
    /// The relay answers with `contact_removed_ok` (or a `not_contacts` /
    /// `rate_limited` error) and then pushes `contact_removed` to BOTH peers.
    /// On success the LOCAL cleanup (contact row, message history, cached
    /// presence and any pending request) is applied immediately; the push
    /// handler performs the same idempotent cleanup on the remote side.
    pub async fn remove_contact(&self, peer_id: &str) -> Result<(), RelayError> {
        self.contact_op(ClientMessage::RemoveContact {
            peer_id: peer_id.to_string(),
        })
        .await?;
        self.drop_contact_locally(peer_id)
    }

    /// Send a friend-request command and wait for the relay's reply. The
    /// pending queue is FIFO, so the next ack (`friend_request_sent`,
    /// `_ok` reply or the `friend_requests` snapshot) resolves the oldest
    /// outstanding command.
    async fn contact_op(&self, message: ClientMessage) -> Result<FriendRequests, RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_contact_ops)?.push_back(tx);
        if let Err(err) = self.send_json(&message) {
            // The request never left, so drop the dangling waiter.
            mutex_guard(&self.inner.pending_contact_ops)?.pop_back();
            return Err(err);
        }
        match tokio::time::timeout(CONTACT_FETCH_TIMEOUT, rx).await {
            Err(_) => {
                // The reply never arrived. Drop the waiter so a late reply can
                // never misalign the FIFO queue for later commands.
                mutex_guard(&self.inner.pending_contact_ops)?.pop_front();
                Err(RelayError::ContactTimeout)
            }
            Ok(inner) => inner.map_err(|_| RelayError::ContactRequestFailed)?,
        }
    }

    // ---------------------------------------------------------------------
    // Inbound push handlers
    // ---------------------------------------------------------------------

    /// A reply ack arrived (`friend_request_sent`, `friend_request_accepted_ok`,
    /// `friend_request_declined_ok` or `contact_removed_ok`): resolve the
    /// oldest outstanding command as a success.
    pub(crate) fn handle_friend_request_ack(&self) -> Result<(), RelayError> {
        if let Some(tx) = mutex_guard(&self.inner.pending_contact_ops)?.pop_front() {
            let _ = tx.send(Ok(FriendRequests::default()));
        }
        Ok(())
    }

    /// A new incoming friend request arrived from `peer_id`. Store it (in
    /// arrival order, de-duplicated) and emit a `friend-request` event so the
    /// UI can toast and refresh its Requests section.
    pub(crate) fn handle_friend_request_received(
        &self,
        peer_id: &str,
        display_name: Option<String>,
    ) -> Result<(), RelayError> {
        {
            let mut incoming = write_guard(&self.inner.friend_requests_incoming)?;
            append_incoming_request(
                &mut incoming,
                FriendRequestIncoming {
                    peer_id: peer_id.to_string(),
                    display_name: display_name.clone(),
                },
            );
        }
        let _ = self.inner.app.emit(
            "friend-request",
            FriendRequestEvent {
                peer_id: peer_id.to_string(),
                display_name,
            },
        );
        Ok(())
    }

    /// A pending OUTGOING request was accepted: `peer_id` is now an accepted
    /// contact. Drop it from the outgoing list, surface the peer in the chat
    /// list and emit a `contact-added` event so the UI can toast and resync.
    pub(crate) fn handle_friend_request_accepted(&self, peer_id: &str) -> Result<(), RelayError> {
        if let Ok(mut outgoing) = write_guard(&self.inner.friend_requests_outgoing) {
            retain_outgoing_peer(&mut outgoing, peer_id);
        }
        self.ensure_contact(peer_id)?;
        let _ = self.inner.app.emit(
            "contact-added",
            ContactAddedEvent {
                peer_id: peer_id.to_string(),
                display_name: None,
            },
        );
        Ok(())
    }

    /// A pending OUTGOING request was declined. Drop it from the outgoing list
    /// and emit a `friend-request-declined` event so the requester's UI can
    /// drop it from the Requests section and toast.
    pub(crate) fn handle_friend_request_declined(&self, peer_id: &str) -> Result<(), RelayError> {
        if let Ok(mut outgoing) = write_guard(&self.inner.friend_requests_outgoing) {
            retain_outgoing_peer(&mut outgoing, peer_id);
        }
        let _ = self.inner.app.emit(
            "friend-request-declined",
            FriendRequestDeclinedEvent {
                peer_id: peer_id.to_string(),
            },
        );
        Ok(())
    }

    /// A contact relationship ended (either side removed it): drop `peer_id`
    /// from every local list and emit a `contact-removed` event so the UI can
    /// close the conversation and toast.
    pub(crate) fn handle_contact_removed(&self, peer_id: &str) -> Result<(), RelayError> {
        self.drop_contact_locally(peer_id)?;
        let _ = self.inner.app.emit(
            "contact-removed",
            ContactRemovedEvent {
                peer_id: peer_id.to_string(),
            },
        );
        Ok(())
    }

    /// A `friend_requests` snapshot arrived (the `get_friend_requests` reply):
    /// store it as the authoritative pending-request state and resolve the
    /// in-flight command with it.
    pub(crate) fn handle_friend_requests(
        &self,
        incoming: Vec<FriendRequestIncoming>,
        outgoing: Vec<String>,
    ) -> Result<(), RelayError> {
        *write_guard(&self.inner.friend_requests_incoming)? = incoming.clone();
        *write_guard(&self.inner.friend_requests_outgoing)? = outgoing.clone();
        if let Some(tx) = mutex_guard(&self.inner.pending_contact_ops)?.pop_front() {
            let _ = tx.send(Ok(FriendRequests { incoming, outgoing }));
        }
        Ok(())
    }

    /// Drop every local trace of `peer_id` after the relationship ended:
    /// the contact row, message history, cached presence, learned profile data
    /// and any pending request, in memory and in the encrypted store.
    /// Idempotent, so both the `remove_contact` command and the
    /// `contact_removed` push can call it without double effects.
    fn drop_contact_locally(&self, peer_id: &str) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        write_guard(&self.inner.contacts)?.retain(|known| known != peer_id);
        write_guard(&self.inner.messages)?.remove(peer_id);
        write_guard(&self.inner.presence)?.remove(peer_id);
        write_guard(&self.inner.profiles)?.contacts.remove(peer_id);
        write_guard(&self.inner.profiles)?
            .contact_avatars
            .remove(peer_id);
        if let Ok(mut incoming) = write_guard(&self.inner.friend_requests_incoming) {
            incoming.retain(|request| request.peer_id != peer_id);
        }
        if let Ok(mut outgoing) = write_guard(&self.inner.friend_requests_outgoing) {
            retain_outgoing_peer(&mut outgoing, peer_id);
        }
        let store_guard = self.store_guard()?;
        let store = store_guard.as_ref().ok_or(RelayError::StoreNotOpen)?;
        store.delete_contact(peer_id)?;
        store.delete_messages_for(peer_id)?;
        Ok(())
    }
}

/// Pure helper for the incoming-request list: append `request` when no
/// request from the same peer is already present. Returns whether it was
/// newly added. Separated so the dedup rule is unit-testable without a live
/// relay client.
fn append_incoming_request(
    incoming: &mut Vec<FriendRequestIncoming>,
    request: FriendRequestIncoming,
) -> bool {
    if incoming
        .iter()
        .any(|existing| existing.peer_id == request.peer_id)
    {
        return false;
    }
    incoming.push(request);
    true
}

/// Pure helper for the outgoing-request list: drop `peer_id` from the pending
/// outgoing requests. An unknown peer is an idempotent no-op.
fn retain_outgoing_peer(outgoing: &mut Vec<String>, peer_id: &str) {
    outgoing.retain(|id| id != peer_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Client wire format -------------------------------------------------

    #[test]
    fn send_friend_request_serializes_to_the_relay_wire_shape() {
        let json = serde_json::to_value(ClientMessage::SendFriendRequest {
            peer_id: "deadbeef0000000000000000".into(),
        })
        .expect("serialize");
        assert_eq!(json["type"], "send_friend_request");
        assert_eq!(json["peer_id"], "deadbeef0000000000000000");
    }

    #[test]
    fn accept_friend_request_serializes_to_the_relay_wire_shape() {
        let json = serde_json::to_value(ClientMessage::AcceptFriendRequest {
            peer_id: "deadbeef0000000000000000".into(),
        })
        .expect("serialize");
        assert_eq!(json["type"], "accept_friend_request");
        assert_eq!(json["peer_id"], "deadbeef0000000000000000");
    }

    #[test]
    fn decline_friend_request_serializes_to_the_relay_wire_shape() {
        let json = serde_json::to_value(ClientMessage::DeclineFriendRequest {
            peer_id: "deadbeef0000000000000000".into(),
        })
        .expect("serialize");
        assert_eq!(json["type"], "decline_friend_request");
        assert_eq!(json["peer_id"], "deadbeef0000000000000000");
    }

    #[test]
    fn get_friend_requests_serializes_to_the_relay_wire_shape() {
        let json = serde_json::to_value(ClientMessage::GetFriendRequests).expect("serialize");
        assert_eq!(json["type"], "get_friend_requests");
        assert_eq!(json.as_object().unwrap().len(), 1, "no extra fields");
    }

    #[test]
    fn remove_contact_serializes_to_the_relay_wire_shape() {
        let json = serde_json::to_value(ClientMessage::RemoveContact {
            peer_id: "deadbeef0000000000000000".into(),
        })
        .expect("serialize");
        assert_eq!(json["type"], "remove_contact");
        assert_eq!(json["peer_id"], "deadbeef0000000000000000");
    }

    // -- Server wire format -------------------------------------------------

    #[test]
    fn friend_request_received_push_parses_with_display_name() {
        let text =
            r#"{"type":"friend_request_received","peer_id":"alice","display_name":"Alice Prime"}"#;
        match serde_json::from_str::<ServerMessage>(text).expect("parse") {
            ServerMessage::FriendRequestReceived {
                peer_id,
                display_name,
            } => {
                assert_eq!(peer_id, "alice");
                assert_eq!(display_name.as_deref(), Some("Alice Prime"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn friend_request_received_without_display_name_defaults_to_none() {
        // Older relays may omit the field entirely; serde must default it.
        let text = r#"{"type":"friend_request_received","peer_id":"alice"}"#;
        match serde_json::from_str::<ServerMessage>(text).expect("parse") {
            ServerMessage::FriendRequestReceived {
                peer_id,
                display_name,
            } => {
                assert_eq!(peer_id, "alice");
                assert_eq!(display_name, None);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn friend_request_accepted_and_declined_pushes_parse() {
        let accepted: ServerMessage =
            serde_json::from_str(r#"{"type":"friend_request_accepted","peer_id":"bob"}"#)
                .expect("parse");
        assert!(matches!(
            accepted,
            ServerMessage::FriendRequestAccepted { peer_id } if peer_id == "bob"
        ));

        let declined: ServerMessage =
            serde_json::from_str(r#"{"type":"friend_request_declined","peer_id":"bob"}"#)
                .expect("parse");
        assert!(matches!(
            declined,
            ServerMessage::FriendRequestDeclined { peer_id } if peer_id == "bob"
        ));
    }

    #[test]
    fn contact_removed_push_parses() {
        let text = r#"{"type":"contact_removed","peer_id":"bob"}"#;
        match serde_json::from_str::<ServerMessage>(text).expect("parse") {
            ServerMessage::ContactRemoved { peer_id } => assert_eq!(peer_id, "bob"),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn friend_request_command_acks_parse() {
        // The relay answers each command with a dedicated unit ack.
        for (json, expected) in [
            (
                r#"{"type":"friend_request_sent"}"#,
                ServerMessage::FriendRequestSent,
            ),
            (
                r#"{"type":"friend_request_accepted_ok"}"#,
                ServerMessage::FriendRequestAcceptedOk,
            ),
            (
                r#"{"type":"friend_request_declined_ok"}"#,
                ServerMessage::FriendRequestDeclinedOk,
            ),
            (
                r#"{"type":"contact_removed_ok"}"#,
                ServerMessage::ContactRemovedOk,
            ),
        ] {
            let message: ServerMessage = serde_json::from_str(json).expect("parse");
            assert!(std::mem::discriminant(&message) == std::mem::discriminant(&expected));
        }
    }

    #[test]
    fn friend_requests_snapshot_parses_incoming_and_outgoing() {
        let text = r#"{
            "type":"friend_requests",
            "incoming":[
                {"peer_id":"alice","display_name":"Alice Prime"},
                {"peer_id":"carol"}
            ],
            "outgoing":["bob"]
        }"#;
        match serde_json::from_str::<ServerMessage>(text).expect("parse") {
            ServerMessage::FriendRequests { incoming, outgoing } => {
                assert_eq!(incoming.len(), 2);
                assert_eq!(incoming[0].peer_id, "alice");
                assert_eq!(incoming[0].display_name.as_deref(), Some("Alice Prime"));
                assert_eq!(incoming[1].display_name, None);
                assert_eq!(outgoing, vec!["bob".to_string()]);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn friend_requests_snapshot_defaults_missing_outgoing() {
        // A snapshot with only incoming (older relays) still parses.
        let text = r#"{"type":"friend_requests","incoming":[{"peer_id":"alice"}]}"#;
        match serde_json::from_str::<ServerMessage>(text).expect("parse") {
            ServerMessage::FriendRequests { incoming, outgoing } => {
                assert_eq!(incoming.len(), 1);
                assert!(outgoing.is_empty());
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn friend_requests_struct_roundtrips_through_json() {
        let snapshot = FriendRequests {
            incoming: vec![FriendRequestIncoming {
                peer_id: "alice".into(),
                display_name: Some("Alice Prime".into()),
            }],
            outgoing: vec!["bob".into()],
        };
        let text = serde_json::to_string(&snapshot).expect("serialize");
        let restored: FriendRequests = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(restored.incoming[0].peer_id, "alice");
        assert_eq!(
            restored.incoming[0].display_name.as_deref(),
            Some("Alice Prime")
        );
        assert_eq!(restored.outgoing, vec!["bob".to_string()]);
    }

    // -- UI contract --------------------------------------------------------

    #[test]
    fn contact_info_status_serializes_for_the_ui() {
        let accepted = ContactInfo {
            peer_id: "alice".into(),
            display_name: None,
            avatar_url: None,
            status: Some("accepted".into()),
        };
        let json = serde_json::to_value(accepted).expect("serialize");
        assert_eq!(json["peer_id"], "alice");
        assert_eq!(json["status"], "accepted");

        let pending = ContactInfo {
            peer_id: "bob".into(),
            display_name: None,
            avatar_url: None,
            status: Some("pending".into()),
        };
        let json = serde_json::to_value(pending).expect("serialize");
        assert_eq!(json["status"], "pending");
    }

    #[test]
    fn chat_state_includes_the_friend_request_lists() {
        let state = ChatState {
            my_peer_id: "self".into(),
            my_display_name: None,
            my_username: None,
            my_avatar_url: None,
            connected: true,
            contacts: Vec::new(),
            messages: HashMap::new(),
            presence: HashMap::new(),
            groups: Vec::new(),
            friend_requests_incoming: vec![FriendRequestIncoming {
                peer_id: "alice".into(),
                display_name: Some("Alice Prime".into()),
            }],
            friend_requests_outgoing: vec!["bob".into()],
            chat_expirations: HashMap::new(),
        };
        let json = serde_json::to_value(state).expect("serialize");
        assert_eq!(json["friend_requests_incoming"][0]["peer_id"], "alice");
        assert_eq!(json["friend_requests_outgoing"][0], "bob");
    }

    #[test]
    fn friend_request_event_serializes_for_the_ui() {
        let event = FriendRequestEvent {
            peer_id: "alice".into(),
            display_name: Some("Alice Prime".into()),
        };
        let json = serde_json::to_value(event).expect("serialize");
        assert_eq!(json["peer_id"], "alice");
        assert_eq!(json["display_name"], "Alice Prime");
    }

    #[test]
    fn contact_removed_event_serializes_for_the_ui() {
        let event = ContactRemovedEvent {
            peer_id: "bob".into(),
        };
        let json = serde_json::to_value(event).expect("serialize");
        assert_eq!(json["peer_id"], "bob");
    }

    #[test]
    fn contact_added_and_declined_events_serialize_for_the_ui() {
        let added = ContactAddedEvent {
            peer_id: "bob".into(),
            display_name: None,
        };
        let json = serde_json::to_value(added).expect("serialize");
        assert_eq!(json["peer_id"], "bob");

        let declined = FriendRequestDeclinedEvent {
            peer_id: "carol".into(),
        };
        let json = serde_json::to_value(declined).expect("serialize");
        assert_eq!(json["peer_id"], "carol");
    }

    #[test]
    fn outgoing_request_retain_removes_only_the_matching_peer() {
        let mut outgoing = vec!["alice".to_string(), "bob".to_string(), "carol".to_string()];
        retain_outgoing_peer(&mut outgoing, "bob");
        assert_eq!(outgoing, vec!["alice".to_string(), "carol".to_string()]);
        // Removing an unknown peer is a no-op, not an error.
        retain_outgoing_peer(&mut outgoing, "ghost");
        assert_eq!(outgoing.len(), 2);
    }

    #[test]
    fn append_incoming_request_adds_once_and_is_idempotent() {
        let mut incoming: Vec<FriendRequestIncoming> = Vec::new();
        assert!(append_incoming_request(
            &mut incoming,
            FriendRequestIncoming {
                peer_id: "alice".into(),
                display_name: Some("Alice Prime".into()),
            },
        ));
        assert!(
            !append_incoming_request(
                &mut incoming,
                FriendRequestIncoming {
                    peer_id: "alice".into(),
                    display_name: None,
                },
            ),
            "a duplicate push must not add a second row"
        );
        assert_eq!(incoming.len(), 1);
        assert_eq!(
            incoming[0].display_name.as_deref(),
            Some("Alice Prime"),
            "the first-learned name is kept"
        );
    }
}
