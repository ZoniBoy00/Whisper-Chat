# Whisper — Media System (encrypted file transfer) — v1 spec

> **Status:** In progress (phase 7). Desktop encrypted file transfer, relay blob
> upload/download, Rust-owned cache and basic image/file UI are implemented.
> Thumbnails, streaming optimization, mobile parity, voice messages and calls
> are still pending.
> **Goal:** end-to-end-encrypted images, videos and files inside chats and
> groups, without ever breaking the zero-knowledge relay model.
> **Related:** `docs/ROADMAP.md` phase 7.

## 1. The core idea

A file is encrypted **on the sender's device** with a freshly generated
symmetric key; only the opaque ciphertext is uploaded to the relay. The key
and the file's metadata travel **inside an existing E2EE message** (Double
Ratchet for 1:1, Megolm for groups), exactly like reactions and quoted
replies do today.

```
Sender:
  1. pick file → generate a random 256-bit key
  2. encrypt the file with that key (AES-256-GCM, chunked)
  3. POST the ciphertext blob  → relay stores it at /media/<sha256>
  4. send an E2EE message with only:
       { kind: "media", hash, key, mime, size, name?, thumb? }

Recipient:
  1. decrypt the E2EE message → hash + key + meta
  2. GET /media/<hash> → ciphertext blob
  3. decrypt with the key → render / save / forward
```

The relay sees only random-looking ciphertext plus a content hash. It never
sees the key, the mime type, the filename or any plaintext — **zero
knowledge is preserved by construction.**

## 2. Cryptography

Vodozemac provides the message channels (X3DH + Double Ratchet, Megolm) but
does **not** encrypt arbitrary large files. File encryption uses a standard,
audited library instead — the same "no hand-rolled crypto" rule:

| Choice | Crate | Why |
|---|---|---|
| **AES-256-GCM** (chunked) | `aes-gcm` (RustCrypto) | NIST standard, constant-time, audited; per-chunk AEAD |
| **XChaCha20-Poly1305** (alt) | `chacha20poly1305` (RustCrypto) | faster on software-only devices; also standard |
| **Streaming secretbox** (alt) | `libsodium` / `crypto_secretstream` | purpose-built for large-file streaming; libsodium is widely audited |

Recommended v1: **AES-256-GCM with a random nonce per file** (or per chunk
when streaming). Key size 256 bits, generated with `getrandom`/`rand`.

Rules:
- One random key **per file**, never reused.
- The key travels ONLY inside an encrypted `ChatPayload::Media` envelope.
- No key material is ever written to the relay database or logs.
- Media crypto lives in `e2ee-core` (new `media.rs`) so mobile reuses it.

## 3. Wire protocol (client → client)

New plaintext payload kind in `e2ee-core/src/payload.rs` (same tagged
envelope as reactions/replies, so it is opaque to the relay):

```json
{
  "kind": "media",
  "hash": "sha256 hex of the ciphertext blob",
  "key": "base64, 32 bytes",
  "mime": "image/jpeg",
  "size": 123456,
  "name": "IMG_2026.jpg",          // optional, display only
  "thumb": { "hash": "...", "mime": "image/jpeg", "size": 2048 },
  "duration_ms": 3500               // optional, videos/voice only
}
```

- `hash` is the address: `GET /media/<hash>` returns the blob.
- `key` decrypts the blob. The thumbnail (`thumb`) uses its own key embedded
  in the same payload, so a client can show a preview before (or without)
  downloading the full file.
- Old clients that cannot parse `kind: "media"` fall back to raw text via the
  existing `parse_plaintext` — the message degrades gracefully.

## 4. Relay storage

Reuses the existing `/media` infrastructure (avatars already live there):

| Endpoint | Method | Behaviour |
|---|---|---|
| `/media/{hash}` | GET | Serve the blob (existing) |
| `/media` | POST | Upload a ciphertext blob, content-addressed, returns `{ hash }` |

Server rules:
- Blob stored at `server/data/media/<sha256>.bin` (content-addressed: same
  file = same hash = no duplicates).
- **No metadata stored**: no mime, no filename, no owner, no plaintext.
  The hash is the only index.
