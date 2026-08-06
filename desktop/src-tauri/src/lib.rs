use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, Listener, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_deep_link::DeepLinkExt;
use tokio::sync::Notify;

mod log_buffer;
mod relay;
mod store;
mod tray;

use log_buffer::{init_tracing, LogBuffer, LogEntry};
use relay::{
    ChatState, FriendRequests, GroupInfo, PeerProfile, PresenceInfo, ProfileSearchResult,
    RelayClient, Settings, SettingsPatch,
};
use tray::setup_tray;

/// Resolve the on-disk location of the persisted identity.
///
/// `WHISPER_IDENTITY_FILE` overrides the default so two Whisper instances can
/// run side by side on one machine (e.g. to test E2EE between two windows).
fn identity_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("WHISPER_IDENTITY_FILE") {
        return Ok(PathBuf::from(path));
    }
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("identity.json"))
}

/// Incoming `whisper://` deep links that arrived before the webview was ready
/// (app launched by clicking a link) or while it was booting. The frontend
/// drains them with `take_pending_deep_link` on startup; live links are also
/// pushed via the `deep-link` event.
struct PendingDeepLink(Mutex<Vec<String>>);

/// Drain and return every deep link received so far (app launch or second
/// instance). The frontend calls this once on startup so a link that opened
/// the app is never lost to a race with the webview boot.
#[tauri::command]
fn take_pending_deep_link(state: State<'_, PendingDeepLink>) -> Vec<String> {
    match state.0.lock() {
        Ok(mut pending) => std::mem::take(&mut *pending),
        Err(_) => Vec::new(),
    }
}

/// Forward one deep link to the webview and park it in the pending queue (so
/// a startup drain can still see it if the webview was not listening yet).
fn handle_deep_link(app: &tauri::AppHandle, url: String) {
    tracing::info!(url = %url, "deep link received");
    if let Some(state) = app.try_state::<PendingDeepLink>() {
        if let Ok(mut pending) = state.0.lock() {
            pending.push(url.clone());
        }
    }
    let _ = app.emit("deep-link", url);
}

/// Report whether a local identity already exists, including its peer ID.
#[tauri::command]
fn get_identity(app: tauri::AppHandle) -> Result<IdentityInfo, String> {
    let path = identity_path(&app)?;
    let json = match fs::read_to_string(&path) {
        Ok(json) => json,
        Err(_) => {
            return Ok(IdentityInfo {
                peer_id: String::new(),
                exists: false,
            });
        }
    };

    let identity = e2ee_core::Identity::from_json(&json).map_err(|e| e.to_string())?;
    Ok(IdentityInfo {
        peer_id: identity.peer_id(),
        exists: true,
    })
}

/// Generate a fresh identity, persist it to the app data directory and return
/// its peer ID. Existing identities are overwritten on purpose — the caller
/// only reaches this command when no identity is present. An optional
/// `display_name` (the onboarding "What should people call you?" answer) is
/// stored in the SQLCipher store so the first connect advertises it.
#[tauri::command]
fn generate_identity(
    app: tauri::AppHandle,
    display_name: Option<String>,
) -> Result<IdentityInfo, String> {
    let identity = e2ee_core::Identity::new();
    let peer_id = identity.peer_id();
    let json = identity.to_json().map_err(|e| e.to_string())?;

    let path = identity_path(&app)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&path, json).map_err(|e| e.to_string())?;

    if let Some(name) = display_name {
        let name = name.trim();
        if !name.is_empty() {
            // Best-effort: a database failure (e.g. a missing directory)
            // must never block identity creation. The name can be set later
            // through `set_display_name`.
            let _ = persist_onboarding_name(&app, &path, &peer_id, name);
        }
    }

    Ok(IdentityInfo {
        peer_id,
        exists: true,
    })
}

/// Store the onboarding display name in the identity-keyed database.
fn persist_onboarding_name(
    app: &tauri::AppHandle,
    identity_file: &PathBuf,
    peer_id: &str,
    name: &str,
) -> Result<(), String> {
    let json = fs::read_to_string(identity_file).map_err(|e| e.to_string())?;
    let key_hex = store::derive_db_key(&json);
    let db_path = relay::resolve_store_path(app, peer_id);
    let chat_store = store::ChatStore::open(&db_path, &key_hex).map_err(|e| e.to_string())?;
    chat_store
        .set_setting("my_display_name", name)
        .map_err(|e| e.to_string())
}

