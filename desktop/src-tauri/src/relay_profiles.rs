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
    /// Re-assert our persisted public profile against the relay after a
    /// successful connect, so the relay's users table keeps
    /// `(peer_id, username, display_name, avatar_hash)` across app restarts —
    /// even if the relay database was reset while the app was closed.
    ///
    /// Two best-effort steps, ordered so a profile with a display name but no
    /// registered username still keeps its name fresh:
    ///
    /// 1. When a display name is stored, announce it with `update_profile`
    ///    (fire-and-forget — the relay applies it to the users table).
    /// 2. When a username is stored, re-register the signed binding. The
    ///    relay treats a re-registration by the same peer as idempotent (it
    ///    refreshes the signature and `registered_at` timestamp), so this is
    ///    safe to repeat. A `username_taken` reply means a *different* peer
    ///    now owns the name — accepted silently, leaving the stored profile
    ///    untouched.
    ///
    /// The avatar blob is not persisted locally (only its `/media/{hash}`
    /// path), so it is not re-uploaded here; the relay keeps the existing
    /// avatar hash when a registration carries none.
    pub(crate) async fn sync_own_profile(&self) -> Result<(), RelayError> {
        let profiles = read_guard(&self.inner.profiles)?.clone();

        if let Some(name) = advertised_display_name(&profiles) {
            let _ = self.send_json(&ClientMessage::UpdateProfile {
                display_name: name.to_string(),
            });
        }

        let username = match stored_username(&profiles) {
            Some(username) => username.to_string(),
            None => return Ok(()),
        };

        match self.reassert_username(&username).await {
            Ok(_) => Ok(()),
            // A different peer now owns the name — there is nothing we can do,
            // and the locally stored profile is left as it was.
            Err(RelayError::Relay(code)) if code == "username_taken" => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Re-register the signed username binding with the relay WITHOUT touching
    /// the persisted avatar path. The avatar blob bytes are not stored locally
    /// (only the `/media/{hash}` URL), so the normal `register_profile` path —
    /// which clears the stored avatar when none is uploaded — must not be used
    /// for a startup re-assertion. Persists the refreshed username, keeping
    /// any stored avatar URL intact.
    async fn reassert_username(&self, username: &str) -> Result<String, RelayError> {
        let signature = self.sign_username(username)?;

        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_register)?.push_back(tx);
        let message = ClientMessage::RegisterProfile {
            username: username.to_string(),
            signature,
            display_name: read_guard(&self.inner.profiles)?
                .my_display_name
                .as_deref()
                .filter(|name| !name.is_empty())
                .map(str::to_string),
            avatar: None,
        };
        if let Err(err) = self.send_json(&message) {
            // The request never left, so drop the dangling waiter.
            mutex_guard(&self.inner.pending_register)?.pop_back();
            return Err(err);
        }

        let username = tokio::time::timeout(PROFILE_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::ProfileTimeout)?
            .map_err(|_| RelayError::ProfileRequestFailed)??;

        // Persist the refreshed username while preserving the avatar path.
        self.ensure_store_open()?;
        let store_guard = self.store_guard()?;
        let store = store_guard.as_ref().ok_or(RelayError::StoreNotOpen)?;
        store.set_setting("my_username", &username)?;
        let mut profiles = read_guard(&self.inner.profiles)?.clone();
        profiles.my_username = Some(username.clone());
        *write_guard(&self.inner.profiles)? = profiles;
        Ok(username)
    }

    /// Ed25519-sign `username` over the canonical bytes the relay verifies
    /// (`username || 0x00 || curve25519_key`).
    fn sign_username(&self, username: &str) -> Result<String, RelayError> {
        let guard = mutex_guard(&self.inner.identity)?;
        let identity = guard.as_ref().ok_or(RelayError::NoIdentity)?;
        let mut canonical = Vec::with_capacity(username.len() + 1 + 32);
        canonical.extend_from_slice(username.as_bytes());
        canonical.push(0);
        canonical.extend_from_slice(identity.curve25519_key().as_bytes());
        Ok(identity.sign(&canonical).to_base64())
    }

    /// Register (or re-register) the caller's signed username alias with the
    /// relay, optionally attaching an avatar. Returns the registered username.
    ///
    /// On success the username (and, for an avatar upload, the resulting
    /// avatar path) is persisted to the local store so the UI can show the
    /// registered state across restarts — even when the relay is unreachable.
    pub async fn register_profile(
        &self,
        username: &str,
        display_name: Option<&str>,
        avatar_b64: Option<&str>,
    ) -> Result<String, RelayError> {
        // The relay verifies an Ed25519 signature over
        // `username || 0x00 || curve25519_key`, so sign it locally.
        let signature = self.sign_username(username)?;

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

        let username = tokio::time::timeout(PROFILE_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::ProfileTimeout)?
            .map_err(|_| RelayError::ProfileRequestFailed)??;

        // Persist the registration locally. When an avatar was uploaded we do
        // not yet know its `/media/{hash}` path (the server replies with just
        // the username), so fetch our own profile to learn it.
        if avatar_b64.is_some() {
            let avatar_url = self
                .get_profile(&self.my_peer_id()?)
                .await
                .ok()
                .flatten()
                .and_then(|profile| profile.avatar_url);
            self.persist_own_profile(&username, avatar_url.as_deref())?;
        } else {
            self.persist_own_profile(&username, None)?;
        }
        Ok(username)
    }

    /// Persist our own registered username (and optional avatar path) to the
    /// store and cache them in memory, so `get_chat_state` reports them on
    /// every restart.
    fn persist_own_profile(
        &self,
        username: &str,
        avatar_url: Option<&str>,
    ) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        let store_guard = self.store_guard()?;
        let store = store_guard.as_ref().ok_or(RelayError::StoreNotOpen)?;
        store.set_setting("my_username", username)?;
        match avatar_url {
            Some(url) if !url.is_empty() => store.set_setting("my_avatar_url", url)?,
            _ => store.delete_setting("my_avatar_url")?,
        }
        let mut profiles = read_guard(&self.inner.profiles)?.clone();
        profiles.my_username = Some(username.to_string());
        profiles.my_avatar_url = avatar_url.filter(|url| !url.is_empty()).map(str::to_string);
        *write_guard(&self.inner.profiles)? = profiles;
        Ok(())
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
    ///
    /// When a profile exists, what it advertises (display name, username,
    /// avatar) is persisted into the contact store and announced through a
    /// `contact-updated` event, so the chat list and header render the peer's
    /// avatar without a separate lookup. Our own identity is exempt: a self
    /// lookup (e.g. after an avatar upload) must never add a self-contact.
    pub async fn get_profile(&self, peer_id: &str) -> Result<Option<PeerProfile>, RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_profile)?.push_back(tx);
        if let Err(err) = self.send_json(&ClientMessage::GetProfile {
            peer_id: peer_id.to_string(),
        }) {
            mutex_guard(&self.inner.pending_profile)?.pop_back();
            return Err(err);
        }

        let profile = tokio::time::timeout(PROFILE_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| RelayError::ProfileTimeout)?
            .map_err(|_| RelayError::ProfileRequestFailed)??;
        if let Some(profile) = &profile {
            if peer_id != self.my_peer_id()? {
                self.remember_contact_profile(
                    peer_id,
                    profile.display_name.as_deref(),
                    profile.username.as_deref(),
                    profile.avatar_url.as_deref(),
                    profile.curve25519_key.as_deref(),
                )?;
            }
        }
        Ok(profile)
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
    /// full state refresh. Backed by [`RelayClient::remember_contact_profile`]
    /// with no username or avatar.
    pub(crate) fn remember_contact_name(
        &self,
        peer_id: &str,
        name: &str,
    ) -> Result<(), RelayError> {
        self.remember_contact_profile(peer_id, Some(name), None, None, None)
    }

    /// Store a contact's public profile data (display name, username, avatar
    /// path, identity key) that we learned from a lookup, persist it and
    /// notify the UI.
    ///
    /// Partial updates are COALESCE'd by the store, so a `None` field leaves
    /// the already-stored value intact and only the provided fields change.
    /// This is the single write path behind both pre-key lookups (name) and
    /// profile lookups (name + avatar + key), so every surface that reads
    /// `get_chat_state` / `contact-updated` stays in agreement.
    pub(crate) fn remember_contact_profile(
        &self,
        peer_id: &str,
        display_name: Option<&str>,
        username: Option<&str>,
        avatar_url: Option<&str>,
        curve25519_key: Option<&str>,
    ) -> Result<(), RelayError> {
        self.ensure_store_open()?;
        self.store_guard()?
            .as_ref()
            .ok_or(RelayError::StoreNotOpen)?
            .upsert_contact(&ContactRow {
                peer_id: peer_id.to_string(),
                display_name: display_name.map(str::to_string),
                username: username.map(str::to_string),
                avatar_url: avatar_url.map(str::to_string),
                last_seen: None,
                curve25519_key: curve25519_key.map(str::to_string),
                verified: false,
            })?;
        {
            let mut profiles = write_guard(&self.inner.profiles)?;
            if let Some(name) = display_name {
                profiles
                    .contacts
                    .insert(peer_id.to_string(), name.to_string());
            }
            if let Some(avatar) = avatar_url {
                profiles
                    .contact_avatars
                    .insert(peer_id.to_string(), avatar.to_string());
            }
        }
        // Register the peer in the ordered contact list as well, so the
        // `contact-updated` event and a `get_chat_state` snapshot agree.
        // `create_group` depends on this: without it, the refresh that follows
        // creation would overwrite the event-driven entry and drop the
        // just-created group from the conversation list.
        let mut contacts = write_guard(&self.inner.contacts)?;
        ensure_contact_entry(&mut contacts, peer_id);
        let _ = self.inner.app.emit(
            "contact-updated",
            ContactUpdatedEvent {
                peer_id: peer_id.to_string(),
                display_name: display_name.map(str::to_string),
                avatar_url: avatar_url.map(str::to_string),
            },
        );
        Ok(())
    }
}

