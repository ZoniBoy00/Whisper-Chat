# AGENTS.md — Whisper

Shared context document for AI assistants (agents) working in this repository.
Technical terms are in English; the original annotations were in Finnish.

## Project description

Whisper (working title Operation Ghost) is a privacy-first, end-to-end-encrypted
(E2EE) chat — a WhatsApp/Signal/Telegram replacement.

- **`server/`** (crate `whisper-relay`): blind relay. It sees only encrypted
  envelopes — never plaintext or keys. The server has no access to message
  content.
- **`e2ee-core/`**: the shared crypto core. Built on the **vodozemac** library:
  X3DH key exchange + Double Ratchet message encryption. All crypto work
  happens here, never on the server.

Core architectural principle: **the server is zero-knowledge** — it can only
relay and store encrypted envelopes.

## Technical rules

1. Code comments ALWAYS in **English**. Finnish comments are not allowed
   (README and documentation language is a separate matter).
2. **No hand-rolled cryptography** — all cryptography comes from the
   [vodozemac](https://github.com/matrix-org/vodozemac) library (X3DH,
   Double Ratchet). Own crypto primitives, own protocol versions or
   "improvements" to existing algorithms are not accepted.
3. **TDD**: all crypto changes require tests before merge. Tests are written
   first, or at minimum in the same commit as the code.
4. **"If it works, leave it alone"** — do not refactor working code without a
   specific reason. Avoid unnecessary cosmetic changes.
5. **Server-specific files are never staged to GitHub**: `server/data/`
   (runtime data) and `.env` (secrets). These must be in `.gitignore` and
   never added to repository changes.
6. **Code hardening**: release builds always use the workspace profile
   (`lto="fat"`, `panic="abort"`, `strip=true` — configured in the root
   Cargo.toml). Do not add debug symbols, do not switch `panic="unwind"` in
   production, and never expose secrets or keys to the UI/JS layer.

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
# Run the whole workspace's tests
cargo test --workspace

# Lint + reject warnings
cargo clippy --workspace -- -D warnings

# Formatting check
cargo fmt --check

# Smoke test (requires the server to be running in a separate terminal):
cd server
cargo build
node tests/smoke.mjs
```
