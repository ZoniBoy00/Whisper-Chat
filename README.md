# Whisper 🔒

> **Your conversations are whispers. Only you and the recipient can hear them.**

Whisper is a privacy-first, end-to-end-encrypted (E2EE) messenger — a
WhatsApp / Signal / Telegram alternative with no backdoors, no scanning and
no plaintext ever reaching the server. Messages are **whispers**: only the
sender and the recipient can read them.

[![CI](https://img.shields.io/badge/CI-passing-brightgreen)](.github/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](#license)
![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange)
![crypto: vodozemac](https://img.shields.io/badge/crypto-vodozemac-blue)

> **Status:** The repository is currently **private**. It will be open-sourced
> at the public beta. Everything below is fully working today.

---

## Features

- **End-to-end encrypted 1:1 messaging** — X3DH key exchange + Double Ratchet
  (via the audited [vodozemac](https://github.com/matrix-org/vodozemac) library).
  Forward and backward secrecy on every message.
- **End-to-end encrypted groups** — Megolm group encryption with owner/admin
  roles, multi-sender (every member sends with their own Megolm session),
  ownership transfer, signed username aliases and avatars (WhatsApp/Signal-style
  group UI).
- **Group invites & join links** — invite a contact (they accept/decline in
  the sidebar) or share a `whisper://join` link; anyone with the link can join
  (WhatsApp-style). Group name and photo travel inside the link.
- **Read receipts, typing & presence** — blue double ticks when a message is
  read (1:1 and groups), live "typing…" indicators with the writer's name in
  groups ("ZoniBoy typing…", "3 members typing…") and Online / Last seen
  status.
- **Group management** — Megolm groups with owner/admin roles, member
  add/remove, group photos, rename, WhatsApp-style "X joined/left" system
  messages and live member counts.
- **Emoji reactions & quoted replies** — react to any message (end-to-end
  encrypted state signals), reply to a message with the quoted bubble rendered
  in the composer (Signal-style). Works in 1:1 chats and groups.
- **Safety numbers & invite links** — Signal-style 60-digit verification
  fingerprint with QR code and a local "verified" flag; share your identity as
  a `whisper://invite` link (opens a profile popup with one-click add) and
  paste/click links to add contacts or join groups.
- **Username & profile system** — register a unique signed username (Ed25519
  binding), set a display name and avatar, search by username or UID.
- **Friend system (anti-spam)** — contacts are established via signed friend
  requests; the relay refuses 1:1 messages, pre-key fetches and group member
  adds between non-contacts (`not_contacts`). A Contacts tab lists every friend
  with live Online / Last seen status and one-click removal.
- **Privacy controls** — hide your online status, disable read receipts or
  typing signals, per-option notification previews.
- **Zero-knowledge blind relay** — the server only ever sees opaque, encrypted
  envelopes. It holds zero plaintext, zero keys and zero message content.
- **No phone number required** — identity is a pure cryptographic key pair.
  Share your peer ID as an invite link. No accounts, no emails.
- **Signed hello / spoofing protection** — connections authenticate with a
  self-signed hello; spoofed sender IDs are rejected (`sender_mismatch`,
  `identity_conflict`, `invalid_hello`).
- **Offline delivery** — SQLite-backed offline queue with a 7-day TTL and
  `fetch_since` sync, so you never miss a message while away.
- **Encrypted local history** — messages, sessions, contacts and settings
  persist in a **SQLCipher-encrypted** SQLite store across restarts. The key is
  derived from the identity (SHA-256 of the identity pickle) and never leaves
  the device; SQLCipher + OpenSSL are statically linked into the binary, so
  users need no extra installs. The codec is detected at runtime via
  `PRAGMA cipher_version` and the database is keyed transparently.
- **Group key rotation** — Megolm group keys rotate every 200 messages
  (backward secrecy): a leaked key can only decrypt messages up to the
  rotation, never anything after it.
- **Rate limiting & DoS guards** — per-IP token bucket (60/min default) and an
  8 MiB envelope size cap.
- **Modern dark UI** — clean, minimalist, Signal/Telegram-grade desktop shell
  (Tauri v2 + React + TypeScript + Tailwind + Lucide icons), Discord-style
  splashscreen and tasteful motion.
- **Full i18n (EN/FI)** — the whole UI is translated into English and Finnish
  with a language switcher.
- **Native notifications & sounds** — desktop notifications, alert sounds and
  per-option notification previews.
- **Unread badges & pinned chats** — per-chat unread counts (cleared on open)
  and pinning your favourite conversations to the top.
- **In-chat message search, day separators & context menus** — find any
  message, grouped history by day, and right-click actions on messages.
- **Auto-reconnect** — the client reconnects to the relay automatically and
  surfaces connection state in the UI. Avatars self-heal: a client opened
  before the relay was up loads every image once the connection is established.
- **Restart-proof sessions** — hydrated sessions and group keys are re-shared
  after a restart, so the first group message sent by a restarted client
  arrives immediately (no warm-up message needed).
- **Daily log files** — each run appends to `whisper-YYYY-MM-DD.log` (local
  time, no ANSI colors) with a one-click "Open logs folder" action in Settings.
- **Automatic backups** — optional daily autobackup (enabled/directory/keep
  count) plus full export/import of identity + history as a single JSON file.
  Every full backup is **password-encrypted** (Argon2id → AES-256-GCM): the
  identity's private keys never leave the device in cleartext. **One password
  covers everything** — set it once (manual export or Settings) and both
  manual and automatic backups reuse it without asking again; restoring asks
  for the password (a wrong one changes nothing on disk).
- **Cross-platform future** — Tauri desktop today, Flutter mobile coming (one
  shared Rust crypto core).
- **Hardened release builds** — `lto="fat"`, `panic="abort"`, `strip=true`
  release profile keeps the binary small and hostile to reverse engineering.

---

## Architecture

Whisper is built around one core principle: **the server is zero-knowledge**.
All cryptography lives in the client-side `e2ee-core` crate; the relay is a
deliberately dumb forwarder.

```
┌────────────────┐   WebSocket (TLS 1.3)   ┌───────────────────────────┐   WebSocket (TLS 1.3)   ┌────────────────┐
│   Client A     │ ───────────────────────▶│        Whisper Relay      │◀───────────────────────│   Client B     │
│   (Tauri)      │    opaque ciphertext    │    (axum + tokio)         │    opaque ciphertext    │   (Tauri)      │
└───────┬────────┘     only, never keys    │   zero-knowledge, sees    │     only, never keys    └───────┬────────┘
        │                                 │   nothing of the content  │                                  │
        │                                 └───────────┬───────────────┘                                  │
        │                                             │ SQLite                                           │
        │  e2ee-core (Rust)                           │ (offline envelopes,                              │  e2ee-core (Rust)
        │  vodozemac: X3DH +                          │  prekeys, users — TTL 7d)                        │  vodozemac: X3DH +
        │  Double Ratchet                             └──────────────────────────────────────────────────┘  Double Ratchet
        └───────────────────────────────────────────────────────────────────────────────────────────────────┘
```

Message lifecycle:

1. Alice fetches Bob's signed **prekey bundle** and runs the X3DH handshake.
2. A **Double Ratchet session** is established; the message is encrypted into a
   ciphertext envelope — *on Alice's device*.
3. The envelope goes to the relay, which stores it (SQLite, TTL) and forwards it.
4. Bob decrypts with his session key. **The server never sees anything but
   opaque ciphertext.**

---

## Tech Stack

| Layer        | Choice                                                          |
|--------------|-----------------------------------------------------------------|
| Shared core  | Rust crate **`e2ee-core`** — one crypto implementation for all   |
| Cryptography | **vodozemac** (X3DH + Double Ratchet) — Apache-2.0, audited     |
| Relay        | Rust + **axum + tokio + WebSocket**                             |
| Server DB    | **SQLite** (ciphertext envelopes only, TTL)                     |
| Desktop      | **Tauri v2** (Rust core) + React + TypeScript                   |
| UI           | **Tailwind CSS** + Lucide icons, dark theme                     |
| CI           | **GitHub Actions** — test, clippy `-D warnings`, fmt on push    |

---

## Security Model

| What is protected                                   | What the server sees                         |
|-----------------------------------------------------|----------------------------------------------|
| Message content — **E2EE** (Double Ratchet)         | Only opaque ciphertext envelopes             |
| Keys — live on the device, never sent anywhere      | Routing metadata (peer IDs) — as with Signal |
| Forward & backward secrecy on every message         | IP addresses (TLS protects them in transit)  |
| No phone number, no personal profile                | Traffic volume patterns                      |

**What there is *not*:** no server-side scanning, no backdoors, no E2EE
workarounds, no telemetry, no plaintext — ever — on the server. By design, a
government/regulator request ("scan all chats") is technically impossible
without rewriting the entire client.

---

## Getting Started

### Prerequisites

- **Rust** toolchain (edition 2021; stable is used in CI)
- **Node.js** (for the desktop frontend) and the **Tauri v2** system
  dependencies for your platform
- On **Windows**, building the full workspace needs **MSVC Build Tools +
  Windows SDK** (see the note under *Testing*)

### 1. Run the relay

```sh
# Build (hardened release profile applies automatically)
cargo build --release

# Run
WHISPER_ADDR=0.0.0.0:8080 cargo run --release
```

The relay listens on `/ws` (WebSocket) and exposes a `/healthz` liveness probe.

### 2. Environment variables (`WHISPER_*`)

Copy `server/.env.example` and adjust (or export directly):

| Variable            | Default          | Purpose                              |
|---------------------|------------------|--------------------------------------|
| `WHISPER_ADDR`      | `0.0.0.0:8080`   | Bind address                         |
| `WHISPER_DB_PATH`   | `data/relay.db`  | SQLite database location             |
| `WHISPER_RATE_BURST`| `60`             | Max envelope burst per IP            |
| `WHISPER_RATE_REFILL`| `1`             | Tokens refilled per second (~60/min) |
| `WHISPER_TRUSTED_PROXIES` | *(empty)* | Comma/space-separated trusted proxy IPs; forwarded headers are only honored from these, so per-IP rate limiting sees the real client behind nginx/Caddy/Cloudflare (empty = direct connections only) |
| `RUST_LOG`          | `info`           | Log level                            |

### 3. Deploy with systemd + a reverse proxy

A hardened, production-ready unit template is included at
`server/deploy/whisper-relay.service` — it runs as a dedicated non-root user
with `NoNewPrivileges`, `ProtectSystem=strict`, `PrivateTmp`, an empty
`CapabilityBoundingSet` and a read-only root filesystem.

For public use the relay sits behind a TLS-terminating reverse proxy (direct
TLS + origin certificate pinning — no tunnel). Pick a template from
`server/deploy/`:

- **nginx** (`whisper-relay.nginx.conf`) — TLS 1.2/1.3, HSTS, WSS upgrade
  headers; certificates via certbot
- **Caddy** (`Caddyfile`) — automatic Let's Encrypt, zero config

Then set `WHISPER_TRUSTED_PROXIES=127.0.0.1` in `/etc/whisper/relay.env` so
rate limiting reads the real client IP from the forwarded headers.

### 4. Desktop app

```sh
cd desktop
npm install
npm run build        # frontend: tsc + vite build
cargo check          # Tauri shell compiles against the shared core
```

### 5. Smoke test (relay must be running)

```sh
cd server
node tests/smoke.mjs   # expects the relay on ws://127.0.0.1:8080/ws
```

### 6. Workspace tests

```sh
# Full workspace (Linux/macOS — includes the Tauri desktop crate)
cargo test --workspace

# Windows without MSVC Build Tools: test the Rust crates only
cargo test -p e2ee-core -p whisper-relay

# Lint & format
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

> **Windows note:** `cargo test --workspace` requires MSVC Build Tools +
> Windows SDK because of Tauri/WebView2 linking. Until they are installed, run
> `cargo test -p e2ee-core -p whisper-relay` to test the core and relay.

---

## Repository Layout

```
├── e2ee-core/        # Shared crypto core: identity, prekeys, X3DH, Double Ratchet, wire protocol
├── server/           # whisper-relay: zero-knowledge blind relay (axum + tokio + SQLite)
│   ├── deploy/       #   hardened systemd unit + nginx/Caddy reverse-proxy templates
│   └── tests/        #   smoke.mjs end-to-end tests
├── desktop/          # Tauri v2 desktop client (React + TypeScript + Tailwind)
├── docs/             # ROADMAP.md and other technical documentation
└── .github/          # GitHub Actions CI workflows
```

---

## Testing & TDD

- **350 unit tests** across the workspace (e2ee-core 88, whisper-desktop 112,
  whisper-relay 150)
- **92 smoke tests** covering live routing, offline delivery, SQLite
  persistence, `fetch_since` sync, rate limiting and signed-hello spoofing
  protection
- **TDD policy:** every crypto change requires tests before merge — tests
  first, or in the same commit. No tests, no merge.

---

## License

Licensed under the **MIT License**. Cryptography is provided by
[vodozemac](https://github.com/matrix-org/vodozemac), which is licensed under
**Apache-2.0**.

---

## Roadmap

**MVP (phases 0–5):** workspace + CI, crypto core, relay, desktop shell, E2EE
1:1 integration, polished UI/UX.

**Beyond MVP (done):** groups (owner/admin roles, multi-sender, ownership
transfer, invites + join links, rename, read receipts, system messages), emoji
reactions, quoted replies, safety numbers + QR, invite/join deep links, i18n
(EN/FI), notifications, presence and privacy controls, daily log files and
automatic backups.

**Next up:** relay deployment to a real VPS (two-machine E2EE test —
the last remaining release blocker), chat export, media/calls, mobile
(Flutter), external audit + post-quantum (X25519Kyber768).

See [docs/ROADMAP.md](docs/ROADMAP.md) for the full technical roadmap.

---

*Whisper — "I've got this." 🔒*
