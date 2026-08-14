//! Password-encrypted full-profile backups.
//!
//! The identity pickle holds the user's **private keys**, so a backup package
//! must never leave the device in plaintext. This module wraps a backup's
//! plaintext body (`identity` + base64 SQLCipher database + metadata) in an
//! AES-256-GCM envelope whose key is derived from a user-chosen password via
//! Argon2id. The result is opaque: anyone holding the backup file without the
//! password learns nothing — the identity, the history and the DB key (which
//! is itself derived from the identity) stay sealed.
//!
//! Backup format (version 2):
//!
//! ```json
//! {
//!   "kind": "whisper-backup",
//!   "version": 2,
//!   "kdf": { "algo": "argon2id", "salt": "<b64>",
//!            "m_cost": 19456, "t_cost": 2, "p_cost": 1, "length": 32 },
//!   "nonce": "<b64 12B>",
//!   "ciphertext_b64": "<b64 AES-256-GCM>"
//! }
//! ```
//!
//! The pre-v2 plaintext format is deliberately rejected: accepting it would
//! keep a password-less path to the private keys alive.

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
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BackupError {
    /// The file is not a valid v2 backup package (missing fields, bad base64,
    /// unsupported KDF, or an old pre-v2 plaintext backup).
    #[error("invalid backup file: {0}")]
    InvalidFormat(String),
    /// The password did not match (AES-GCM authentication failed).
    #[error("wrong backup password")]
    WrongPassword,
    /// A low-level crypto failure (key derivation or AEAD error).
    #[error("backup crypto failure: {0}")]
    Crypto(String),
}

/// Minimum accepted password length, enforced by the UI and the backend.
pub const MIN_PASSWORD_LEN: usize = 8;

/// Serialize the envelope's public metadata as authenticated data (AAD).
/// GCM authenticates this alongside the ciphertext, so tampering with ANY of
/// it fails decryption. This matters for the KDF parameters in particular:
/// without binding, an attacker who weakened `m_cost`/`t_cost`/`p_cost` in
/// the file would cheapen the password stretch and speed up brute force.
/// The serialization is deterministic (serde_json maps sort keys), and the
/// decrypt path rebuilds the AAD from the exact values it read back.
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

/// Seal `plaintext` (the full backup body as JSON) under `password`.
/// Returns the v2 package described above. A fresh random salt + nonce are
/// generated for every call, so identical bodies never produce equal outputs.
pub fn encrypt_package(plaintext: &str, password: &str) -> Result<serde_json::Value, BackupError> {
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

    Ok(serde_json::json!({
        "kind": "whisper-backup",
        "version": 2,
        "kdf": kdf,
        "nonce": nonce_b64,
        "ciphertext_b64": B64.encode(ciphertext),
    }))
}

/// Open a v2 package produced by [`encrypt_package`]. Wrong passwords fail
/// with [`BackupError::WrongPassword`]; anything that is not a well-formed v2
/// package (including pre-v2 plaintext backups) fails with
/// [`BackupError::InvalidFormat`]. Resolves with the plaintext JSON body.
pub fn decrypt_package(
    package: &serde_json::Value,
    password: &str,
) -> Result<serde_json::Value, BackupError> {
    // Old plaintext backups (no `kdf` envelope) are rejected on purpose: they
    // held the identity in cleartext and must not silently keep working.
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
    // KDF costs are read from the file (they are bound into the AAD below, so
    // tampering with them fails authentication anyway) — this keeps backups
    // openable if the constants are ever raised in a future version.
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

    // Rebuild the AAD from the exact metadata we read: any tampering with
    // kind/version/kdf/nonce fails GCM authentication below.
    let aad = authenticated_metadata(
        "whisper-backup",
        version,
        &serde_json::Value::Object(kdf.clone()),
        nonce_b64,
    );

    // Invalid (too-small) KDF costs are rejected as a malformed file — the
    // Argon2 parameter validation enforces m_cost >= 8 * p_cost.
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

    serde_json::from_slice(&plaintext)
        .map_err(|e| BackupError::InvalidFormat(format!("bad backup body: {e}")))
}