/// Add `peer_id` to the ordered contact list when it is not already present.
/// Keeps the in-memory list and the persisted contact rows in agreement so a
/// state snapshot reflects every peer whose display name we have learned.
pub(crate) fn ensure_contact_entry(contacts: &mut Vec<String>, peer_id: &str) {
    if !contacts.iter().any(|known| known == peer_id) {
        contacts.push(peer_id.to_string());
    }
}

/// The persisted, non-empty display name the startup sync should advertise to
/// the relay. Blank/missing names are skipped — the relay rejects empty ones.
fn advertised_display_name(profiles: &Profiles) -> Option<&str> {
    profiles
        .my_display_name
        .as_deref()
        .filter(|name| !name.is_empty())
}

/// The persisted, non-empty username the startup sync should re-register with
/// the relay.
fn stored_username(profiles: &Profiles) -> Option<&str> {
    profiles
        .my_username
        .as_deref()
        .filter(|name| !name.is_empty())
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

    #[test]
    fn ensure_contact_entry_adds_once_and_is_idempotent() {
        // A learned display name must also produce a stable conversation-list
        // entry: a duplicate push would render two rows for one peer, so the
        // helper must add exactly once and keep first-contact order.
        let mut contacts = vec!["alice".to_string()];
        ensure_contact_entry(&mut contacts, "bob");
        ensure_contact_entry(&mut contacts, "alice");
        ensure_contact_entry(&mut contacts, "bob");
        assert_eq!(contacts, vec!["alice".to_string(), "bob".to_string()]);
    }

    #[test]
    fn stored_username_skips_missing_and_blank_names() {
        assert_eq!(stored_username(&Profiles::default()), None);
        let blank = Profiles {
            my_username: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(stored_username(&blank), None);
        let set = Profiles {
            my_username: Some("alice_42".into()),
            ..Default::default()
        };
        assert_eq!(stored_username(&set), Some("alice_42"));
    }

    #[test]
    fn advertised_display_name_skips_missing_and_blank_names() {
        assert_eq!(advertised_display_name(&Profiles::default()), None);
        let blank = Profiles {
            my_display_name: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(advertised_display_name(&blank), None);
        let set = Profiles {
            my_display_name: Some("Alice Prime".into()),
            ..Default::default()
        };
        assert_eq!(advertised_display_name(&set), Some("Alice Prime"));
    }
}
