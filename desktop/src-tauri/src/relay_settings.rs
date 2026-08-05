//! Settings persistence and end-to-end receipts.
//!
//! Persisted preferences (`get_settings`, `set_relay_url`, `set_theme`,
//! `update_settings`) plus the privacy toggle `set_privacy`, which also
//! round-trips to the relay so other peers observe the choice. End-to-end
//! receipts also live here: sending read/typing receipts encrypted inside the
//! ratchet session (`send_receipt`, `send_typing`) and applying inbound ones
//! (`handle_receipt`), which flips our outgoing messages to "read" and relays
//! typing indicators to the UI.
//!
//! This module owns the corresponding `impl RelayClient` block; the
//! [`Settings`] / [`SettingsPatch`] types stay in the relay core (`super`)
//! because the UI contract depends on them.

use super::*;

impl RelayClient {
    // ---------------------------------------------------------------------
    // Settings persistence
    // ---------------------------------------------------------------------

    /// Return the persisted settings, hydrated from the store on first use.
    pub fn get_settings(&self) -> Result<Settings, RelayError> {
        self.ensure_store_open()?;
        let settings = read_guard(&self.inner.settings)?.clone();
        Ok(settings)
    }

    /// Persist `settings` to the store and cache them in memory.
    pub(crate) fn save_settings(&self, settings: &Settings) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        let store_guard = self.store_guard()?;
        let store = store_guard.as_ref().ok_or(RelayError::StoreNotOpen)?;
        match &settings.relay_url {
            Some(url) if !url.is_empty() => store.set_setting("relay_url", url)?,
            _ => store.delete_setting("relay_url")?,
        }
        match &settings.theme {
            Some(theme) if !theme.is_empty() => store.set_setting("theme", theme)?,
            _ => store.delete_setting("theme")?,
        }
        store.set_setting("presence_visible", setting_str(settings.presence_visible))?;
        store.set_setting("read_receipts", setting_str(settings.read_receipts))?;
        store.set_setting("typing_indicator", setting_str(settings.typing_indicator))?;
        store.set_setting(
            "notifications_enabled",
            setting_str(settings.notifications_enabled),
        )?;
        store.set_setting(
            "notification_preview",
            setting_str(settings.notification_preview),
        )?;
        store.set_setting(
            "notification_sound",
            setting_str(settings.notification_sound),
        )?;
        match &settings.language {
            Some(lang) if !lang.is_empty() => store.set_setting("language", lang)?,
            _ => store.delete_setting("language")?,
        }
        store.set_setting("minimize_to_tray", setting_str(settings.minimize_to_tray))?;
        store.set_setting("enter_to_send", setting_str(settings.enter_to_send))?;
        match &settings.message_font_scale {
            Some(scale) if !scale.is_empty() => store.set_setting("message_font_scale", scale)?,
            _ => store.delete_setting("message_font_scale")?,
        }
        store.set_setting("autostart", setting_str(settings.autostart))?;
        *write_guard(&self.inner.settings)? = settings.clone();
        Ok(())
    }

    /// Persist a new relay endpoint. If the client is connected to a different
    /// URL, the connection is dropped so the UI can reconnect to the new
    /// address.
    pub fn set_relay_url(&self, url: &str) -> Result<(), RelayError> {
        let mut settings = self.get_settings()?;
        let changed = settings.relay_url.as_deref() != Some(url);
        settings.relay_url = Some(url.to_string());
        self.save_settings(&settings)?;
        if changed && self.inner.connected.load(Ordering::SeqCst) {
            self.disconnect()?;
        }
        Ok(())
    }

    /// Persist a new UI theme preference.
    pub fn set_theme(&self, theme: &str) -> Result<(), RelayError> {
        let mut settings = self.get_settings()?;
        settings.theme = Some(theme.to_string());
        self.save_settings(&settings)
    }

    /// Toggle whether our online status and last-seen are visible to other
    /// peers. The preference is persisted locally so it restores on restart,
    /// and sent to the relay (best-effort) so it takes effect for others
    /// immediately. The relay answers with `privacy_updated`.
    pub fn set_privacy(&self, presence_visible: bool) -> Result<(), RelayError> {
        let mut settings = self.get_settings()?;
        settings.presence_visible = presence_visible;
        self.save_settings(&settings)?;
        if self.inner.connected.load(Ordering::SeqCst) {
            self.send_json(&ClientMessage::SetPrivacy { presence_visible })?;
        }
        Ok(())
    }

    /// Apply a partial boolean-preferences update (read receipts, typing
    /// indicator, notifications) and persist it. Each `Some` field overwrites
    /// the stored value; `None` fields are left untouched.
    pub fn update_settings(&self, patch: &SettingsPatch) -> Result<(), RelayError> {
        let mut settings = self.get_settings()?;
        if let Some(value) = patch.read_receipts {
            settings.read_receipts = value;
        }
        if let Some(value) = patch.typing_indicator {
            settings.typing_indicator = value;
        }
        if let Some(value) = patch.notifications_enabled {
            settings.notifications_enabled = value;
        }
        if let Some(value) = patch.notification_preview {
            settings.notification_preview = value;
        }
        if let Some(value) = patch.notification_sound {
            settings.notification_sound = value;
        }
        if let Some(value) = patch.language.clone() {
            settings.language = (!value.is_empty()).then_some(value);
        }
        if let Some(value) = patch.minimize_to_tray {
            settings.minimize_to_tray = value;
        }
        if let Some(value) = patch.enter_to_send {
            settings.enter_to_send = value;
        }
        if let Some(value) = patch.message_font_scale.clone() {
            settings.message_font_scale = (!value.is_empty()).then_some(value);
        }
        if let Some(value) = patch.autostart {
            settings.autostart = value;
        }
        self.save_settings(&settings)
    }

    // ---------------------------------------------------------------------
    // End-to-end receipts (read + typing)
    // ---------------------------------------------------------------------

    /// Send an end-to-end typing indicator (or the "stopped" signal) to a
    /// peer, encrypted inside the established session. When the typing
    /// indicator is disabled in settings this is a no-op — the peer never
    /// learns that we are typing.
    pub fn send_typing(&self, peer_id: &str, is_typing: bool) -> Result<(), RelayError> {
        if !read_guard(&self.inner.settings)?.typing_indicator {
            return Ok(());
        }
        let kind = if is_typing {
            ReceiptKind::Typing
        } else {
            ReceiptKind::TypingStopped
        };
        self.send_receipt(peer_id, kind)
    }

    /// Encrypt and send an end-to-end receipt inside the session with
    /// `peer_id`. The receipt is serialized as [`e2ee_core::EnvelopeContent`]
    /// and encrypted like an ordinary message, so the relay only ever sees the
    /// ciphertext of a [`e2ee_core::Message`].
    pub(crate) fn send_receipt(&self, peer_id: &str, kind: ReceiptKind) -> Result<(), RelayError> {
        let my_peer_id = self.my_peer_id()?;
        let content = EnvelopeContent::Receipt { kind };
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

    /// Apply an inbound end-to-end receipt. Read receipts flip all of our
    /// outgoing messages to the sender to "read"; typing receipts are relayed
    /// to the UI (with a 5-second auto-timeout that emits `false` unless a
    /// newer indicator arrives first).
    pub(crate) fn handle_receipt(&self, sender: &str, kind: ReceiptKind) -> Result<(), RelayError> {
        match kind {
            ReceiptKind::Read => {
                // A single `Read` receipt acknowledges every message the peer
                // has read so far, so all unread outgoing messages to them
                // flip at once. Each flip emits one `message-status` event.
                let flipped = {
                    let mut messages = write_guard(&self.inner.messages)?;
                    apply_read(&mut messages, sender)
                };
                for client_id in &flipped {
                    self.persist_message_status(client_id, "read")?;
                }
                for client_id in flipped {
                    let _ = self.inner.app.emit(
                        "message-status",
                        MessageStatusEvent {
                            client_id,
                            status: "read".to_string(),
                        },
                    );
                }
                Ok(())
            }
            ReceiptKind::Typing | ReceiptKind::TypingStopped => {
                let is_typing = kind == ReceiptKind::Typing;
                let mut timers = mutex_guard(&self.inner.typing_timeouts)?;
                let generation = timers.entry(sender.to_string()).or_insert(0);
                *generation += 1;
                let generation = *generation;
                drop(timers);
                let _ = self.inner.app.emit(
                    "typing",
                    TypingEvent {
                        peer_id: sender.to_string(),
                        is_typing,
                    },
                );
                // A "stopped" receipt cancels any pending auto-timeout by
                // bumping the generation; nothing more is scheduled for it.
                if is_typing {
                    let client = self.clone();
                    let peer = sender.to_string();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(TYPING_TIMEOUT_SECS))
                            .await;
                        let timers = match mutex_guard(&client.inner.typing_timeouts) {
                            Ok(t) => t,
                            Err(_) => return,
                        };
                        if timers.get(&peer).copied() == Some(generation) {
                            drop(timers);
                            let _ = client.inner.app.emit(
                                "typing",
                                TypingEvent {
                                    peer_id: peer.clone(),
                                    is_typing: false,
                                },
                            );
                        }
                    });
                }
                Ok(())
            }
        }
    }
}