/// Derive the 32-byte AES-256 key from `password` + `salt` with Argon2id
/// using the current production constants (used when SEALING a backup).
fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; KEY_LEN], BackupError> {
    derive_key_with_costs(password, salt, KDF_M_COST, KDF_T_COST, KDF_P_COST)
}

/// Derive the 32-byte AES-256 key with explicit Argon2id costs (used when
/// OPENING a backup, where the costs are read from the file and bound into
/// the AAD). Invalid parameters surface as a malformed file, not a crash.
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

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "correct horse battery staple";

    #[test]
    fn encrypt_decrypt_roundtrips_the_plaintext_body() {
        let body = r#"{"identity":"pickle","database_b64":"AAAA","created_at":"2026-08-09"}"#;
        let package = encrypt_package(body, PASSWORD).expect("encrypt");
        assert_eq!(package["kind"], "whisper-backup");
        assert_eq!(package["version"], 2);
        assert!(!package["ciphertext_b64"].as_str().unwrap().is_empty());

        let opened = decrypt_package(&package, PASSWORD).expect("decrypt");
        assert_eq!(opened["identity"], "pickle");
        assert_eq!(opened["database_b64"], "AAAA");
        assert_eq!(opened["created_at"], "2026-08-09");
    }

    #[test]
    fn wrong_password_is_rejected_with_no_data() {
        let package = encrypt_package(r#"{"identity":"pickle"}"#, PASSWORD).expect("encrypt");
        assert_eq!(
            decrypt_package(&package, "wrong password"),
            Err(BackupError::WrongPassword)
        );
    }

    #[test]
    fn identical_bodies_produce_distinct_ciphertexts() {
        // A fresh salt + nonce per call means equal bodies never collide —
        // an attacker cannot tell whether two backups hold the same identity.
        let a = encrypt_package(r#"{"identity":"same"}"#, PASSWORD).expect("encrypt a");
        let b = encrypt_package(r#"{"identity":"same"}"#, PASSWORD).expect("encrypt b");
        assert_ne!(a["ciphertext_b64"], b["ciphertext_b64"]);
        assert_ne!(a["kdf"]["salt"], b["kdf"]["salt"]);
    }

    #[test]
    fn tampered_ciphertext_fails_authentication() {
        let package = encrypt_package(r#"{"identity":"pickle"}"#, PASSWORD).expect("encrypt");
        let mut tampered = package.clone();
        // Flip the first ciphertext byte: GCM auth must reject it as a wrong
        // password rather than returning corrupted data.
        let ct = tampered["ciphertext_b64"].as_str().unwrap().to_string();
        let mut bytes = B64.decode(&ct).expect("decode");
        bytes[0] ^= 0x01;
        tampered["ciphertext_b64"] = serde_json::Value::String(B64.encode(bytes));
        assert_eq!(
            decrypt_package(&tampered, PASSWORD),
            Err(BackupError::WrongPassword)
        );
    }

    #[test]
    fn tampered_kdf_costs_fail_authentication() {
        // Weakening the Argon2id costs (m_cost/t_cost/p_cost) is the classic
        // way to cheapen brute force against a stolen backup. The KDF params
        // are bound into the GCM AAD, so editing them must fail decryption —
        // even to values that are themselves valid Argon2 parameters.
        let package = encrypt_package(r#"{"identity":"pickle"}"#, PASSWORD).expect("encrypt");
        for (field, value) in [("m_cost", 1024u64), ("t_cost", 1u64), ("p_cost", 2u64)] {
            let mut tampered = package.clone();
            tampered["kdf"][field] = serde_json::Value::from(value);
            assert_eq!(
                decrypt_package(&tampered, PASSWORD),
                Err(BackupError::WrongPassword),
                "editing {field} must fail authentication"
            );
        }
    }

    #[test]
    fn invalid_kdf_costs_are_rejected_as_a_malformed_file() {
        // m_cost=1 is below Argon2's floor (m_cost >= 8 * p_cost): the file
        // is malformed, not merely authenticated-with-the-wrong-password.
        let package = encrypt_package(r#"{"identity":"pickle"}"#, PASSWORD).expect("encrypt");
        let mut tampered = package.clone();
        tampered["kdf"]["m_cost"] = serde_json::Value::from(1u64);
        assert!(matches!(
            decrypt_package(&tampered, PASSWORD),
            Err(BackupError::InvalidFormat(_))
        ));
    }

    #[test]
    fn tampered_metadata_fields_fail_authentication() {
        // The nonce is part of the authenticated metadata; the version is
        // rejected earlier by the format check. Either way, tampering never
        // yields data.
        let package = encrypt_package(r#"{"identity":"pickle"}"#, PASSWORD).expect("encrypt");

        let mut version_bumped = package.clone();
        version_bumped["version"] = serde_json::Value::from(3u64);
        assert!(matches!(
            decrypt_package(&version_bumped, PASSWORD),
            Err(BackupError::InvalidFormat(_))
        ));

        let mut nonce_flipped = package.clone();
        let nonce = nonce_flipped["nonce"].as_str().unwrap().to_string();
        let mut bytes = B64.decode(&nonce).expect("decode");
        bytes[0] ^= 0x01;
        nonce_flipped["nonce"] = serde_json::Value::String(B64.encode(bytes));
        assert_eq!(
            decrypt_package(&nonce_flipped, PASSWORD),
            Err(BackupError::WrongPassword)
        );
    }

    #[test]
    fn pre_v2_plaintext_backups_are_rejected() {
        // The old format stored the identity in cleartext — it must not be
        // accepted anymore, even with a password supplied.
        let legacy = serde_json::json!({
            "kind": "whisper-backup",
            "version": 1,
            "identity": "plaintext-pickle",
            "database_b64": "AAAA",
        });
        let err = decrypt_package(&legacy, PASSWORD).expect_err("legacy must be rejected");
        assert!(matches!(err, BackupError::InvalidFormat(_)));
    }

    #[test]
    fn missing_fields_are_rejected() {
        let empty = serde_json::json!({});
        assert!(matches!(
            decrypt_package(&empty, PASSWORD),
            Err(BackupError::InvalidFormat(_))
        ));
        let no_cipher = serde_json::json!({
            "kind": "whisper-backup",
            "version": 2,
            "kdf": { "algo": "argon2id", "salt": "AA==" },
            "nonce": "AA==",
        });
        assert!(matches!(
            decrypt_package(&no_cipher, PASSWORD),
            Err(BackupError::InvalidFormat(_))
        ));
    }

    #[test]
    fn unsupported_kdf_is_rejected() {
        let package = encrypt_package(r#"{"identity":"pickle"}"#, PASSWORD).expect("encrypt");
        let mut bogus = package.clone();
        bogus["kdf"]["algo"] = serde_json::Value::String("scrypt".into());
        assert!(matches!(
            decrypt_package(&bogus, PASSWORD),
            Err(BackupError::InvalidFormat(_))
        ));
    }

    #[test]
    fn empty_passwords_are_supported_at_the_crypto_layer_but_guarded_upstream() {
        // The crypto layer itself works with any password string; the
        // MIN_PASSWORD_LEN guard lives in the command layer so the UI enforces
        // a minimum without the backend silently weakening.
        let package = encrypt_package(r#"{"identity":"pickle"}"#, "").expect("encrypt");
        assert!(decrypt_package(&package, "").is_ok());
        assert_eq!(MIN_PASSWORD_LEN, 8);
    }
}