/// Delete the persisted identity file AND the identity-keyed local database
/// (messages, contacts, settings), returning to the onboarding state.
/// Missing files are treated as success so the command is idempotent.
///
/// Without the database wipe, a reset would leave the old `whisper-{peer}.db`
/// orphaned in the app data folder forever — encrypted with a key derived from
/// a now-deleted identity, so it can never be opened again, only reclaimed.
#[tauri::command]
fn delete_identity(app: tauri::AppHandle) -> Result<(), String> {
    let path = identity_path(&app)?;
    // Derive the peer ID before removing the identity file so the matching
    // database (named after the peer ID) can be removed too.
    if let Ok(json) = fs::read_to_string(&path) {
        if let Ok(identity) = e2ee_core::Identity::from_json(&json) {
            let db_path = relay::resolve_store_path(&app, &identity.peer_id());
            for variant in ["", "-wal", "-shm"] {
                let candidate = db_path.with_file_name(format!(
                    "{}{}",
                    db_path
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default(),
                    variant
                ));
                match fs::remove_file(&candidate) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
        }
    }
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Identity information reported to the UI.
#[derive(Serialize)]
struct IdentityInfo {
    peer_id: String,
    exists: bool,
}

/// Connect to the relay using the persisted identity and start the pumps.
#[tauri::command]
async fn connect_relay(state: State<'_, RelayClient>) -> Result<(), String> {
    state.connect().await.map_err(|e| e.to_string())
}

/// Generate and publish a fresh batch of one-time pre-keys.
#[tauri::command]
async fn publish_prekeys(state: State<'_, RelayClient>) -> Result<(), String> {
    state.publish_prekeys().await.map_err(|e| e.to_string())
}

/// Register (or re-register) the signed username alias, optionally with an
/// avatar (base64 image, ≤2 MB). Returns the registered username.
#[tauri::command]
async fn register_profile(
    state: State<'_, RelayClient>,
    username: String,
    display_name: Option<String>,
    avatar: Option<String>,
) -> Result<String, String> {
    state
        .register_profile(&username, display_name.as_deref(), avatar.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Prefix-search registered usernames and peer IDs.
#[tauri::command]
async fn search_users(
    state: State<'_, RelayClient>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<ProfileSearchResult>, String> {
    state
        .search_users(&query, limit)
        .await
        .map_err(|e| e.to_string())
}

/// Fetch one peer's public profile; `null` when they have none registered.
#[tauri::command]
async fn get_profile(
    state: State<'_, RelayClient>,
    peer_id: String,
) -> Result<Option<PeerProfile>, String> {
    state.get_profile(&peer_id).await.map_err(|e| e.to_string())
}

/// Re-register the profile with a new avatar image (base64, ≤2 MB).
#[tauri::command]
async fn set_avatar(
    state: State<'_, RelayClient>,
    username: String,
    avatar: String,
) -> Result<(), String> {
    state
        .set_avatar(&username, &avatar)
        .await
        .map_err(|e| e.to_string())
}

/// Establish an encrypted session with a peer and send the first message.
#[tauri::command]
async fn start_chat(state: State<'_, RelayClient>, peer_id: String) -> Result<(), String> {
    state.start_chat(&peer_id).await.map_err(|e| e.to_string())
}

/// Encrypt and send a message over the established session. `client_id` is a
/// UI-generated id that travels back inside the emitted `chat-message` event so
/// the UI can deduplicate optimistic insertions. `quote` makes the message a
/// quoted reply (the snapshot travels inside the encrypted payload).
#[tauri::command]
async fn send_message(
    state: State<'_, RelayClient>,
    peer_id: String,
    text: String,
    client_id: String,
    quote: Option<e2ee_core::Quote>,
) -> Result<(), String> {
    state
        .send_message(&peer_id, &text, &client_id, quote)
        .await
        .map_err(|e| e.to_string())
}

/// React to a message with an emoji. `active` is the sender's freshly computed
/// absolute state (true = react, false = unreact) and travels inside the
/// encrypted payload, so no relay or server changes are involved.
#[tauri::command]
async fn send_reaction(
    state: State<'_, RelayClient>,
    peer_id: String,
    message_id: String,
    emoji: String,
    active: bool,
) -> Result<(), String> {
    state
        .send_reaction(&peer_id, &message_id, &emoji, active)
        .await
        .map_err(|e| e.to_string())
}

/// Build a `whisper://invite` link for our own identity (with profile hints).
#[tauri::command]
fn get_invite_link(state: State<'_, RelayClient>) -> Result<String, String> {
    state.get_invite_link().map_err(|e| e.to_string())
}

/// Compute the safety number shared with `peer_id` plus our verification
/// state. Fails until the peer's identity key has been learned.
#[tauri::command]
fn get_safety_number(
    state: State<'_, RelayClient>,
    peer_id: String,
) -> Result<relay::SafetyNumberInfo, String> {
    state.get_safety_number(&peer_id).map_err(|e| e.to_string())
}

/// Set (or clear) the locally-stored verified flag for a contact.
#[tauri::command]
fn mark_contact_verified(
    state: State<'_, RelayClient>,
    peer_id: String,
    verified: bool,
) -> Result<(), String> {
    state
        .set_contact_verified(&peer_id, verified)
        .map_err(|e| e.to_string())
}

/// Invite `peer_id` to `group_id` (owner/admin only). The invitee accepts or
/// declines; they join the roster only on accept.
#[tauri::command]
async fn send_group_invite(
    state: State<'_, RelayClient>,
    group_id: String,
    peer_id: String,
) -> Result<(), String> {
    state
        .send_group_invite(&group_id, &peer_id)
        .await
        .map_err(|e| e.to_string())
}

/// Accept a pending invite to `group_id`.
#[tauri::command]
async fn accept_group_invite(
    state: State<'_, RelayClient>,
    group_id: String,
) -> Result<(), String> {
    state
        .accept_group_invite(&group_id)
        .await
        .map_err(|e| e.to_string())
}

/// Decline a pending invite to `group_id`.
#[tauri::command]
async fn decline_group_invite(
    state: State<'_, RelayClient>,
    group_id: String,
) -> Result<(), String> {
    state
        .decline_group_invite(&group_id)
        .await
        .map_err(|e| e.to_string())
}

/// Fetch the pending group invites for this identity.
#[tauri::command]
async fn get_group_invites(
    state: State<'_, RelayClient>,
) -> Result<Vec<relay::GroupInviteInfo>, String> {
    state.get_group_invites().await.map_err(|e| e.to_string())
}

/// Get (or create) the group's shareable join link. Any member may ask.
#[tauri::command]
async fn get_group_join_link(
    state: State<'_, RelayClient>,
    group_id: String,
) -> Result<String, String> {
    state
        .get_group_join_link(&group_id)
        .await
        .map_err(|e| e.to_string())
}

/// Join a group via its shareable join link (group id + secret token).
#[tauri::command]
async fn join_group_by_link(
    state: State<'_, RelayClient>,
    group_id: String,
    token: String,
) -> Result<(), String> {
    state
        .join_group(&group_id, &token)
        .await
        .map_err(|e| e.to_string())
}

/// Rename a group (owner/admin only).
#[tauri::command]
async fn rename_group(
    state: State<'_, RelayClient>,
    group_id: String,
    name: String,
) -> Result<(), String> {
    state
        .rename_group(&group_id, &name)
        .await
        .map_err(|e| e.to_string())
}

/// Snapshot of identity, connection, contacts and messages for the UI.
#[tauri::command]
async fn get_chat_state(state: State<'_, RelayClient>) -> Result<ChatState, String> {
    state.get_chat_state().map_err(|e| e.to_string())
}

/// Close the relay connection (used when resetting the identity).
#[tauri::command]
async fn disconnect_relay(state: State<'_, RelayClient>) -> Result<(), String> {
    state.disconnect().map_err(|e| e.to_string())
}

/// Close the connection and wipe all in-memory chat state. Called when the
/// identity is reset so stale data never survives into a fresh identity.
#[tauri::command]
async fn reset_relay(state: State<'_, RelayClient>) -> Result<(), String> {
    state.reset().map_err(|e| e.to_string())
}

/// Return the currently persisted relay URL and theme.
#[tauri::command]
async fn get_settings(state: State<'_, RelayClient>) -> Result<Settings, String> {
    state.get_settings().map_err(|e| e.to_string())
}

/// Persist a new relay URL and, if the connection is open to a different
/// endpoint, drop it so the UI can reconnect to the new address.
#[tauri::command]
async fn set_relay_url(state: State<'_, RelayClient>, url: String) -> Result<(), String> {
    state.set_relay_url(&url).map_err(|e| e.to_string())
}

/// Persist a new UI theme preference.
#[tauri::command]
async fn set_theme(state: State<'_, RelayClient>, theme: String) -> Result<(), String> {
    state.set_theme(&theme).map_err(|e| e.to_string())
}

/// Toggle whether our online status and last-seen are visible to other peers.
/// Persisted locally and pushed to the relay (best-effort) so it takes effect
/// immediately.
#[tauri::command]
async fn set_privacy(state: State<'_, RelayClient>, presence_visible: bool) -> Result<(), String> {
    state
        .set_privacy(presence_visible)
        .map_err(|e| e.to_string())
}

/// Apply a partial boolean-preferences update (read receipts, typing
/// indicator, notifications) and persist it.
#[tauri::command]
async fn update_settings(
    state: State<'_, RelayClient>,
    patch: SettingsPatch,
) -> Result<(), String> {
    state.update_settings(&patch).map_err(|e| e.to_string())
}

/// Remove the accepted contact relationship with `peer_id` on both sides. The
/// relay broadcasts a `contact_removed` push to both peers; the local contact
/// row, history and presence are dropped immediately.
#[tauri::command]
async fn remove_contact(state: State<'_, RelayClient>, peer_id: String) -> Result<(), String> {
    state
        .remove_contact(&peer_id)
        .await
        .map_err(|e| e.to_string())
}

/// Send a friend request to `peer_id`. The peer becomes an accepted contact
/// once they accept. Rejects locally with `cannot_add_self` when `peer_id` is
/// our own identity; other failures surface as relay error codes.
#[tauri::command]
async fn send_friend_request(state: State<'_, RelayClient>, peer_id: String) -> Result<(), String> {
    state
        .send_friend_request(&peer_id)
        .await
        .map_err(|e| e.to_string())
}

/// Accept a pending incoming friend request from `peer_id`. Both sides become
/// accepted contacts and the requester receives a `friend_request_accepted`
/// push.
#[tauri::command]
async fn accept_friend_request(
    state: State<'_, RelayClient>,
    peer_id: String,
) -> Result<(), String> {
    state
        .accept_friend_request(&peer_id)
        .await
        .map_err(|e| e.to_string())
}

/// Decline a pending incoming friend request from `peer_id`.
#[tauri::command]
async fn decline_friend_request(
    state: State<'_, RelayClient>,
    peer_id: String,
) -> Result<(), String> {
    state
        .decline_friend_request(&peer_id)
        .await
        .map_err(|e| e.to_string())
}

/// Fetch the pending friend-request snapshot (incoming + outgoing).
#[tauri::command]
async fn get_friend_requests(state: State<'_, RelayClient>) -> Result<FriendRequests, String> {
    state.get_friend_requests().await.map_err(|e| e.to_string())
}

/// Delete one message locally ("delete for me"): the decrypted history in
/// memory and its row in the encrypted store. The peer's copy and any
/// relay-queued envelopes are untouched.
#[tauri::command]
async fn delete_message(
    state: State<'_, RelayClient>,
    peer_id: String,
    message_id: String,
) -> Result<(), String> {
    state
        .delete_message(&peer_id, &message_id)
        .map_err(|e| e.to_string())
}

/// Wipe the entire message history on this device (every conversation). The
/// contacts, sessions, groups and settings are kept — only the decrypted
/// message history is cleared, in memory and in the encrypted store.
#[tauri::command]
async fn clear_chat_history(state: State<'_, RelayClient>) -> Result<(), String> {
    state.clear_chat_history().map_err(|e| e.to_string())
}

/// Open a native save dialog and copy the persisted identity file to the
/// chosen location, so the user can back it up off-device. Resolves with the
/// destination path on success. Fails when no identity exists yet or when the
/// dialog is cancelled.
#[tauri::command]
async fn export_identity(app: tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;
    let source = identity_path(&app)?;
    if !source.exists() {
        return Err("no local identity to back up".to_string());
    }
    let target = app
        .dialog()
        .file()
        .add_filter("Whisper identity", &["json"])
        .set_file_name("whisper-identity.json")
        .blocking_save_file()
        .ok_or_else(|| "identity export cancelled".to_string())?;
    let target = target.into_path().map_err(|e| e.to_string())?;
    fs::copy(&source, &target).map_err(|e| e.to_string())?;
    Ok(target.display().to_string())
}

/// Open a native pick dialog, validate the selected file and restore it over
/// the persisted identity. Two file shapes are accepted:
///   - a bare Whisper identity JSON (only the identity is restored), or
///   - a full `whisper-backup` package (identity AND the encrypted database —
///     messages, contacts, sessions, settings — are restored, exactly like
///     "Restore everything").
/// The frontend then drops the cached identity (`reload_identity`) and reloads
/// the webview so the restored identity takes effect — a full app restart is
/// not required.
#[tauri::command]
async fn import_identity(
    app: tauri::AppHandle,
    state: State<'_, RelayClient>,
) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .add_filter("Whisper identity or backup", &["json"])
        .blocking_pick_file()
        .ok_or_else(|| "identity import cancelled".to_string())?;
    let source = picked.into_path().map_err(|e| e.to_string())?;
    let json = fs::read_to_string(&source).map_err(|e| e.to_string())?;

    // A full backup package restores identity + database (contacts, messages,
    // settings). A bare identity file restores only the identity.
    if let Ok(package) = serde_json::from_str::<serde_json::Value>(&json) {
        if package.get("kind").and_then(|k| k.as_str()) == Some("whisper-backup") {
            return restore_package(&app, &state, package);
        }
    }

    // Bare identity file: validate before touching the file on disk, so a
    // corrupt or foreign file can never brick the app.
    e2ee_core::Identity::from_json(&json).map_err(|e| e.to_string())?;
    let target = identity_path(&app)?;
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&target, json).map_err(|e| e.to_string())?;
    Ok(target.display().to_string())
}

/// Build the full-profile backup package (identity + encrypted database) as a
/// JSON value. Shared by the manual "Backup everything" dialog and the
/// automatic backup scheduler.
fn build_backup_package(app: &tauri::AppHandle) -> Result<serde_json::Value, String> {
    use base64::Engine;

    let identity_file = identity_path(app)?;
    if !identity_file.exists() {
        return Err("no local identity to back up".to_string());
    }
    let identity_json = fs::read_to_string(&identity_file).map_err(|e| e.to_string())?;
    let identity = e2ee_core::Identity::from_json(&identity_json).map_err(|e| e.to_string())?;

    let db_path = relay::resolve_store_path(app, &identity.peer_id());
    let database_b64 = if db_path.exists() {
        let bytes = fs::read(&db_path).map_err(|e| e.to_string())?;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    } else {
        String::new()
    };

    Ok(serde_json::json!({
        "kind": "whisper-backup",
        "version": 1,
        "identity": identity_json,
        "database_b64": database_b64,
    }))
}

/// Export EVERYTHING — identity + the encrypted local database (history,
/// sessions, contacts, settings) — as a single JSON backup file. Copy this to
/// a new machine to move the whole Whisper profile. Resolves with the
/// destination path on success.
#[tauri::command]
async fn export_everything(app: tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;

    let package = build_backup_package(&app)?;

    let target = app
        .dialog()
        .file()
        .add_filter("Whisper backup", &["json"])
        .set_file_name("whisper-backup.json")
        .blocking_save_file()
        .ok_or_else(|| "backup export cancelled".to_string())?;
    let target = target.into_path().map_err(|e| e.to_string())?;
    let pretty = serde_json::to_string_pretty(&package).map_err(|e| e.to_string())?;
    fs::write(&target, pretty).map_err(|e| e.to_string())?;
    Ok(target.display().to_string())
}

/// Open a native folder picker so the user can choose the automatic-backup
/// destination (typically a cloud-synced folder). Resolves with the chosen
/// path, or an error when cancelled.
#[tauri::command]
async fn pick_autobackup_dir(app: tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .blocking_pick_folder()
        .ok_or_else(|| "folder pick cancelled".to_string())?;
    let path = picked.into_path().map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// Write a full-profile backup into `dir` as `whisper-backup-<date>.json`,
/// pruning older backups beyond the configured keep count. Used by the
/// scheduler; `run_autobackup_now` exposes it to the UI.
fn write_autobackup(app: &tauri::AppHandle, dir: &str, keep: usize) -> Result<PathBuf, String> {
    let dir = PathBuf::from(dir);
    if !dir.is_dir() {
        return Err("automatic backup folder does not exist".to_string());
    }
    let package = build_backup_package(app)?;
    let pretty = serde_json::to_string_pretty(&package).map_err(|e| e.to_string())?;

    let now = chrono_now_date();
    let filename = format!("whisper-backup-{now}.json");
    let target = dir.join(&filename);
    fs::write(&target, pretty).map_err(|e| e.to_string())?;

    // Prune old backups, keeping the newest `keep` (including the one just
    // written).
    if let Ok(entries) = fs::read_dir(&dir) {
        let mut backups: Vec<(std::time::SystemTime, PathBuf)> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("whisper-backup-")
                    && entry.file_name().to_string_lossy().ends_with(".json")
            })
            .filter_map(|entry| {
                let modified = entry.metadata().ok().and_then(|m| m.modified().ok())?;
                Some((modified, entry.path()))
            })
            .collect();
        backups.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        for (_, path) in backups.into_iter().skip(keep) {
            let _ = fs::remove_file(path);
        }
    }

    Ok(target)
}

/// The current UTC date as `YYYY-MM-DD` (no external chrono dependency).
fn chrono_now_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    // Days since 1970-01-01 → civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Run an automatic backup right now, using the persisted settings. Errors
/// are returned so the UI can surface them from a manual "Back up now" click.
#[tauri::command]
async fn run_autobackup_now(app: tauri::AppHandle) -> Result<String, String> {
    let dir = app
        .state::<RelayClient>()
        .get_settings()
        .map_err(|e| e.to_string())?
        .autobackup_dir
        .ok_or_else(|| "no automatic backup folder configured".to_string())?;
    let keep = app
        .state::<RelayClient>()
        .get_settings()
        .map_err(|e| e.to_string())?
        .autobackup_keep;
    let target = write_autobackup(&app, &dir, keep)?;
    Ok(target.display().to_string())
}

/// Import a Whisper backup created by `export_everything`: restores the
/// identity file AND the encrypted local database. The in-memory client state
/// is reset first so the database file is not locked; the frontend must then
/// reload the webview for the restored profile to take effect.
#[tauri::command]
async fn import_everything(
    app: tauri::AppHandle,
    state: State<'_, RelayClient>,
) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;

    let picked = app
        .dialog()
        .file()
        .add_filter("Whisper backup", &["json"])
        .blocking_pick_file()
        .ok_or_else(|| "backup import cancelled".to_string())?;
    let source = picked.into_path().map_err(|e| e.to_string())?;
    let text = fs::read_to_string(&source).map_err(|e| e.to_string())?;
    let package: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;

    if package.get("kind").and_then(|k| k.as_str()) != Some("whisper-backup") {
        return Err("not a Whisper backup file".to_string());
    }
    restore_package(&app, &state, package)
}