/// Pure helper for [`RelayClient::handle_receipt`]: flip every outgoing
/// message to `peer_id` to "read", returning the client ids that changed so
/// the caller can notify the UI. Incoming messages and already-read messages
/// are left untouched.
fn apply_read(messages: &mut HashMap<String, Vec<UIMessage>>, peer_id: &str) -> Vec<String> {
    let mut flipped = Vec::new();
    if let Some(msgs) = messages.get_mut(peer_id) {
        for message in msgs.iter_mut() {
            if message.outgoing && message.status != "read" {
                message.status = "read".to_string();
                flipped.push(message.id.clone());
            }
        }
    }
    flipped
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // Settings
    // ---------------------------------------------------------------------

    #[test]
    fn settings_parse_handles_missing_fields() {
        let settings: Settings =
            serde_json::from_str(r#"{"theme":"dark"}"#).expect("partial settings must parse");
        assert_eq!(settings.relay_url, None);
        assert_eq!(settings.theme.as_deref(), Some("dark"));
        // Opt-out preferences default to enabled when the field is missing.
        assert!(settings.presence_visible);
        assert!(settings.read_receipts);
        assert!(settings.typing_indicator);
        assert!(settings.notifications_enabled);
        assert!(settings.notification_preview);
        assert!(settings.notification_sound);
        assert_eq!(settings.language, None);
        // New behavioral preferences keep their documented defaults too:
        // minimize-to-tray and autostart opt OUT by default, Enter-to-send is on.
        assert!(!settings.minimize_to_tray);
        assert!(settings.enter_to_send);
        assert_eq!(settings.message_font_scale, None);
        assert!(!settings.autostart);
    }

    #[test]
    fn settings_parse_honours_explicit_opt_out_fields() {
        let settings: Settings = serde_json::from_str(
            r#"{"presence_visible":false,"read_receipts":false,"typing_indicator":false,"notifications_enabled":false,"notification_preview":false,"notification_sound":false,"language":"fi","minimize_to_tray":true,"enter_to_send":false,"message_font_scale":"large","autostart":true}"#,
        )
        .expect("full settings must parse");
        assert!(!settings.presence_visible);
        assert!(!settings.read_receipts);
        assert!(!settings.typing_indicator);
        assert!(!settings.notifications_enabled);
        assert!(!settings.notification_preview);
        assert!(!settings.notification_sound);
        assert_eq!(settings.language.as_deref(), Some("fi"));
        assert!(settings.minimize_to_tray);
        assert!(!settings.enter_to_send);
        assert_eq!(settings.message_font_scale.as_deref(), Some("large"));
        assert!(settings.autostart);
    }

    #[test]
    fn settings_patch_defaults_every_field_to_none() {
        let patch: SettingsPatch =
            serde_json::from_str(r#"{"read_receipts":true}"#).expect("partial patch must parse");
        assert_eq!(patch.read_receipts, Some(true));
        assert_eq!(patch.typing_indicator, None);
        assert_eq!(patch.notifications_enabled, None);
        assert_eq!(patch.notification_preview, None);
        assert_eq!(patch.notification_sound, None);
        assert_eq!(patch.language, None);
        assert_eq!(patch.minimize_to_tray, None);
        assert_eq!(patch.enter_to_send, None);
        assert_eq!(patch.message_font_scale, None);
        assert_eq!(patch.autostart, None);
    }

    #[test]
    fn settings_patch_carries_language_and_sound() {
        let patch: SettingsPatch = serde_json::from_str(
            r#"{"notification_sound":false,"language":"fi","minimize_to_tray":true,"enter_to_send":false,"message_font_scale":"small","autostart":true}"#,
        )
        .expect("partial patch must parse");
        assert_eq!(patch.notification_sound, Some(false));
        assert_eq!(patch.language.as_deref(), Some("fi"));
        assert_eq!(patch.minimize_to_tray, Some(true));
        assert_eq!(patch.enter_to_send, Some(false));
        assert_eq!(patch.message_font_scale.as_deref(), Some("small"));
        assert_eq!(patch.autostart, Some(true));
    }

    #[test]
    fn setting_bool_parses_strings_and_falls_back_to_default() {
        assert!(setting_bool(Some("true".into()), false));
        assert!(!setting_bool(Some("false".into()), true));
        assert!(setting_bool(None, true));
        assert!(!setting_bool(Some("garbage".into()), false));
        assert_eq!(setting_str(true), "true");
        assert_eq!(setting_str(false), "false");
    }

    #[test]
    fn relay_url_resolution_prefers_settings_then_env_then_default() {
        let custom = Settings {
            relay_url: Some("ws://custom".into()),
            ..Settings::default()
        };
        assert_eq!(resolve_relay_url(&custom, Some("ws://env")), "ws://custom");

        let blank = Settings {
            relay_url: Some(String::new()),
            ..Settings::default()
        };
        assert_eq!(resolve_relay_url(&blank, Some("ws://env")), "ws://env");
        assert_eq!(resolve_relay_url(&blank, None), DEFAULT_RELAY_URL);

        let defaults = Settings::default();
        assert_eq!(resolve_relay_url(&defaults, Some("ws://env")), "ws://env");
        assert_eq!(resolve_relay_url(&defaults, None), DEFAULT_RELAY_URL);
    }

    #[test]
    fn set_privacy_client_message_serializes() {
        let hide = serde_json::to_value(ClientMessage::SetPrivacy {
            presence_visible: false,
        })
        .expect("serialize");
        assert_eq!(hide["type"], "set_privacy");
        assert_eq!(hide["presence_visible"], false);

        let show = serde_json::to_value(ClientMessage::SetPrivacy {
            presence_visible: true,
        })
        .expect("serialize");
        assert_eq!(show["presence_visible"], true);
    }

    #[test]
    fn privacy_updated_server_message_parses() {
        let message: ServerMessage =
            serde_json::from_str(r#"{"type":"privacy_updated"}"#).expect("parse");
        assert!(matches!(message, ServerMessage::PrivacyUpdated));
    }

    // ---------------------------------------------------------------------
    // Read receipt bookkeeping
    // ---------------------------------------------------------------------

    #[test]
    fn apply_read_flips_outgoing_peer_messages_and_returns_their_ids() {
        let mut messages: HashMap<String, Vec<UIMessage>> = HashMap::new();
        messages.insert(
            "peer-1".into(),
            vec![
                UIMessage {
                    id: "sent-1".into(),
                    text: "a".into(),
                    outgoing: true,
                    timestamp: 0,
                    status: "sent".into(),
                },
                UIMessage {
                    id: "delivered-1".into(),
                    text: "b".into(),
                    outgoing: true,
                    timestamp: 0,
                    status: "delivered".into(),
                },
            ],
        );
        // A different peer's messages must be left alone.
        messages.insert(
            "peer-2".into(),
            vec![UIMessage {
                id: "delivered-2".into(),
                text: "c".into(),
                outgoing: true,
                timestamp: 0,
                status: "delivered".into(),
            }],
        );

        let flipped = apply_read(&mut messages, "peer-1");
        assert_eq!(flipped, vec!["sent-1", "delivered-1"]);
        assert_eq!(messages["peer-1"][0].status, "read");
        assert_eq!(messages["peer-1"][1].status, "read");
        assert_eq!(messages["peer-2"][0].status, "delivered");
    }

    #[test]
    fn apply_read_skips_incoming_and_already_read_messages() {
        let mut messages: HashMap<String, Vec<UIMessage>> = HashMap::new();
        messages.insert(
            "peer-1".into(),
            vec![
                UIMessage {
                    id: "in-1".into(),
                    text: "incoming".into(),
                    outgoing: false,
                    timestamp: 0,
                    status: "delivered".into(),
                },
                UIMessage {
                    id: "read-1".into(),
                    text: "already read".into(),
                    outgoing: true,
                    timestamp: 0,
                    status: "read".into(),
                },
            ],
        );

        let flipped = apply_read(&mut messages, "peer-1");
        assert!(flipped.is_empty());
        assert_eq!(messages["peer-1"][0].status, "delivered");
        assert_eq!(messages["peer-1"][1].status, "read");
    }

    #[test]
    fn apply_read_unknown_peer_is_a_noop() {
        let mut messages: HashMap<String, Vec<UIMessage>> = HashMap::new();
        messages.insert(
            "peer-1".into(),
            vec![UIMessage {
                id: "out-1".into(),
                text: "hi".into(),
                outgoing: true,
                timestamp: 0,
                status: "delivered".into(),
            }],
        );

        assert!(apply_read(&mut messages, "ghost").is_empty());
        assert_eq!(messages["peer-1"][0].status, "delivered");
    }

    // ---------------------------------------------------------------------
    // Receipt transport (encrypted inside a Message)
    // ---------------------------------------------------------------------

    #[test]
    fn receipt_transport_roundtrips_inside_the_ratchet_session() {
        // A receipt is serialized as EnvelopeContent, encrypted like any
        // message, and recovered by parsing the decrypted plaintext.
        let alice = Identity::new();
        let mut bob = Identity::new();
        let bundle = bob.pre_key_bundle(5);
        let mut alice_session = ChatSession::create_outbound(&alice, &bundle).expect("session");
        let first = alice_session.encrypt(b"hello bob").expect("encrypt");
        let pre_key = match first {
            OlmMessage::PreKey(pk) => pk,
            OlmMessage::Normal(_) => panic!("first message must be a pre-key message"),
        };
        let inbound = ChatSession::create_inbound(&mut bob, alice.curve25519_key(), &pre_key)
            .expect("inbound session");
        let mut bob_session = inbound.session;

        // Bob sends a read receipt back to Alice.
        let content = EnvelopeContent::Receipt {
            kind: ReceiptKind::Read,
        };
        let payload = serde_json::to_vec(&content).expect("serialize receipt");
        let ciphertext = bob_session.encrypt(payload).expect("encrypt receipt");
        let plaintext = alice_session.decrypt(&ciphertext).expect("decrypt receipt");

        let restored: EnvelopeContent = serde_json::from_slice(&plaintext).expect("parse");
        assert_eq!(
            restored,
            EnvelopeContent::Receipt {
                kind: ReceiptKind::Read
            }
        );
    }
}
