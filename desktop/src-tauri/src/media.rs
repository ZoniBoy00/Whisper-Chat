//! Rust-owned encrypted media transfer and cache.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

const MAX_MEDIA_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaInfo {
    pub hash: String,
    pub mime: String,
    pub size: u64,
    pub name: Option<String>,
    pub duration_ms: Option<u64>,
    pub local_path: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("media file exceeds 100 MiB")]
    TooLarge,
    #[error("unsupported media type")]
    UnsupportedType,
    #[error("media metadata error: {0}")]
    Metadata(String),
    #[error("media transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("media IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("media crypto error: {0}")]
    Crypto(#[from] e2ee_core::media::MediaError),
    #[error("media hash mismatch")]
    HashMismatch,
}

pub fn cache_dir(app: &AppHandle) -> Result<PathBuf, MediaError> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("media").join("decrypted"))
        .map_err(|e| MediaError::Io(std::io::Error::other(e.to_string())))
}

/// Open a previously decrypted cache file without allowing arbitrary paths.
pub fn open_cached_file(app: &AppHandle, path: &Path) -> Result<(), MediaError> {
    let root = cache_dir(app)?;
    let root = root.canonicalize().map_err(MediaError::Io)?;
    let candidate = path.canonicalize().map_err(MediaError::Io)?;
    if !candidate.starts_with(&root) {
        return Err(MediaError::Metadata(
            "path is outside the media cache".into(),
        ));
    }
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &candidate.to_string_lossy()])
            .spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(&candidate).spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&candidate)
            .spawn()?;
    }
    Ok(())
}
pub fn mime_for_path(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "mp3" => Some("audio/mpeg"),
        "wav" => Some("audio/wav"),
        "ogg" => Some("audio/ogg"),
        "mp4" => Some("video/mp4"),
        "webm" => Some("video/webm"),
        "pdf" => Some("application/pdf"),
        "txt" => Some("text/plain"),
        "zip" => Some("application/zip"),
        _ => None,
    }
}
pub fn relay_http_url(relay: &str) -> String {
    relay
        .replace("wss://", "https://")
        .replace("ws://", "http://")
}
fn safe_name(name: Option<&str>, hash: &str) -> String {
    let candidate = name.unwrap_or("media");
    let basename = Path::new(candidate)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("media");
    let clean: String = basename
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || ".-_".contains(*c))
        .collect();
    format!(
        "{}-{}",
        hash,
        if clean.is_empty() { "media" } else { &clean }
    )
}

pub async fn send_file(
    _app: &AppHandle,
    relay: &str,
    path: &Path,
    message_id: String,
) -> Result<(e2ee_core::MediaPayload, MediaInfo), MediaError> {
    let metadata = tokio::fs::metadata(path).await?;
    if metadata.len() > MAX_MEDIA_BYTES {
        return Err(MediaError::TooLarge);
    }
    let plaintext = tokio::fs::read(path).await?;
    if plaintext.len() as u64 > MAX_MEDIA_BYTES {
        return Err(MediaError::TooLarge);
    }
    let mime = mime_for_path(path).ok_or(MediaError::UnsupportedType)?;
    let key = e2ee_core::media::generate_key();
    let blob = e2ee_core::media::encrypt(&plaintext, &key)?;
    let hash = e2ee_core::media::hash(&blob);
    Client::new()
        .post(format!(
            "{}/media",
            relay_http_url(relay).trim_end_matches('/')
        ))
        .body(blob)
        .send()
        .await?
        .error_for_status()?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .map(ToOwned::to_owned);
    let payload = e2ee_core::MediaPayload::new(
        message_id,
        hash.clone(),
        BASE64.encode(key),
        mime,
        plaintext.len() as u64,
        name.clone(),
        None,
    )
    .map_err(MediaError::Metadata)?;
    Ok((
        payload,
        MediaInfo {
            hash,
            mime: mime.to_string(),
            size: plaintext.len() as u64,
            name,
            duration_ms: None,
            local_path: None,
        },
    ))
}

pub async fn fetch_and_decrypt(
    app: &AppHandle,
    relay: &str,
    payload: &e2ee_core::MediaPayload,
) -> Result<MediaInfo, MediaError> {
    payload.validate().map_err(MediaError::Metadata)?;
    let dir = cache_dir(app)?;
    tokio::fs::create_dir_all(&dir).await?;
    let target = dir.join(safe_name(payload.name.as_deref(), &payload.hash));
    if !target.exists() || tokio::fs::metadata(&target).await?.len() != payload.size {
        let blob = Client::new()
            .get(format!(
                "{}/media/{}",
                relay_http_url(relay).trim_end_matches('/'),
                payload.hash
            ))
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        if e2ee_core::media::hash(&blob) != payload.hash {
            return Err(MediaError::HashMismatch);
        }
        let plaintext = e2ee_core::media::decrypt(
            &blob,
            &payload
                .key_bytes()
                .ok_or_else(|| MediaError::Metadata("invalid key".into()))?,
        )?;
        if plaintext.len() as u64 != payload.size {
            return Err(MediaError::HashMismatch);
        }
        tokio::fs::write(&target, plaintext).await?;
    }
    Ok(MediaInfo {
        hash: payload.hash.clone(),
        mime: payload.mime.clone(),
        size: payload.size,
        name: payload.name.clone(),
        duration_ms: payload.duration_ms,
        local_path: Some(target.to_string_lossy().into_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relay_urls_upgrade_tls() {
        assert_eq!(relay_http_url("wss://x/ws"), "https://x/ws");
    }
    #[test]
    fn names_cannot_escape_cache() {
        assert!(safe_name(Some("../../x"), "a").starts_with("a-"));
    }
}
