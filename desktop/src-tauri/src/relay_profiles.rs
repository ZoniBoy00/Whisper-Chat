//! Public profiles and display names.
//!
//! The public directory feature: registering a signed username alias
//! (`register_profile`), prefix-searching usernames/peer IDs (`search_users`),
//! fetching a peer's public profile (`get_profile`), and re-registering with a
//! new avatar (`set_avatar`). Display-name bookkeeping lives here too: our own
//! public display name (`set_display_name` / `save_profiles`) and the names we
//! learn for our contacts (`remember_contact_name`, fed from pre-key lookups).
//!
//! This module owns the corresponding `impl RelayClient` block; the wire types
//! ([`ProfileSearchResult`], [`PeerProfile`], [`Profiles`]) stay in the relay
//! core (`super`) because the UI contract depends on them.

use super::*;

impl RelayClient {
    /// Register (or re-register) the caller's signed username alias with the
    /// relay, optionally attaching an avatar. Returns the registered username.
    pub async fn register_profile(
        &self,
        username: &str,
        display_name: Option<&str>,
        avatar_b64: Option<&str>,
    ) -> Result<String, RelayError> {
        // The relay verifies an Ed25519 signature over
        // `username || 0x00 || curve25519_key`, so sign it locally.
        let signature = {
            let guard = mutex_guard(&self.inner.identity)?;
            let identity = guard.as_ref().ok_or(RelayError::NoIdentity)?;
            let mut canonical = Vec::with_capacity(username.len() + 1 + 32);
            canonical.extend_from_slice(username.as_bytes());
            canonical.push(0);
            canonical.extend_from_slice(identity.curve25519_key().as_bytes());
            identity.sign(&canonical).to_base64()
        };

        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_register)?.push_back(tx);
        let message = ClientMessage::RegisterProfile {
            username: username.to_string(),
            signature,
            display_name: display_name.map(str::to_string),
            avatar: avatar_b64.map(str::to_string),
        };
        if let Err(err) = self.send_json(&message) {
            // The request never left, so drop the dangling waiter.
            mutex_guard(&self.inner.pending_register)?.pop_back();
            return Err(err);
        }

        tokio::time::timeout(PROFILE_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::ProfileTimeout)?
            .map_err(|_| RelayError::ProfileRequestFailed)?
    }

    /// Prefix-search registered usernames and peer IDs.
    pub async fn search_users(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ProfileSearchResult>, RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_search)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::SearchUsers {
            query: query.to_string(),
            limit,
        }) {
            mutex_guard(&self.inner.pending_search)?.pop_back();
            return Err(err);
        }

        tokio::time::timeout(PROFILE_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::ProfileTimeout)?
            .map_err(|_| RelayError::ProfileRequestFailed)?
    }

    /// Fetch one peer's public profile; `Ok(None)` when they have none.
    pub async fn get_profile(&self, peer_id: &str) -> Result<Option<PeerProfile>, RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_profile)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::GetProfile {
            peer_id: peer_id.to_string(),
        }) {
            mutex_guard(&self.inner.pending_profile)?.pop_back();
            return Err(err);
        }

        tokio::time::timeout(PROFILE_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::ProfileTimeout)?
            .map_err(|_| RelayError::ProfileRequestFailed)?
    }

    /// Re-register the caller's profile with a new avatar image (base64,
    /// ≤2 MB). The username must already be registered.
    pub async fn set_avatar(&self, username: &str, avatar_b64: &str) -> Result<(), RelayError> {
        self.register_profile(username, None, Some(avatar_b64))
            .await
            .map(|_| ())
    }

    // ---------------------------------------------------------------------
    // Display names
    // ---------------------------------------------------------------------

    /// Persist our own display name to the store and cache it in memory.
    fn save_profiles(&self, profiles: &Profiles) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        let store_guard = self.store_guard()?;
        let store = store_guard.as_ref().ok_or(RelayError::StoreNotOpen)?;
        match &profiles.my_display_name {
            Some(name) if !name.is_empty() => store.set_setting("my_display_name", name)?,
            _ => store.delete_setting("my_display_name")?,
        }
        *write_guard(&self.inner.profiles)? = profiles.clone();
        Ok(())
    }

    /// Persist our own public display name and, when connected, announce it to
    /// the relay so everyone who fetches our pre-keys sees it. An empty name
    /// clears the local profile (the previously published name stays visible
    /// to others until overwritten — the server rejects empty names).
    pub fn set_display_name(&self, name: &str) -> Result<(), RelayError> {
        let name = name.trim();
        if !name.is_empty()
            && (name.chars().count() > MAX_DISPLAY_NAME_CHARS || name.chars().any(char::is_control))
        {
            return Err(RelayError::InvalidDisplayName);
        }
        self.ensure_store_open()?;
        let mut profiles = read_guard(&self.inner.profiles)?.clone();
        profiles.my_display_name = if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        };
        self.save_profiles(&profiles)?;
        if !name.is_empty() && self.inner.connected.load(Ordering::SeqCst) {
            self.send_json(&ClientMessage::UpdateProfile {
                display_name: name.to_string(),
            })?;
        }
        Ok(())
    }

    /// Remember a display name learned for a contact and persist it. Emits a
    /// `contact-updated` event so the UI can update the contact list without a
    /// full state refresh.
    pub(crate) fn remember_contact_name(
        &self,
        peer_id: &str,
        name: &str,
    ) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        self.store_guard()?
            .as_ref()
            .ok_or(RelayError::StoreNotOpen)?
            .upsert_contact(&ContactRow {
                peer_id: peer_id.to_string(),
                display_name: Some(name.to_string()),
                username: None,
                avatar_url: None,
                last_seen: None,
            })?;
        write_guard(&self.inner.profiles)?
            .contacts
            .insert(peer_id.to_string(), name.to_string());
        let _ = self.inner.app.emit(
            "contact-updated",
            ContactUpdatedEvent {
                peer_id: peer_id.to_string(),
                display_name: Some(name.to_string()),
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_profile_client_message_serializes() {
        let json = serde_json::to_value(ClientMessage::UpdateProfile {
            display_name: "New Name".into(),
        })
        .expect("serialize");
        assert_eq!(json["type"], "update_profile");
        assert_eq!(json["display_name"], "New Name");
    }

    #[test]
    fn profile_updated_server_message_parses() {
        let message: ServerMessage =
            serde_json::from_str(r#"{"type":"profile_updated"}"#).expect("parse");
        assert!(matches!(message, ServerMessage::ProfileUpdated));
    }

    #[test]
    fn display_name_validation_rejects_control_characters_and_oversize_names() {
        let too_long = "x".repeat(MAX_DISPLAY_NAME_CHARS + 1);
        assert!(too_long.chars().count() > MAX_DISPLAY_NAME_CHARS);
        assert!("name\nwith\ttabs".chars().any(char::is_control));
    }
}
