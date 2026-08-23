//! Versioned chunked encryption for opaque media blobs.

use aes_gcm::{
    aead::{AeadInOut, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngExt;
use sha2::{Digest, Sha256};
use std::{fmt, mem};

/// A 256-bit AES-GCM key, generated once for each media file.
pub type MediaKey = [u8; 32];

const MAGIC: &[u8; 4] = b"WHM1";
const HEADER_LEN: usize = 4 + 4 + 8 + 8;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
/// 64 KiB keeps memory use bounded while avoiding excessive AEAD overhead.
pub const CHUNK_SIZE: usize = 64 * 1024;
const MAX_CHUNKS: u64 = 1 << 32;

/// Errors returned when a media blob is malformed or cannot be authenticated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    InvalidKey,
    InvalidHeader,
    InvalidMetadata,
    InvalidChunk,
    AuthenticationFailed,
    SizeOverflow,
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidKey => "media key must be 32 bytes",
            Self::InvalidHeader => "invalid media header",
            Self::InvalidMetadata => "invalid media metadata",
            Self::InvalidChunk => "invalid media chunk",
            Self::AuthenticationFailed => "media authentication failed",
            Self::SizeOverflow => "media size overflow",
        })
    }
}

impl std::error::Error for MediaError {}

/// Generate a cryptographically secure key for one media file.
pub fn generate_key() -> MediaKey {
    let mut key = [0u8; 32];
    rand::rng().fill(&mut key);
    key
}

/// Encrypt bytes into the version-1 chunked media blob format.
pub fn encrypt(plaintext: &[u8], key: &MediaKey) -> Result<Vec<u8>, MediaError> {
    // Empty files still carry one authenticated zero-length record so their
    // header cannot be modified without detection.
    let chunk_count = if plaintext.is_empty() {
        1
    } else {
        plaintext.len().div_ceil(CHUNK_SIZE) as u64
    };
    if chunk_count > MAX_CHUNKS {
        return Err(MediaError::SizeOverflow);
    }

    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&(CHUNK_SIZE as u32).to_le_bytes());
    header.extend_from_slice(&(plaintext.len() as u64).to_le_bytes());
    header.extend_from_slice(&chunk_count.to_le_bytes());

    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| MediaError::InvalidKey)?;
    let mut blob = header.clone();
    let mut seal_chunk = |index: u64, chunk: &[u8]| -> Result<(), MediaError> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::rng().fill(&mut nonce);
        let mut ciphertext = chunk.to_vec();
        cipher
            .encrypt_in_place(
                &Nonce::try_from(&nonce[..]).expect("nonce length is fixed"),
                &chunk_aad(&header, index),
                &mut ciphertext,
            )
            .map_err(|_| MediaError::AuthenticationFailed)?;
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(())
    };
    if plaintext.is_empty() {
        seal_chunk(0, &[])?;
    } else {
        for (index, chunk) in plaintext.chunks(CHUNK_SIZE).enumerate() {
            seal_chunk(index as u64, chunk)?;
        }
    }
    Ok(blob)
}

/// Decrypt and fully validate a version-1 chunked media blob.
pub fn decrypt(blob: &[u8], key: &MediaKey) -> Result<Vec<u8>, MediaError> {
    if blob.len() < HEADER_LEN || &blob[..4] != MAGIC {
        return Err(MediaError::InvalidHeader);
    }
    let chunk_size = u32::from_le_bytes(blob[4..8].try_into().unwrap()) as usize;
    let plain_size = u64::from_le_bytes(blob[8..16].try_into().unwrap());
    let chunk_count = u64::from_le_bytes(blob[16..24].try_into().unwrap());
    if chunk_size != CHUNK_SIZE || chunk_count > MAX_CHUNKS {
        return Err(MediaError::InvalidMetadata);
    }
    let expected_count = if plain_size == 0 {
        1
    } else {
        (plain_size - 1) / chunk_size as u64 + 1
    };
    if chunk_count != expected_count || plain_size > usize::MAX as u64 {
        return Err(MediaError::InvalidMetadata);
    }

    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| MediaError::InvalidKey)?;
    let header = &blob[..HEADER_LEN];
    let mut offset = HEADER_LEN;
    let mut plaintext = Vec::with_capacity(plain_size as usize);
    for index in 0..chunk_count {
        let remaining_plain = plain_size - index * chunk_size as u64;
        let plain_len = remaining_plain.min(chunk_size as u64) as usize;
        let record_len = NONCE_LEN
            .checked_add(plain_len)
            .and_then(|n| n.checked_add(TAG_LEN))
            .ok_or(MediaError::SizeOverflow)?;
        let end = offset
            .checked_add(record_len)
            .ok_or(MediaError::SizeOverflow)?;
        if end > blob.len() {
            return Err(MediaError::InvalidChunk);
        }
        let nonce = &blob[offset..offset + NONCE_LEN];
        let mut chunk = blob[offset + NONCE_LEN..end].to_vec();
        cipher
            .decrypt_in_place(
                &Nonce::try_from(nonce).map_err(|_| MediaError::InvalidChunk)?,
                &chunk_aad(header, index),
                &mut chunk,
            )
            .map_err(|_| MediaError::AuthenticationFailed)?;
        plaintext.extend_from_slice(&chunk);
        offset = end;
    }
    if offset != blob.len() || plaintext.len() != plain_size as usize {
        return Err(MediaError::InvalidChunk);
    }
    Ok(plaintext)
}

/// Return the lowercase SHA-256 address of an encrypted blob.
pub fn hash(blob: &[u8]) -> String {
    let digest = Sha256::digest(blob);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn chunk_aad(header: &[u8], index: u64) -> Vec<u8> {
    let mut aad = Vec::with_capacity(header.len() + mem::size_of::<u64>());
    aad.extend_from_slice(header);
    aad.extend_from_slice(&index.to_le_bytes());
    aad
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_empty_and_multi_chunk() {
        let key = generate_key();
        for data in [Vec::new(), vec![7; CHUNK_SIZE * 2 + 19]] {
            let blob = encrypt(&data, &key).unwrap();
            assert_eq!(decrypt(&blob, &key).unwrap(), data);
        }
    }

    #[test]
    fn tamper_and_wrong_key_fail() {
        let key = generate_key();
        let mut blob = encrypt(b"secret", &key).unwrap();
        blob[HEADER_LEN + NONCE_LEN] ^= 1;
        assert!(matches!(
            decrypt(&blob, &key),
            Err(MediaError::AuthenticationFailed)
        ));
        let blob = encrypt(b"secret", &key).unwrap();
        assert!(decrypt(&blob, &generate_key()).is_err());
    }

    #[test]
    fn invalid_metadata_and_trailing_data_fail() {
        let key = generate_key();
        let mut blob = encrypt(b"secret", &key).unwrap();
        blob[4] = 1;
        assert_eq!(decrypt(&blob, &key), Err(MediaError::InvalidMetadata));
        let mut blob = encrypt(b"secret", &key).unwrap();
        blob.push(0);
        assert_eq!(decrypt(&blob, &key), Err(MediaError::InvalidChunk));
    }
}
