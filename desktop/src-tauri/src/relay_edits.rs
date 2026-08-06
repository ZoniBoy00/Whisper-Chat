//! Message editing and delete-for-everyone: send and apply.
//!
//! Both are end-to-end control signals that travel inside the normal
//! encrypted channel — a Double Ratchet session for 1:1 chats, a Megolm
//! session for groups — so the relay only ever sees ciphertext and no server
//! changes are needed. The wire forms are [`ChatPayload::Edit`] and
//! [`ChatPayload::Delete`] envelopes; the recipient applies them to the target
//! message (keyed by the SENDER's shared message id) and notifies the UI.
//!
//! # Trust note
//!
//! The edit/delete payload carries only the target message id, which any
//! group member can observe. The UI only offers the actions on the user's OWN
//! outgoing messages; the wire itself does not re-authenticate the editor
//! (message ids are not signed). This matches the MVP scope of reactions.

use super::*;

impl RelayClient {
    /// Edit one of our own messages: replace its text on every recipient's
    /// device. `peer_id` is the conversation key (peer ID for 1:1, group ID
    /// for groups).
    pub fn edit_message(
        &self,
        peer_id: &str,
        message_id: &str,
        new_text: &str,
    ) -> Result<(), RelayError> {
        // Only our own messages may be edited; the UI hides the action on
        // incoming ones, this check is the backend's safety net.
        let own = self
            .inner
            .messages
            .read()
            .map(|messages| {
                messages
                    .get(peer_id)
                    .map(|thread| thread.iter().any(|m| m.id == message_id && m.outgoing))
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !own {
            return Err(RelayError::MessageNotFound(
                peer_id.to_string(),
                message_id.to_string(),
            ));
        }
        // Apply locally first, then broadcast the edit signal.
        self.apply_edit(peer_id, message_id, new_text)?;
        let payload = ChatPayload::Edit(EditPayload::new(message_id, new_text));
        let bytes = serde_json::to_vec(&payload)?;
        if read_guard(&self.inner.groups)?.contains_key(peer_id) {
            self.send_group_payload(
                peer_id,
                &bytes,
                relay_groups::GroupSend {
                    record: false,
                    client_id: String::new(),
                    quote: None,
                    message_id: None,
                    display_text: None,
                    expires_at: None,
                },
            )
        } else {
            self.send_control_1to1(peer_id, &bytes)
        }
    }

    /// Delete one of our own messages on every recipient's device. `peer_id`
    /// is the conversation key.
    pub fn delete_for_everyone(&self, peer_id: &str, message_id: &str) -> Result<(), RelayError> {
        let own = self
            .inner
            .messages
            .read()
            .map(|messages| {
                messages
                    .get(peer_id)
                    .map(|thread| thread.iter().any(|m| m.id == message_id && m.outgoing))
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !own {
            return Err(RelayError::MessageNotFound(
                peer_id.to_string(),
                message_id.to_string(),
            ));
        }
        // Delete locally first (memory + store), then broadcast the signal.
        self.delete_message(peer_id, message_id)?;
        self.emit_deleted(peer_id, &[message_id.to_string()]);
        let payload = ChatPayload::Delete(DeletePayload::new(message_id));
        let bytes = serde_json::to_vec(&payload)?;
        if read_guard(&self.inner.groups)?.contains_key(peer_id) {
            self.send_group_payload(
                peer_id,
                &bytes,
                relay_groups::GroupSend {
                    record: false,
                    client_id: String::new(),
                    quote: None,
                    message_id: None,
                    display_text: None,
                    expires_at: None,
                },
            )
        } else {
            self.send_control_1to1(peer_id, &bytes)
        }
    }

    /// Encrypt and send a control-signal plaintext (edit/delete) inside the
    /// 1:1 Double Ratchet session. Best-effort like receipts: never recorded
    /// in the thread and no ack mapping.
    fn send_control_1to1(&self, peer_id: &str, bytes: &[u8]) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        let (olm, session_id) = {
            let mut sessions = mutex_guard(&self.inner.sessions)?;
            let session = sessions
                .get_mut(peer_id)
                .ok_or_else(|| RelayError::NoSession(peer_id.to_string()))?;
            let session_id = session.session_id();
            let olm = session.encrypt(bytes)?;
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

    /// Apply an inbound edit: update the target message's text (and mark it
    /// edited), mirror it to the store and notify the UI.
    pub(crate) fn handle_edit(
        &self,
        peer_id: &str,
        message_id: &str,
        new_text: &str,
    ) -> Result<(), RelayError> {
        let updated = {
            let mut messages = write_guard(&self.inner.messages)?;
            let thread = messages.entry(peer_id.to_string()).or_default();
            match thread.iter_mut().find(|m| m.id == message_id) {
                Some(message) => {
                    message.text = new_text.to_string();
                    message.edited = true;
                    true
                }
                None => false,
            }
        };
        if updated {
            if let Ok(store) = self.store_guard() {
                if let Some(store) = store.as_ref() {
                    let _ = store.edit_message(message_id, new_text);
                }
            }
        }
        // Emit regardless: a late-arriving edit for a message the UI has not
        // loaded yet is a harmless no-op on the render side.
        let _ = self.inner.app.emit(
            "chat-message-edited",
            ChatMessageEditedEvent {
                peer_id: peer_id.to_string(),
                message_id: message_id.to_string(),
                text: new_text.to_string(),
            },
        );
        Ok(())
    }

    /// Apply an inbound delete-for-everyone: remove the target message from
    /// memory and the store, then notify the UI.
    pub(crate) fn handle_delete(&self, peer_id: &str, message_id: &str) -> Result<(), RelayError> {
        self.delete_message(peer_id, message_id)?;
        self.emit_deleted(peer_id, &[message_id.to_string()]);
        Ok(())
    }

    /// Apply a local edit (the sender's own optimistic update): same path as
    /// [`RelayClient::handle_edit`] without the emit (the UI already re-renders
    /// the composer state).
    fn apply_edit(
        &self,
        peer_id: &str,
        message_id: &str,
        new_text: &str,
    ) -> Result<(), RelayError> {
        let mut messages = write_guard(&self.inner.messages)?;
        let thread = messages.entry(peer_id.to_string()).or_default();
        if let Some(message) = thread.iter_mut().find(|m| m.id == message_id) {
            message.text = new_text.to_string();
            message.edited = true;
        }
        if let Ok(store) = self.store_guard() {
            if let Some(store) = store.as_ref() {
                let _ = store.edit_message(message_id, new_text);
            }
        }
        Ok(())
    }

    /// Emit a `chat-message-deleted` event for one or more message ids in a
    /// thread (shared with the disappearing-message purge path).
    pub(crate) fn emit_deleted(&self, peer_id: &str, message_ids: &[String]) {
        let _ = self.inner.app.emit(
            "chat-message-deleted",
            ChatMessageDeletedEvent {
                peer_id: peer_id.to_string(),
                message_ids: message_ids.to_vec(),
            },
        );
    }
}
