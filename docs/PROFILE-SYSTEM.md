# Whisper — Username & Profile System (v1 spec)

> Implementation spec for WhatsApp/Signal-style registration: unique username,
> display name, avatar, and search by username + UID.
> Status: **implemented 2026-08-05** (e2ee-core profile.rs, relay register/search/get, avatars via /media).

## Goal

Users can register a unique username, set a display name and an avatar, and be
found by searching username or UID (peer ID). The crypto identity stays the
local keypair — the username is a **signed directory alias** bound to the
user's public key, exactly like Signal maps a phone number to a key.

## Concepts

| Term | Definition |
|---|---|
| **UID / peer ID** | Existing 24-hex fingerprint of the X25519 identity key. Unchanged — stays the routing address and the crypto identity. |
| **Username** | User-chosen unique alias: lowercase `[a-z0-9_]`, 3–32 chars. |
| **Display name** | Free-form, optional, shown in conversations (e.g. "Tersika"). |
| **Avatar** | Optional image, public by design (like Signal), served by the relay. |

## Security model (critical)

- **Signed binding** — Username → public key binding MUST be signed with the
  user's Ed25519 key. The relay stores the signature and re-verifies on every
  lookup/registration. This prevents a compromised relay or DB breach from
  hijacking a username to an attacker's key.
- **Authenticated registration** — username can only be registered by the
  authenticated WS peer (signed hello). No anonymous claims.
- **No email / phone / password.** "Login" = reconnect with the existing local
  identity. Cross-device recovery is a separate feature (seed phrase / backup)
  — out of scope for this spec.
- **Rate limits** — separate `profile:<ip>` bucket for registration + search
  (same pattern as the existing `prekey:<ip>` bucket).

## Canonical signing bytes

```
canonical = username_utf8_bytes || 0x00 || curve25519_public_key_raw(32 bytes)
signature = Ed25519.sign(canonical)
```

Relay verify: load the peer's Ed25519 key from `users`, recompute canonical
from the claimed username + stored curve25519 key, verify signature.

## DB schema (server store)

Extend the `users` table (migration, ALTER TABLE ... ADD COLUMN helper):

```sql
ALTER TABLE users ADD COLUMN username           TEXT;   -- UNIQUE, nullable
ALTER TABLE users ADD COLUMN username_signature TEXT;   -- Ed25519 sig (base64)
ALTER TABLE users ADD COLUMN display_name       TEXT DEFAULT '';
ALTER TABLE users ADD COLUMN avatar_hash        TEXT;   -- sha256 of avatar blob
ALTER TABLE users ADD COLUMN registered_at      INTEGER; -- unix seconds
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_username ON users(username) WHERE username IS NOT NULL;
```

## Wire protocol (WS client → server)

| Client message | Fields | Behavior |
|---|---|---|
| `register_profile` | `username`, `signature` (base64), `display_name?`, `avatar?` (base64 image, ≤2 MB) | Validate username + signature, reject duplicates/reserved, store, reply `profile_registered` |
| `search_users` | `query`, `limit?` (default 10, max 25) | Prefix match on username (case-insensitive) + prefix match on UID; returns `users_search` |
| `get_profile` | `peer_id` | Returns profile for one peer; `no_profile` if none |

Server messages:

| Server message | Fields |
|---|---|
| `profile_registered` | `username` |
| `users_search` | `results: [{ username?, peer_id, display_name, avatar_url? }]` |
| `profile` | `{ username?, peer_id, display_name, avatar_url?, curve25519_key? }` |
| `error` | existing codes + `invalid_username`, `username_taken`, `bad_signature`, `no_profile`, `invalid_avatar` |

## Avatar

- Uploaded with `register_profile` (authenticated).
- Stored content-addressed: `sha256(blob)`; served at `GET /media/{hash}`.
- Public (not E2EE) — avatars are identity metadata, same as Signal/WhatsApp.
- Limits: ≤2 MB, decode as PNG/JPEG/WebP, ≤1024×1024 (reject oversized).
- No avatar = client renders the initial letter / identicon locally.

## Username validation & reserved names

```
^[a-z0-9_]{3,32}$   (lowercase; normalize input by lowercasing)
Reserved: admin, whisper, support, mod, system, root (configurable list)
```

Empty username = peer has no alias → UI shows UID only.

## UI (desktop)

- Settings → Profile: username field with live validation, display name,
  avatar picker (file → upload → preview). "Keys never leave this device" text stays.
- Sidebar search: extend "Search by Whisper ID" → search by username OR UID
  (prefix). Result rows show avatar + display name + username.
- Conversation header: show display name + @username, fallback to UID.
- Onboarding: optional "choose your username" step (skippable — UID works
  without a username).

## Anti-abuse

- Registration rate limit: ~5 claims/hour/IP (token bucket `profile:<ip>`,
  configurable via `WHISPER_PROFILE_RATE_BURST` / `WHISPER_PROFILE_RATE_REFILL`).
- Username squatting: later phase — claim + 30-day inactivity release.
- Search rate limit: shared `profile:<ip>` bucket.

## Files touched

| File | Purpose |
|---|---|
| `e2ee-core/src/profile.rs` (new) | Username validation, `canonical_bytes()`, `sign_username()`, `verify_username_signature()` |
| `e2ee-core/src/lib.rs` | Module + re-exports |
| `server/src/store.rs` | users-table migration + `register_username`, `get_profile`, `search_users`, avatar hash |
| `server/src/relay.rs` | `register_profile` / `search_users` / `get_profile` handlers (authenticated, rate limited, signature-verified) |
| `server/src/main.rs` | `GET /media/:hash` blob serving |
| `server/tests/smoke.mjs` | registration → search → get_profile roundtrip, tampered-signature rejection, duplicate-username rejection |
| `desktop/...` | commands, bindings, Settings → Profile, Sidebar search (UI work in progress) |

## Tests (required — done)

- Unit: username validation (length, charset, reserved), canonical bytes,
  signature roundtrip + tamper rejection.
- Relay: register → search → get_profile roundtrip; duplicate username →
  `username_taken`; tampered signature → `bad_signature`; unauthenticated
  register → rejected; reserved name → `invalid_username`.
- Smoke: full flow with two identities (register both, search each other,
  fetch profile), rate-limit block.

## Out of scope (later)

- Cross-device recovery (seed phrase / backup key)
- Username release / inactivity cleanup
- Profile encryption (avatars stay public)
- End-to-end profile sync (e.g. display-name changes pushed to contacts)
