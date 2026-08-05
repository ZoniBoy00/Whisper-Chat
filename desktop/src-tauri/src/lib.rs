use std::fs;
use std::path::PathBuf;

use serde::Serialize;
use tauri::{Manager, State};

mod relay;

use relay::{ChatState, RelayClient};

/// Resolve the on-disk location of the persisted identity.
fn identity_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
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
/// only reaches this command when no identity is present.
#[tauri::command]
fn generate_identity(app: tauri::AppHandle) -> Result<IdentityInfo, String> {
    let identity = e2ee_core::Identity::new();
    let peer_id = identity.peer_id();
    let json = identity.to_json().map_err(|e| e.to_string())?;

    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join("identity.json"), json).map_err(|e| e.to_string())?;

    Ok(IdentityInfo {
        peer_id,
        exists: true,
    })
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            app.manage(RelayClient::new(app.handle().clone()));
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
            reset_relay
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