/// Shared restore logic for a validated `whisper-backup` package: write the
/// identity file and the encrypted database, after resetting the client so
/// neither file is locked. Returns the restored peer ID.
fn restore_package(
    app: &tauri::AppHandle,
    state: &RelayClient,
    package: serde_json::Value,
) -> Result<String, String> {
    use base64::Engine;

    let identity_json = package
        .get("identity")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "backup is missing the identity".to_string())?
        .to_string();
    // Validate before touching anything on disk.
    let identity = e2ee_core::Identity::from_json(&identity_json).map_err(|e| e.to_string())?;

    // Reset the client: closes the store (releasing the database file) and
    // wipes in-memory state so the restored files take effect cleanly.
    state.reset().map_err(|e| e.to_string())?;

    let identity_file = identity_path(app)?;
    if let Some(dir) = identity_file.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&identity_file, &identity_json).map_err(|e| e.to_string())?;

    let database_b64 = package
        .get("database_b64")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !database_b64.is_empty() {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(database_b64)
            .map_err(|e| e.to_string())?;
        let db_path = relay::resolve_store_path(app, &identity.peer_id());
        if let Some(dir) = db_path.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        fs::write(&db_path, bytes).map_err(|e| e.to_string())?;
    }

    Ok(identity.peer_id())
}

