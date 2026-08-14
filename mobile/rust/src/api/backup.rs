//! Password-encrypted full backups — a direct port of the desktop client's
//! `backup.rs`: Argon2id (KDF) + AES-256-GCM (AEAD) over the whole package.
//! The identity's private keys never leave the device in cleartext.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Algorithm::Argon2id;
use argon2::{Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rand::TryRng;

/// KDF and AEAD parameters for version-2 backups.
const KDF_ALGO: &str = "argon2id";
const KDF_M_COST: u32 = 19_456; // 19 MiB — OWASP-recommended Argon2id cost
const KDF_T_COST: u32 = 2;
const KDF_P_COST: u32 = 1;
const KEY_LEN: usize = 32; // AES-256
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12; // 96-bit GCM nonce

/// Errors produced while sealing or opening a backup package.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BackupError {
    /// The file is not a valid v2 backup package.
    InvalidFormat(String),
    /// The password did not match (AES-GCM authentication failed).
    WrongPassword,
    /// A low-level crypto failure.
    Crypto(String),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupError::InvalidFormat(m) => write!(f, "invalid backup file: {m}"),
            BackupError::WrongPassword => write!(f, "wrong backup password"),
            BackupError::Crypto(m) => write!(f, "backup crypto failure: {m}"),
        }
    }
}

/// Serialize the envelope's public metadata as authenticated data (AAD).
/// GCM authenticates this alongside the ciphertext, so tampering with the KDF
/// parameters (weakening the password stretch) fails decryption.
fn authenticated_metadata(
    kind: &str,
    version: u64,
    kdf: &serde_json::Value,
    nonce_b64: &str,
) -> Vec<u8> {
    let meta = serde_json::json!({
        "kind": kind,
        "version": version,
        "kdf": kdf,
        "nonce": nonce_b64,
    });
    serde_json::to_vec(&meta).expect("serialize aad must not fail")
}

/// Derive the 32-byte AES-256 key from `password` + `salt` with Argon2id.
fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; KEY_LEN], BackupError> {
    derive_key_with_costs(password, salt, KDF_M_COST, KDF_T_COST, KDF_P_COST)
}

/// Derive the key with explicit Argon2id costs (read from the file on open).
fn derive_key_with_costs(
    password: &str,
    salt: &[u8],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
) -> Result<[u8; KEY_LEN], BackupError> {
    let params = Params::new(m_cost, t_cost, p_cost, Some(KEY_LEN))
        .map_err(|e| BackupError::InvalidFormat(format!("invalid kdf parameters: {e}")))?;
    let argon = Argon2::new(Argon2id, Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| BackupError::Crypto(e.to_string()))?;
    Ok(key)
}

/// Seal `plaintext` (the full backup body as JSON) under `password`.
/// Returns the v2 package as a JSON string. A fresh salt + nonce per call.
/// FFI-facing: errors are human-readable strings.
pub fn backup_encrypt(plaintext: &str, password: &str) -> Result<String, String> {
    encrypt_package(plaintext, password).map_err(|e| e.to_string())
}

pub(crate) fn encrypt_package(
    plaintext: &str,
    password: &str,
) -> Result<String, BackupError> {
    if password.len() < 8 {
        return Err(BackupError::InvalidFormat(
            "password must be at least 8 characters".into(),
        ));
    }
    let mut salt = [0u8; SALT_LEN];
    rand::rngs::SysRng
        .try_fill_bytes(&mut salt)
        .map_err(|e| BackupError::Crypto(e.to_string()))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::SysRng
        .try_fill_bytes(&mut nonce_bytes)
        .map_err(|e| BackupError::Crypto(e.to_string()))?;

    let kdf = serde_json::json!({
        "algo": KDF_ALGO,
        "salt": B64.encode(salt),
        "m_cost": KDF_M_COST,
        "t_cost": KDF_T_COST,
        "p_cost": KDF_P_COST,
        "length": KEY_LEN,
    });
    let nonce_b64 = B64.encode(nonce_bytes);
    let aad = authenticated_metadata("whisper-backup", 2, &kdf, &nonce_b64);

    let key = derive_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| BackupError::Crypto(e.to_string()))?;
    let nonce = Nonce::try_from(nonce_bytes.as_slice())
        .map_err(|_| BackupError::Crypto("invalid nonce length".into()))?;
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext.as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|e| BackupError::Crypto(e.to_string()))?;

    serde_json::to_string(&serde_json::json!({
        "kind": "whisper-backup",
        "version": 2,
        "kdf": kdf,
        "nonce": nonce_b64,
        "ciphertext_b64": B64.encode(ciphertext),
    }))
    .map_err(|e| BackupError::Crypto(e.to_string()))
}

/// Open a v2 package (JSON string) produced by [`backup_encrypt`].
/// FFI-facing: errors are human-readable strings.
pub fn backup_decrypt(package_json: &str, password: &str) -> Result<String, String> {
    decrypt_package(package_json, password).map_err(|e| e.to_string())
}

