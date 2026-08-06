//! Emoji reactions: send and apply.
//!
//! A reaction is an end-to-end control signal that travels inside the normal
//! encrypted channel — a Double Ratchet session for 1:1 chats, a Megolm
//! session for groups — so the relay only ever sees ciphertext and no server
//! changes are needed. The wire form is a [`ChatPayload::Reaction`] envelope;
//! the recipient applies it to the target message and the UI toggles the pill
//! under the bubble.
//!
//! # State signalling
//!
//! Reactions are transmitted as an idempotent **state** signal, not a toggle:
//! the sender computes their own new state (`active: true` = react,
//! `active: false` = unreact) and ships it as an absolute value. The recipient
//! applies it without knowing the previous state, so a reaction envelope
//! racing ahead of its target message (or being redelivered) can never flip a
//! pill the wrong way.

use super::*;

impl RelayClient {
    /// React to a message. `peer_id` is the conversation key: a peer ID for
    /// 1:1 chats, a group ID for groups. `active` is the sender's freshly
    /// computed absolute state.
    pub async fn send_reaction(
        &self,
        peer_id: &str,
        message_id: &str,
        emoji: &str,
        active: bool,
    ) -> Result<(), RelayError> {
        if read_guard(&self.inner.groups)?.contains_key(peer_id) {
            return self.send_group_reaction(peer_id, message_id, emoji, active);
        }
        self.send_reaction_1to1(peer_id, message_id, emoji, active)
    }

    /// Encrypt and send a 1:1 reaction inside the Double Ratchet session.
    /// Best-effort like receipts: never recorded in the thread and no ack
    /// mapping — the relay acknowledges, but the reaction is not a chat
    /// message and must not surface as one.
    fn send_reaction_1to1(
        &self,
        peer_id: &str,
        message_id: &str,
        emoji: &str,
        active: bool,
    ) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        let content = ChatPayload::Reaction(ReactionPayload::new(message_id, emoji, active));
        let (olm, session_id) = {
            let mut sessions = mutex_guard(&self.inner.sessions)?;
            let session = sessions
                .get_mut(peer_id)
                .ok_or_else(|| RelayError::NoSession(peer_id.to_string()))?;
            let session_id = session.session_id();
            let olm = session.encrypt(serde_json::to_vec(&content)?)?;
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

    /// Apply an inbound reaction to a conversation: update the in-memory
    /// message (when loaded), mirror the state to the store and notify the
    /// UI with a `message-reaction` event.
    pub(crate) fn handle_reaction(
        &self,
        peer_id: &str,
        message_id: &str,
        sender: &str,
        emoji: &str,
        active: bool,
    ) -> Result<(), RelayError> {
        self.apply_reaction_in_memory(peer_id, message_id, sender, emoji, active)?;
        if let Ok(store) = self.store_guard() {
            if let Some(store) = store.as_ref() {
                let _ = store.set_reaction_state(peer_id, message_id, sender, emoji, active);
            }
        }
        let _ = self.inner.app.emit(
            "message-reaction",
            ReactionEvent {
                peer_id: peer_id.to_string(),
                message_id: message_id.to_string(),
                sender: sender.to_string(),
                emoji: emoji.to_string(),
                active,
            },
        );
        Ok(())
    }

    /// Update the in-memory reaction list of one message. No-op when the
    /// message is not loaded yet: the reaction is persisted in the store and
    /// hydrates together with the message on the next load.
    fn apply_reaction_in_memory(
        &self,
        peer_id: &str,
        message_id: &str,
        sender: &str,
        emoji: &str,
        active: bool,
    ) -> Result<(), RelayError> {
        let mut messages = write_guard(&self.inner.messages)?;
        let thread = messages.entry(peer_id.to_string()).or_default();
        if let Some(message) = thread.iter_mut().find(|m| m.id == message_id) {
            if active {
                if let Some(existing) = message.reactions.iter_mut().find(|r| r.sender == sender) {
                    existing.emoji = emoji.to_string();
                } else {
                    message.reactions.push(ReactionView {
                        sender: sender.to_string(),
                        emoji: emoji.to_string(),
                    });
                }
            } else {
                message.reactions.retain(|r| r.sender != sender);
            }
        }
        Ok(())
    }
}
