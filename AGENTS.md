# AGENTS.md — Whisper

Shared context document for AI assistants (agents) working in this repository.
Technical terms are in English.

## Project description

Whisper is a privacy-first, end-to-end-encrypted (E2EE) chat — a
WhatsApp/Signal/Telegram replacement. The architecture is a **zero-knowledge
blind relay**: the server only ever sees opaque, encrypted envelopes — never
plaintext, keys or message content.

| Directory | Crate / role | Notes |
|---|---|---|
| `e2ee-core/` | `e2ee-core` — shared crypto core | vodozemac: X3DH + Double Ratchet, Megolm groups, signed usernames, receipts |
| `server/` | `whisper-relay` — blind relay | axum + tokio + SQLite; routed by peer ID, fan-out to group members |
| `desktop/` | `whisper-desktop` — Tauri v2 + React/TS client | Rust core + React UI; encrypted local history (SQLCipher-capable store) |
| `docs/` | — | ROADMAP.md, PROFILE-SYSTEM.md (username/avatar spec) |
| `scripts/` | — | `run-smoke.sh` (CI smoke wrapper) |
| `.agents/skills/` | local skills | frontend-design, rust-best-practices, accessibility, seo — read and follow the relevant one |

Core architectural principle: **the server is zero-knowledge** — it can only
relay and store encrypted envelopes. Even group encryption keys are shared
between members over 1:1 E2EE, never via the server.

## Repository layout (source modules)

```
e2ee-core/src/   identity.rs  prekey.rs  session.rs  group.rs  profile.rs  wire.rs
server/src/      main.rs  relay.rs(core: socket, hello, routing)  store.rs
                 + groups.rs  profiles.rs  presence.rs  prekeys.rs  ratelimit.rs
desktop/src-tauri/src/  main.rs  lib.rs(commands)  relay.rs(core)  store.rs
                 + relay_groups.rs  relay_profiles.rs  relay_presence.rs  relay_settings.rs
desktop/src/     components/  components/settings/  hooks/  lib/  types.ts
```

Keep large handlers in their dedicated modules (impl blocks over
`#[path]`-declared siblings); do not let `relay.rs` files balloon again.

## Technical rules

1. Code comments ALWAYS in **English**. Finnish comments are not allowed
   (README and documentation language is a separate matter).
2. **No hand-rolled cryptography** — all cryptography comes from the
   [vodozemac](https://github.com/matrix-org/vodozemac) library (X3DH,
   Double Ratchet, Megolm). Own crypto primitives, own protocol versions or
   "improvements" to existing algorithms are not accepted.
3. **TDD**: all crypto changes require tests before merge. Tests are written
   first, or at minimum in the same commit as the code.
4. **"If it works, leave it alone"** — do not refactor working code without a
   specific reason. Avoid unnecessary cosmetic changes.
5. **Server-specific files are never staged to GitHub**: `server/data/`
   (runtime data), `server/data/media/` (avatars) and `.env` (secrets) are
   gitignored and must never be added to repository changes.
6. **Local test artifacts never staged**: `identity2.json`, `profiles2.json`,
   `sessions2.json` (second-instance test files) are gitignored.
7. **Code hardening**: release builds always use the workspace profile
   (`lto="fat"`, `panic="abort"`, `strip=true` — configured in the root
   Cargo.toml). Do not add debug symbols, do not switch `panic="unwind"` in
   production, and never expose secrets or keys to the UI/JS layer.
8. **Wire compatibility**: never change existing serde wire formats, error
   codes, event payloads or the `ChatState` UI contract without updating every
   consumer (server, desktop, smoke tests) in the same change.

## Pipeline division of labor

Work is done systematically in the following order:

1. **Planner** — plans the task, breaks it into subtasks.
2. **Coder(s)** — implement code. Max **3 coders in parallel** (no more, to
   avoid conflicts in the same codebase).
3. **Tester** — runs the tests and verifies the changes work.
4. **Reviewer** — reviews the code (security, style, rules).
5. **Reporter** — reports the result to the user.
6. The user **approves** the result.
7. **Push** — changes are pushed to GitHub.

Each role waits for the previous one to finish; parallel coding is capped at 3.

## Quick reference for commands

```sh
# Run the whole workspace's tests (277 unit tests as of 2026-08-05:
# e2ee-core 48, whisper-desktop 89, whisper-relay 140)
cargo test --workspace

# Windows note: desktop requires the MSVC toolchain (rustup default
# stable-x86_64-pc-windows-msvc + VS Build Tools). If linking fails, use:
cargo test -p e2ee-core -p whisper-relay

# Lint + reject warnings
cargo clippy --workspace --all-targets -- -D warnings

# Formatting check
cargo fmt --check

# Smoke suite (67 checks) — starts a fresh relay automatically:
bash scripts/run-smoke.sh

# Manual smoke: start the relay, then run the tests
cd server && cargo run          # terminal 1
cd server && node tests/smoke.mjs   # terminal 2

# Run two desktop instances side by side (E2EE test between windows)
cd desktop && npm run tauri:dev          # window 1
cd desktop && npm run tauri:dev:second   # window 2 (own identity, port 1421)
```

## Privacy & security invariants

- Read receipts, typing and presence are best-effort; users can disable
  sending them (privacy settings). Hiding online status is server-enforced.
- The relay stores usernames as signed bindings (Ed25519 over
  `username || 0x00 || curve25519_key`) and re-verifies on every registration.
- Never log message content; logs are peer-ID level only.
