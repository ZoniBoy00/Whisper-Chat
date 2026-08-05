use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{Listener, Manager, State};
use tokio::sync::Notify;

mod relay;
mod store;

use relay::{
    ChatState, GroupInfo, PeerProfile, PresenceInfo, ProfileSearchResult, RelayClient, Settings,
    SettingsPatch,
};

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

/// Delete the persisted identity file, returning it to the onboarding state.
/// Missing files are treated as success so the command is idempotent.
#[tauri::command]
fn delete_identity(app: tauri::AppHandle) -> Result<(), String> {
    let path = identity_path(&app)?;
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
/// the UI can deduplicate optimistic insertions.
#[tauri::command]
async fn send_message(
    state: State<'_, RelayClient>,
    peer_id: String,
    text: String,
    client_id: String,
) -> Result<(), String> {
    state
        .send_message(&peer_id, &text, &client_id)
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

/// Remove a contact and its message history locally (client-side only).
#[tauri::command]
async fn remove_contact(state: State<'_, RelayClient>, peer_id: String) -> Result<(), String> {
    state.remove_contact(&peer_id).map_err(|e| e.to_string())
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

/// Persist our own public display name and announce it to the relay so other
/// peers can show it when they look us up.
#[tauri::command]
async fn set_display_name(state: State<'_, RelayClient>, name: String) -> Result<(), String> {
    state.set_display_name(&name).map_err(|e| e.to_string())
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

/// Remove the caller from a group's roster.
#[tauri::command]
async fn leave_group(state: State<'_, RelayClient>, group_id: String) -> Result<(), String> {
    state
        .leave_group(&group_id)
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
    tauri::Builder::default()
        // Re-injected on every page load so a navigation (or a window that was
        // still booting when the setup hook ran) can never bring the browser
        // menu back.
        .on_page_load(|webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                let _ = webview.eval(SUPPRESS_CONTEXT_MENU_SCRIPT);
            }
        })
        .setup(|app| {
            app.manage(RelayClient::new(app.handle().clone()));

            setup_splash_screen(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_identity,
            generate_identity,
            delete_identity,
            connect_relay,
            publish_prekeys,
            start_chat,
            send_message,
            get_chat_state,
            disconnect_relay,
            reset_relay,
            get_settings,
            set_relay_url,
            set_theme,
            set_privacy,
            update_settings,
            remove_contact,
            delete_message,
            set_display_name,
            send_typing,
            get_presence,
            register_profile,
            search_users,
            get_profile,
            set_avatar,
            watch_presence,
            create_group,
            get_group_info,
            promote_member,
            demote_member,
            remove_member,
            leave_group
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
