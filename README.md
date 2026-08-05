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
- **Zero-knowledge blind relay** — the server only ever sees opaque, encrypted
  envelopes. It holds zero plaintext, zero keys and zero message content.
- **No phone number required** — identity is a pure cryptographic key pair.
  Share your peer ID as an invite link. No accounts, no emails.
- **Signed hello / spoofing protection** — connections authenticate with a
  self-signed hello; spoofed sender IDs are rejected (`sender_mismatch`,
  `identity_conflict`, `invalid_hello`).
- **Offline delivery** — SQLite-backed offline queue with a 7-day TTL and
  `fetch_since` sync, so you never miss a message while away.
- **Rate limiting & DoS guards** — per-IP token bucket (60/min default) and an
  8 MiB envelope size cap.
- **Modern dark UI** — clean, minimalist, Signal/Telegram-grade desktop shell
  (Tauri v2 + React + TypeScript + Tailwind + Lucide icons).
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
| `RUST_LOG`          | `info`           | Log level                            |

### 3. Deploy with systemd

A hardened, production-ready unit template is included at
`server/deploy/whisper-relay.service` — it runs as a dedicated non-root user
with `NoNewPrivileges`, `ProtectSystem=strict`, `PrivateTmp`, an empty
`CapabilityBoundingSet` and a read-only root filesystem.

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
│   ├── deploy/       #   hardened systemd unit template
│   └── tests/        #   smoke.mjs end-to-end tests
├── desktop/          # Tauri v2 desktop client (React + TypeScript + Tailwind)
├── docs/             # ROADMAP.md and other technical documentation
└── .github/          # GitHub Actions CI workflows
```

---

## Testing & TDD

- **48+ unit tests** across the crypto core (23) and the relay (25)
- **19+ smoke tests** covering live routing, offline delivery, SQLite
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

**After MVP (phases 6–9):** groups + disappearing messages (Megolm), encrypted
media + calls (WebRTC/DTLS-SRTP), Flutter mobile, external audit +
post-quantum (X25519Kyber768).

See [docs/ROADMAP.md](docs/ROADMAP.md) for the full technical roadmap.

---

*Whisper — "I've got this." 🔒*