- Upload limit: **100 MB per file** (configurable `WHISPER_MEDIA_MAX_BYTES`),
  rate-limited per IP (reuse the token-bucket pattern).
- TTL: blobs referenced by live messages live for **30 days**; the cleanup
  job (already hourly for envelopes) also purges media older than the TTL
  that no client re-fetched.
- Optional: reference counting is not needed for v1 — TTL is the safety net.

## 5. Client storage

| What | Where | Notes |
|---|---|---|
| Ciphertext cache | `%APPDATA%\com.whisper.desktop\media\` | transient; re-downloadable from the relay until TTL |
| Decrypted files | `%APPDATA%\com.whisper.desktop\media\decrypted\` | local only |
| Media metadata | messages table (new `media_json` column) | hash, key, mime, size, name, thumb |

- The DB stores the metadata + key (the DB is SQLCipher-encrypted already),
  so history and backups keep working.
- "Clear chat history" also clears the media cache.
- The full-profile backup (`export_everything`) includes the decrypted cache
  when it exists.

## 6. Sending flow (desktop)

1. User picks a file (native dialog, already wired via `tauri-plugin-dialog`).
2. `e2ee_core::media::encrypt_file()` streams the file → ciphertext + key.
3. Ciphertext is POSTed to `/media`; the response hash is captured.
4. `send_message` is called with a `ChatPayload::Media` payload (1:1 or group
   path, same as text/reply — no server changes needed for routing).
5. The bubble shows an optimistic progress row; the relay ack flips it to
   "delivered" exactly like text.

## 7. Receiving flow (desktop)

1. The E2EE message arrives; `parse_plaintext` yields `ParsedPayload::Media`.
2. The bubble renders immediately from `thumb` (tiny preview) + metadata.
3. The ciphertext is fetched from `/media/<hash>` and decrypted in a
   background task.
4. Images render inline; videos/files show a card with a play/open action.
5. Decrypted bytes are cached; metadata is persisted so history re-renders
   offline.

## 8. UI plan

- **Image message:** rounded bubble with the image, tap to open fullscreen,
  WhatsApp-style. Thumbnail while downloading, spinner + progress.
- **Video message:** thumbnail + play button; opens a media player.
- **File message:** icon + name + size + open/save button.
- **Voice message:** waveform placeholder + play/pause (phase 7.5).
- Drafts/composer get a paperclip button.

## 9. Security & privacy invariants

- Relay never holds keys or plaintext media — enforced by design (blob has no
  metadata, keys only in E2EE envelopes).
- A compromised relay can serve the ciphertext but cannot decrypt it.
- Forward secrecy is inherited from the session ratchet for the *metadata
  message*; file keys are ephemeral per file.
- Media is not re-encrypted on the server (no reason to: it never sees keys).
- Thumbnails are encrypted with their own key; a client may refuse to
  auto-download full media (setting) like WhatsApp.

## 10. Implementation phases

| # | Work | Tests |
|---|---|---|
| 1 | `e2ee-core::media`: encrypt/decrypt file, chunking, key gen | roundtrip, tamper, large-file, chunk-boundary |
| 2 | Relay `POST /media` + TTL purge + size/rate limits | upload→fetch, limit enforcement, purge |
| 3 | `ChatPayload::Media` + `ParsedPayload::Media` + ingest paths | wire roundtrip, legacy fallback |
| 4 | Desktop send: picker → encrypt → upload → send | smoke flow 1:1 + group |
| 5 | Desktop receive: fetch → decrypt → render (image/video/file) | two-client smoke, offline cache |
| 6 | Settings: auto-download toggle, cache management | UI + persistence tests |

Each phase is one pipeline run (Planner → Coder → Tester → Reviewer →
Reporter → approval → push), TDD-first for the crypto.

## 11. Open questions

1. Video transcoding (sizes)? v1: send as-is, client plays what it can.
2. Voice messages: opus/WebM chunks over the same media channel.
3. Do we need sender-side progress + pause/resume for very large files?
4. Group media: recipients fetch individually (v1) vs relay fan-out of the
   blob (later, to save bandwidth).

---

*Whisper — "I've got this." 🔒*
