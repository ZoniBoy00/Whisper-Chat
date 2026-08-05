use std::fs;
use std::path::PathBuf;

use serde::Serialize;
use tauri::Manager;

/// Identity information reported to the UI.
#[derive(Serialize)]
struct IdentityInfo {
    peer_id: String,
    exists: bool,
}

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_identity, generate_identity])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