pub(crate) fn decrypt_package(
    package_json: &str,
    password: &str,
) -> Result<String, BackupError> {
    let package: serde_json::Value = serde_json::from_str(package_json)
        .map_err(|_| BackupError::InvalidFormat("not a valid backup".into()))?;

    let version = package
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| BackupError::InvalidFormat("missing version".into()))?;
    if version != 2 {
        return Err(BackupError::InvalidFormat(format!(
            "unsupported backup version {version} — this backup was not encrypted; \
             create a new one with the current Whisper"
        )));
    }

    let kdf = package
        .get("kdf")
        .and_then(|k| k.as_object())
        .ok_or_else(|| BackupError::InvalidFormat("missing kdf parameters".into()))?;
    let algo = kdf
        .get("algo")
        .and_then(|a| a.as_str())
        .ok_or_else(|| BackupError::InvalidFormat("missing kdf algorithm".into()))?;
    if algo != KDF_ALGO {
        return Err(BackupError::InvalidFormat(format!(
            "unsupported kdf algorithm {algo}"
        )));
    }
    let m_cost = kdf
        .get("m_cost")
        .and_then(|v| v.as_u64())
        .unwrap_or(KDF_M_COST as u64) as u32;
    let t_cost = kdf
        .get("t_cost")
        .and_then(|v| v.as_u64())
        .unwrap_or(KDF_T_COST as u64) as u32;
    let p_cost = kdf
        .get("p_cost")
        .and_then(|v| v.as_u64())
        .unwrap_or(KDF_P_COST as u64) as u32;
    let salt = B64
        .decode(kdf.get("salt").and_then(|s| s.as_str()).unwrap_or(""))
        .map_err(|_| BackupError::InvalidFormat("bad kdf salt".into()))?;
    let nonce_b64 = package
        .get("nonce")
        .and_then(|n| n.as_str())
        .ok_or_else(|| BackupError::InvalidFormat("missing nonce".into()))?;
    let nonce_bytes = B64
        .decode(nonce_b64)
        .map_err(|_| BackupError::InvalidFormat("bad nonce".into()))?;
    let ciphertext_b64 = package
        .get("ciphertext_b64")
        .and_then(|c| c.as_str())
        .ok_or_else(|| BackupError::InvalidFormat("missing ciphertext".into()))?;
    let ciphertext = B64
        .decode(ciphertext_b64)
        .map_err(|_| BackupError::InvalidFormat("bad ciphertext encoding".into()))?;

    let aad = authenticated_metadata(
        "whisper-backup",
        version,
        &serde_json::Value::Object(kdf.clone()),
        nonce_b64,
    );

    let key = derive_key_with_costs(password, &salt, m_cost, t_cost, p_cost)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| BackupError::Crypto(e.to_string()))?;
    let nonce = Nonce::try_from(nonce_bytes.as_slice())
        .map_err(|_| BackupError::Crypto("invalid nonce length".into()))?;
    let plaintext = cipher
        .decrypt(
            &nonce,
            Payload {
                msg: ciphertext.as_ref(),
                aad: &aad,
            },
        )
        .map_err(|_| BackupError::WrongPassword)?;

    String::from_utf8(plaintext).map_err(|_| BackupError::InvalidFormat("bad plaintext".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "correct horse battery staple";

    #[test]
    fn encrypt_decrypt_roundtrips_the_plaintext_body() {
        let body = r#"{"identity":"pickle","database_b64":"AAAA","created_at":"2026-08-09"}"#;
        let package = encrypt_package(body, PASSWORD).expect("encrypt");
        assert!(package.contains("\"kind\":\"whisper-backup\""));
        assert!(package.contains("\"version\":2"));
        let opened = decrypt_package(&package, PASSWORD).expect("decrypt");
        assert!(opened.contains("\"identity\":\"pickle\""));
    }

    #[test]
    fn wrong_password_is_rejected() {
        let package = encrypt_package(r#"{"identity":"pickle"}"#, PASSWORD).expect("encrypt");
        assert_eq!(
            decrypt_package(&package, "wrong password"),
            Err(BackupError::WrongPassword)
        );
    }

    #[test]
    fn identical_bodies_produce_distinct_ciphertexts() {
        let a = encrypt_package(r#"{"identity":"same"}"#, PASSWORD).expect("a");
        let b = encrypt_package(r#"{"identity":"same"}"#, PASSWORD).expect("b");
        assert_ne!(a, b);
    }

    #[test]
    fn tampered_ciphertext_fails_authentication() {
        let package = encrypt_package(r#"{"identity":"pickle"}"#, PASSWORD).expect("encrypt");
        let mut v: serde_json::Value = serde_json::from_str(&package).unwrap();
        let ct = v["ciphertext_b64"].as_str().unwrap().to_string();
        let mut bytes = B64.decode(&ct).expect("decode");
        bytes[0] ^= 0x01;
        v["ciphertext_b64"] = serde_json::Value::String(B64.encode(bytes));
        assert_eq!(
            decrypt_package(&v.to_string(), PASSWORD),
            Err(BackupError::WrongPassword)
        );
    }

    #[test]
    fn short_password_is_rejected() {
        assert!(matches!(
            encrypt_package("{}", "short"),
            Err(BackupError::InvalidFormat(_))
        ));
    }
}