/// Drop the cached identity so the next `connect` reloads it from disk. Called
/// after a successful `import_identity`, before the webview reloads.
#[tauri::command]
async fn reload_identity(state: State<'_, RelayClient>) -> Result<(), String> {
    state.reload_identity().map_err(|e| e.to_string())
}

/// Enable or disable launching Whisper at system startup (the OS-level
/// autostart registration). The preference itself is persisted separately
/// through `update_settings`.
#[tauri::command]
async fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())
    } else {
        manager.disable().map_err(|e| e.to_string())
    }
}

/// Return the most recent client log lines from the in-process ring buffer.
/// `limit` caps the number of lines; the newest lines are returned.
#[tauri::command]
fn get_client_logs(state: State<'_, LogBuffer>, limit: Option<usize>) -> Vec<LogEntry> {
    state.snapshot(limit)
}

/// Append a log line forwarded from the webview (e.g. an uncaught JS error),
/// so the Logs tab shows frontend failures alongside the Rust logs.
#[tauri::command]
fn append_client_log(state: State<'_, LogBuffer>, level: String, message: String) {
    state.write_line(&level, "webview", &message);
}

/// Open the daily client log folder (`<app-data>/logs`) in the OS file
/// manager, so users can grab the log file for a bug report.
#[tauri::command]
fn open_logs_folder(app: tauri::AppHandle) -> Result<(), String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("logs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let opener = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(&dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Persist our own public display name and announce it to the relay so other
/// peers can show it when they look us up.
#[tauri::command]
async fn set_display_name(state: State<'_, RelayClient>, name: String) -> Result<(), String> {
    state.set_display_name(&name).map_err(|e| e.to_string())
}

/// Send an end-to-end read receipt for a conversation that is visible on
/// screen: a 1:1 receipt, or a group read receipt for `message_id` (the
/// newest visible incoming message in a group).
#[tauri::command]
async fn send_read_receipt(
    state: State<'_, RelayClient>,
    peer_id: String,
    message_id: Option<String>,
) -> Result<(), String> {
    state
        .mark_read(&peer_id, message_id)
        .map_err(|e| e.to_string())
}

/// Send an end-to-end typing indicator to a peer (encrypted in the session).
#[tauri::command]
async fn send_typing(
    state: State<'_, RelayClient>,
    peer_id: String,
    is_typing: bool,
) -> Result<(), String> {
    state
        .send_typing(&peer_id, is_typing)
        .await
        .map_err(|e| e.to_string())
}

/// Fetch a peer's current presence (online status + last-seen timestamp).
#[tauri::command]
async fn get_presence(
    state: State<'_, RelayClient>,
    peer_id: String,
) -> Result<PresenceInfo, String> {
    state
        .get_presence(&peer_id)
        .await
        .map_err(|e| e.to_string())
}

/// Subscribe to presence pushes for a peer (re-sent on every connect).
#[tauri::command]
async fn watch_presence(state: State<'_, RelayClient>, peer_id: String) -> Result<(), String> {
    state.watch_presence(&peer_id).map_err(|e| e.to_string())
}

/// Create a group: register it on the relay, add `member_ids` to its roster,
/// build the Megolm outbound session and share its session key to every
/// member over the existing 1:1 Double Ratchet channels. Returns the
/// relay-assigned group ID.
#[tauri::command]
async fn create_group(
    state: State<'_, RelayClient>,
    name: String,
    member_ids: Vec<String>,
) -> Result<String, String> {
    state
        .create_group(&name, member_ids)
        .await
        .map_err(|e| e.to_string())
}

/// Fetch a group's public metadata and member roster (with roles).
#[tauri::command]
async fn get_group_info(
    state: State<'_, RelayClient>,
    group_id: String,
) -> Result<GroupInfo, String> {
    state
        .get_group_info(&group_id)
        .await
        .map_err(|e| e.to_string())
}

/// Promote a member to group admin (owner or admin only).
#[tauri::command]
async fn promote_member(
    state: State<'_, RelayClient>,
    group_id: String,
    peer_id: String,
) -> Result<(), String> {
    state
        .promote_member(&group_id, &peer_id)
        .await
        .map_err(|e| e.to_string())
}

/// Demote an admin back to a regular member (owner only).
#[tauri::command]
async fn demote_member(
    state: State<'_, RelayClient>,
    group_id: String,
    peer_id: String,
) -> Result<(), String> {
    state
        .demote_member(&group_id, &peer_id)
        .await
        .map_err(|e| e.to_string())
}

/// Remove a member from a group (owner only).
#[tauri::command]
async fn remove_member(
    state: State<'_, RelayClient>,
    group_id: String,
    peer_id: String,
) -> Result<(), String> {
    state
        .remove_member(&group_id, &peer_id)
        .await
        .map_err(|e| e.to_string())
}

/// Transfer group ownership to `peer_id` (owner only). The old owner becomes
/// an admin; `peer_id` takes over the owner role.
#[tauri::command]
async fn transfer_ownership(
    state: State<'_, RelayClient>,
    group_id: String,
    peer_id: String,
) -> Result<(), String> {
    state
        .transfer_ownership(&group_id, &peer_id)
        .await
        .map_err(|e| e.to_string())
}

/// Remove the caller from a group's roster.
#[tauri::command]
async fn leave_group(state: State<'_, RelayClient>, group_id: String) -> Result<(), String> {
    state
        .leave_group(&group_id)
        .await
        .map_err(|e| e.to_string())
}

/// Add a peer to a group's roster after creation (owner or admin only). On
/// success every existing member shares its own Megolm session key to the new
/// member over a 1:1 encrypted channel.
#[tauri::command]
async fn add_group_member(
    state: State<'_, RelayClient>,
    group_id: String,
    peer_id: String,
) -> Result<(), String> {
    state
        .add_group_member(&group_id, &peer_id)
        .await
        .map_err(|e| e.to_string())
}

/// Set a group's avatar image (base64, ≤2 MB; raw base64 without the
/// `data:image/...;base64,` prefix). The relay stores the blob content
/// addressed and exposes it as `avatar_url` in the group metadata. Owner or
/// admin only.
#[tauri::command]
async fn set_group_avatar(
    state: State<'_, RelayClient>,
    group_id: String,
    avatar: String,
) -> Result<(), String> {
    state
        .set_group_avatar(&group_id, &avatar)
        .await
        .map_err(|e| e.to_string())
}

/// Suppresses the WebView2 default right-click menu (Reload/Inspect/Copy/...)
/// on every window. Tauri 2.11 has no `Webview::on_menu` / `prevent_default`
/// API — `on_menu_event` only reports native app-menu clicks — so the supported
/// way to kill the browser menu is a DOM-level `contextmenu` preventDefault:
/// WebView2 honors it and skips its built-in menu. The global `on_page_load`
/// hook covers BOTH config-defined windows (splash + main) plus any window
/// created later.
const SUPPRESS_CONTEXT_MENU_SCRIPT: &str =
    "window.addEventListener('contextmenu',(e)=>e.preventDefault(),true);";

/// Splash screen handoff. The main window is created hidden so the splash
/// window is the first thing the user sees. The frontend emits a `splash-done`
/// event once its view is ready; if that never arrives (e.g. the webview
/// failed to boot) a short timeout still opens the main window, so the app
/// never dead-ends on an empty splash.
fn setup_splash_screen(app: &mut tauri::App) -> tauri::Result<()> {
    if let Some(main) = app.get_webview_window("main") {
        main.hide()?;
    }

    let splash_done = Arc::new(Notify::new());

    let notify = splash_done.clone();
    app.listen("splash-done", move |_event| {
        notify.notify_one();
    });

    let app_handle = app.handle().clone();
    let splash_done = splash_done.clone();
    tauri::async_runtime::spawn(async move {
        // Wait for the frontend signal, but never longer than the fallback.
        tokio::time::timeout(Duration::from_millis(2500), splash_done.notified())
            .await
            .ok();

        show_main_window(&app_handle);
    });

    Ok(())
}

/// Close the splash window (if it is still around) and bring the main window
/// to the foreground. Idempotent, so both the event path and the timeout path
/// can call it without side effects.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(splash) = app.get_webview_window("splash") {
        let _ = splash.close();
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        // Native desktop notifications (the HTML5 Notification API is not
        // reliable inside the Tauri webview, so the plugin talks to the OS
        // directly).
        .plugin(tauri_plugin_notification::init())
        // Native save/pick dialogs for the identity backup/restore feature
        // (called from Rust commands, not the webview).
        .plugin(tauri_plugin_dialog::init())
        // OS-level "launch at startup" registration, wired to the autostart
        // setting in the General tab.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        // Deep links: registers the `whisper://` scheme (Windows: HKCU
        // Software\Classes\whisper -> our exe "%1") so clicking an invite
        // link in a browser opens Whisper with the invite pre-loaded.
        .plugin(tauri_plugin_deep_link::init())
        // Re-injected on every page load so a navigation (or a window that was
        // still booting when the setup hook ran) can never bring the browser
        // menu back.
        .on_page_load(|webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                let _ = webview.eval(SUPPRESS_CONTEXT_MENU_SCRIPT);
            }
        })
        // "Minimize to tray on close": when the setting is on, closing the main
        // window hides it (the app keeps running in the tray) instead of
        // quitting. When off, the close is allowed to proceed normally. Only
        // the main window is affected — the splash handoff calls `close()` on
        // the splash window and must never be swallowed.
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let minimize_to_tray = window
                    .app_handle()
                    .try_state::<RelayClient>()
                    .and_then(|client| client.get_settings().ok())
                    .map(|settings| settings.minimize_to_tray)
                    .unwrap_or(false);
                if minimize_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            // Client log ring buffer backing the Logs settings tab. Captures
            // every tracing event the app emits plus webview errors forwarded
            // through `append_client_log`.
            let log_buffer = LogBuffer::new();
            // Daily log files in the app-data `logs/` folder so users can
            // share complete error logs in bug reports.
            let log_dir = app.path().app_data_dir().ok().map(|dir| dir.join("logs"));
            init_tracing(&log_buffer, log_dir);
            app.manage(log_buffer);

            app.manage(PendingDeepLink(Mutex::new(Vec::new())));

            app.manage(RelayClient::new(app.handle().clone()));

            // Register the whisper:// scheme (best-effort: in installed
            // bundles the installer does it; this covers dev/portable runs).
            // EVERY instance registers its own exe: in dev, the primary
            // instance (tauri:dev, vite on :1420) and the second instance
            // (tauri:dev:second, vite on :1421) each point the registry at
            // their own binary, so a clicked link always launches the exe
            // whose dev server is actually running. The most recently started
            // instance wins the registry entry.
            let _ = app.deep_link().register("whisper");
            // The app was launched by clicking a whisper:// link: the plugin
            // parsed the CLI argument into `get_current()`.
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                for url in urls {
                    handle_deep_link(app.handle(), url.to_string());
                }
            }
            // The app is already running and another instance handed us a URL
            // (single-instance plugin) or the OS opened one while running.
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    handle_deep_link(&handle, url.to_string());
                }
            });

            setup_tray(app.handle())?;

            // Automatic full-profile backups: if enabled, run one shortly
            // after startup and then every 24h. Best-effort — a missing
            // folder or a transient failure just logs and waits for the next
            // tick, never crashes the app.
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    loop {
                        let enabled = handle
                            .state::<RelayClient>()
                            .get_settings()
                            .map(|s| s.autobackup_enabled)
                            .unwrap_or(false);
                        if enabled {
                            let settings = handle
                                .state::<RelayClient>()
                                .get_settings()
                                .unwrap_or_default();
                            if let Some(dir) = &settings.autobackup_dir {
                                match write_autobackup(&handle, dir, settings.autobackup_keep) {
                                    Ok(path) => tracing::info!(
                                        path = %path.display(),
                                        "automatic backup written"
                                    ),
                                    Err(err) => tracing::warn!(
                                        error = %err,
                                        "automatic backup failed"
                                    ),
                                }
                            }
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(86_400)).await;
                    }
                });
            }

            setup_splash_screen(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_identity,
            generate_identity,
            delete_identity,
            take_pending_deep_link,
            connect_relay,
            publish_prekeys,
            start_chat,
            send_message,
            send_reaction,
            get_invite_link,
            get_safety_number,
            mark_contact_verified,
            send_group_invite,
            accept_group_invite,
            decline_group_invite,
            get_group_invites,
            get_group_join_link,
            join_group_by_link,
            rename_group,
            get_chat_state,
            disconnect_relay,
            reset_relay,
            reload_identity,
            get_settings,
            set_relay_url,
            set_theme,
            set_privacy,
            update_settings,
            remove_contact,
            send_friend_request,
            accept_friend_request,
            decline_friend_request,
            get_friend_requests,
            delete_message,
            clear_chat_history,
            set_display_name,
            send_typing,
            send_read_receipt,
            get_presence,
            register_profile,
            search_users,
            get_profile,
            set_avatar,
            watch_presence,
            create_group,
            get_group_info,
            add_group_member,
            promote_member,
            demote_member,
            remove_member,
            transfer_ownership,
            leave_group,
            set_group_avatar,
            get_client_logs,
            append_client_log,
            open_logs_folder,
            export_identity,
            import_identity,
            export_everything,
            import_everything,
            pick_autobackup_dir,
            run_autobackup_now,
            set_autostart
        ]);

    // Single-instance only for the primary app instance: opening a
    // `whisper://` link (or launching the app again) while it is already
    // running forwards the URL to the first instance instead of spawning a
    // second window. The second dev instance (`WHISPER_IDENTITY_FILE` set by
    // `tauri:dev:second`) must be able to run SIDE BY SIDE for two-window
    // E2EE tests — the single-instance mutex is keyed by the app identifier,
    // so it would otherwise silently swallow the second window.
    let builder = if std::env::var("WHISPER_IDENTITY_FILE").is_err() {
        builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            tracing::info!(?args, "second instance detected, forwarding");
            if let Some(url) = args.iter().find(|arg| arg.starts_with("whisper://")) {
                handle_deep_link(app, url.clone());
            }
        }))
    } else {
        builder
    };

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
