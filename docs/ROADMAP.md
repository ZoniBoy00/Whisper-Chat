# Whisper (Whisper-Chat) — Technical Roadmap

> Whisper — "a whisper". Messages are whispers: only the sender and the
> recipient can hear them. Privacy-first, end-to-end-encrypted (E2EE)
> general-purpose chat — a WhatsApp/Signal/Telegram replacement without
> backdoors or scanning mechanisms.
>
> **Date:** 2026-08-05 (updated: branding, product vision, hardening, deployment security)
> **Status:** Planning (Phase 1) — relay + crypto core coded and tested
> **Origin:** App specification + Gemini cross-check + Byte evaluation
> **Working title (before branding):** Operation Ghost

---

## 1. Goal and Product Vision

**A general-purpose, easy, modern chat** that replaces WhatsApp, Signal and
Telegram — not a technical toy. Uncompromising E2EE everywhere: messages,
groups, media, calls.

| Feature | Goal |
|---|---|
| **Easy onboarding** | Install → key pair generated automatically → share your ID as an invite link. No phone number, no email, no accounts. |
| **Modern UI** | Clean, minimalist, Signal/Telegram-grade. Dark theme, Lucide icons, fast and responsive. |
| **Everything E2EE** | 1:1 messages, groups, media, files, voice and video calls. The server never sees any content. |
| **Secure by default** | No opt-in security. Verified contacts (safety numbers), disappearing messages. |
| **Light & fast** | Desktop < 20 MB, battery-friendly mobile, fast sync. |
| **Resilient to regulation** | The architecture makes content scanning technically impossible (there is no "single switch" to regulate). |

**Metadata model (decision):** no Tor relay, no onion routing, no mixnet. The
server sees routing information (who talks to whom + IP) — **exactly like
Signal and WhatsApp**. Privacy comes from E2EE, not from hiding the network.
This is an accepted and honest model for the average user.

## 2. Core Principles

| Principle | What it means in practice |
|---|---|
| **Zero-Knowledge (honestly)** | E2EE protects the **content**. The server sees only encrypted envelopes, peer IDs and traffic volumes. Metadata on the server exists (routing requires it) — documented honestly. |
| **Zero-Trust** | No single component can leak conversations. Every layer is assumed hostile. |
| **No hand-rolled crypto** | All cryptography comes from a proven, audited library (`vodozemac`). Own crypto is a guaranteed disaster. |
| **Ease must not break security** | Secure by default; security never requires technical skill from the user. |
| **Light & fast** | Desktop binary < 20 MB, memory usage a fraction of Electron, battery-friendly mobile. |
| **No backdoors** | No server-side scanning, no E2EE workarounds, no telemetry. |
| **Scope discipline** | MVP = Desktop + relay + 1:1 E2EE. Groups, media, calls and mobile are out of MVP scope. |

**Why this withstands Chat-Control-style regulation?**
Chat Control / client-side scanning requires a scanning mechanism in the
client app. Because the Whisper client **cannot** see content (keys live in
the Double Ratchet session, messages are encrypted before being written to
disk), there is no "single switch" to regulate. Content scanning is not
possible without rewriting the entire application.

---

## 3. Locked Decisions

| # | Decision |
|---|---|
| 1 | 🛑 **No hand-rolled Double Ratchet** → **`vodozemac`** (Apache-2.0, audited). `libsignal` (AGPL) rejected. |
| 2 | **No onion/Tor** → Signal/WhatsApp model: E2EE for content, server sees routing. |
| 3 | **Scope discipline** → MVP = Desktop + relay + 1:1 E2EE. |
| 4 | **"Zero-knowledge" defined honestly** → content protected, metadata exists. |
| 5 | **Name:** **Whisper** (working title Operation Ghost). Repo `ZoniBoy00/Whisper-Chat`, MIT license. |

---

## 4. Tech Stack

**Core principle: one shared Rust core (`e2ee-core`) used by** the Tauri
desktop (natively), Flutter mobile (`flutter_rust_bridge`) and the tests. The
crypto logic is written **once**, tested **once**, shared everywhere.

| Layer | Choice | Rationale |
|---|---|---|
| **Shared core** | Rust crate **`e2ee-core`** | Same crypto/protocol for every platform |
| **Cryptography** | **vodozemac** (X3DH + Double Ratchet) | Apache-2.0, audited, no hand-rolled crypto |
| **Relay/backend** | Rust + **axum + tokio + WebSocket** | Light, fast, memory-safe |
| **Server DB** | **SQLite** (ciphertext envelopes + prekeys only, TTL) | No unencrypted data ever written to disk |
| **Desktop** | **Tauri v2** (Rust core + React/TS) | ~10 MB vs Electron 100+ MB |
| **UI** | **React + TypeScript + Tailwind + shadcn/ui + Lucide icons** | Modern, clean, fast to develop, dark theme |
| **Local DB (client)** | rusqlite + **SQLCipher** | Encrypted local history |
| **Mobile (phase 8)** | **Flutter + flutter_rust_bridge** | One codebase for iOS+Android, same Rust core |
| **Calls (phase 7)** | WebRTC (DTLS-SRTP) + coturn (STUN/TURN) | P2P, encrypted media |
| **Transport** | TLS 1.3 + certificate pinning | Protects traffic end to end to the server |
| **Deploy** | Hetzner VPS + systemd (hardened), GitHub Actions CI | Existing infrastructure |
| **Testing** | `cargo test` (TDD), `clippy -D warnings`, `proptest`, `cargo audit` | Crypto without tests = no merge |

### Why not X?

| Alternative | Why not |
|---|---|
| Electron | 100+ MB binary, 200+ MB RAM. Breaks the "light and fast" value. |
| React Native | JS bridge slows down crypto, two dependency ecosystems. |
| Go backend | Valid, but Rust delivers performance **and** memory safety + a shared crypto core. |
| libsignal (AGPL) | AGPL forces the whole project under the same license. |

---

## 5. Cryptographic Plan

### Identity (no personal data)
- **Key pair:** X25519 (key exchange) + Ed25519 (signatures).
- **Peer ID:** hash of the identity public key (SHA-256 → 24 hex = 96-bit, collision-safe at planetary scale) — no phone
  number, no name.
- **Safety Numbers:** contact verification via QR code / digit sequence (like
  Signal). No trust in the server.

### Session setup (X3DH) and messages (Double Ratchet)
1. Alice fetches Bob's **prekey bundle** (signed, tamper-protected — see `e2ee-core/src/prekey.rs`).
2. X3DH handshake → root key → **Double Ratchet session**.
3. Messages are encrypted by **vodozemac** — the cipher is defined by the
   Olm specification (AES-256-CBC with HMAC-SHA256), not a free choice of
   ours; keys are derived with HKDF-SHA256. Full forward & backward secrecy.
4. **Library:** `vodozemac` (LOCKED). **TDD requirement:** crypto change
   without tests = no merge.

### Groups (phase 6) & Post-Quantum (phase 9+)
- Groups: **Sender Keys / Megolm**. Future: MLS (RFC 9420).
- PQ: **X25519Kyber768** hybrid — "harvest now, decrypt later" protection. Not
  in v1, but room is left for it.

---

## 6. Architecture

### 6.1 Diagram

```
┌─────────────┐     WebSocket (TLS 1.3)     ┌──────────────────┐
│  Tauri App  │ ───────────────────────────▶ │  Whisper Relay   │
│  (Rust+TS)  │ ◀─────────────────────────── │  (axum + tokio)  │
└─────┬───────┘      ciphertext only         └────────┬─────────┘
      │                                              │
      │ e2ee-core (Rust)                             │ SQLite
      │  ├─ vodozemac (X3DH+Ratchet)                │ (envelopes,
      │  ├─ wire-protocol (serde, versioned)         │  prekeys,
      │  └─ local store (SQLCipher)                  │  users)
      └──────────────────────────────────────────────┘
```

### 6.2 Message lifecycle
1. The sender creates/updates a Double Ratchet session with the recipient's prekey.
2. The message is encrypted → ciphertext envelope.
3. The envelope goes to the relay → stored (SQLite, TTL 7 days) + forwarded.
4. The recipient unwraps the session → **the server sees nothing**.

### 6.3 The server's role & metadata (accepted model)

The server sees only `{ sender, recipient, payload: <opaque ciphertext>, seq }`. No
plaintext message history, no profiles, no analytics. Routing information (who
talks to whom, IP) is visible just as in Signal/WhatsApp — this is an
**accepted product choice**, not a shortcoming.

| Level | Whisper (v1) |
|---|---|
| Message content | 🔒 E2EE (Double Ratchet) |
| Routing | ⚠️ Server sees peer IDs (routing requires it) — like Signal |
| IP addresses | ⚠️ Server sees IPs (TLS protects them in transit) |
| Traffic patterns | ⚠️ Visible → light padding (optional later) |

---

## 7. Development Phases

> Each phase = one pipeline run (Planner → Coder(s) → Tester → Reviewer → Reporter).
> Integration tests in between.

| Phase | Content | Agents | Status |
|---|---|---|---|
| **0** | Repo + workspace, CI, AGENTS.md | 1 | ✅ **done** |
| **1** | **Crypto core `e2ee-core`** (vodozemac: identity, prekeys, X3DH, ratchet, wire v1, signed hello) | 2 | ✅ **done** |
| **2** | **Relay server** (WebSocket, SQLite offline queue, `fetch_since`, rate limiting, signed hello + spoofing protection, prekeys, display names, presence watch) | 1 | ✅ **done** |
| **3** | **Desktop shell:** Tauri v2 + React/TS — onboarding, contact view, chat view, settings, themes, splashscreen | 1 | ✅ **done** |
| **4** | **E2EE integration:** 1:1 messages end to end (prekey exchange → session → encryption → relay → decryption), session persistence | 1–2 | ✅ **done** |
| **5** | **UI/UX:** Signal/Telegram-grade — read receipts (blue ticks), typing indicator, display names, Online/Last seen presence, WhatsApp-style chat list, settings | 1–2 | ✅ **done** |
| **5.5** | **Username & Profile System:** signed username binding (Ed25519), avatars via /media, search by username/UID | 2 | ✅ **done** — spec: `docs/PROFILE-SYSTEM.md` |
| **6** | **Groups:** Megolm E2EE groups — owner/admin roles, key sharing over 1:1 E2EE, WhatsApp/Signal-style group chat + info UI | 2 | ✅ **done** |
| **6.5** | **Disappearing messages:** per-chat TTL, auto-delete both ends | 1–2 | 🔒 Next |
| **7** | **Media + calls:** encrypted file transfer, WebRTC (DTLS-SRTP) + coturn | 2 | 🔒 After MVP |
| **8** | **Mobile:** Flutter + flutter_rust_bridge; push (APNs/FCM — only "you have a message") | separate | 🔒 After MVP |
| **9** | **Audit + PQ:** cargo audit, fuzz, external review, threat model, X25519Kyber768 | — | 🔒 After MVP |

**MVP = phases 0–5.** Realistic timeline with pipeline work: **2–4 weeks**.

**Security note (dependencies):** `glib` is locked at 0.18.5 by `gtk 0.18`
(pinned by tauri 2.11). The Dependabot advisory (VariantStrIter UB in
glib <0.20, Linux-only) is **blocked upstream** — 0.20 cannot be selected
without breaking the Linux build. Revisit when tauri moves to gtk 0.19+.
Tracked/ignored in `.github/dependabot.yml`.

---

## 8. Code Hardening (anti-debug / anti-reverse)

Rust is already considerably harder to reverse engineer than JS/Python. Goal:
**raise the bar high enough that nobody bothers** — 100% protection does not
exist and is not promised.

| Technique | Where | Status |
|---|---|---|
| **Release profiles:** `opt-level=3`, `lto="fat"`, `codegen-units=1`, `panic="abort"`, `strip=true` | workspace Cargo.toml | ✅ done |
| **No debug symbols in production** | release builds | ✅ (strip) |
| **Crypto only in Rust** (never in the JS layer) | e2ee-core | ✅ in architecture |
| **UI without secrets:** the React layer never handles keys — only encrypted blobs | desktop | ⏳ to be confirmed in phase 3 |
| **Mobile hardening:** Android R8/ProGuard + native strip, iOS strip + code signing | mobile | 🔒 phase 8 |
| **Integrity/tamper-evidence** (binary integrity check, discretionary) | desktop | 🔒 phase 9 |
| **Cargo audit + dependency discipline** (minimized dependencies = smaller attack surface) | whole project | ⏳ ongoing |

**Rule for agents:** release builds always with stripped symbols; `panic="abort"`
must not be "unwind" in production; secrets never go into the JS/UI layer.

---

## 9. Server Security (deploy)

| Measure | Description |
|---|---|
| **Dedicated user without sudo** | The service runs as the `whisper` user, never root |
| **systemd hardening** | `NoNewPrivileges`, `ProtectSystem=strict`, `PrivateTmp`, `ProtectHome`, empty `CapabilityBoundingSet` — unit template in the repo (`server/deploy/`) |
| **Only required ports open** | ufw: 443 (and 80 redirect) or Cloudflare Tunnel; SSH key-only + fail2ban |
| **TLS 1.3** | Certbot/Let's Encrypt, HSTS, certificate pinning in the client |
| **Rate limiting + size limits** | ✅ done (token bucket 60/min/IP, 8 MiB envelope cap) |
| **No message-content logs** | Logs only at peer-ID level, log rotation, `RUST_LOG=info` |
| **Secrets** | `.env` + `WHISPER_DB_PATH`; `.env.example` documented, never committed to the repo |
| **Updates** | Automatic security updates (unattended-upgrades), `cargo audit` in CI |
| **Database** | Ciphertext envelopes only, TTL 7 days, 500/peer cap, `server/data/` gitignored |

---

## 10. Multi-Agent Division of Labor

| Agent | Where | Role |
|---|---|---|
| **OpenCode + Agnes** | Joni's PC | Lead coder — feature lanes |
| **Byte (me)** | VPS/Hermes | Planning, architecture, tests, review, infra, deploy |
| **OpenCode (2nd instance)** | Joni's PC | Parallel feature lane / review |

**Per phase:** Joni → "go" → bite-sized tasks → parallel coders (max 3) →
tester → reviewer → report → approval → push.

---

## 11. Infra & Repo

- **Relay:** Hetzner VPS + systemd (`whisper-relay.service`), Cloudflare
  Tunnel or direct port.
- **CI:** GitHub Actions — `cargo test --workspace`, `clippy -D warnings`,
  `fmt` on every push.
- **Repo:** `ZoniBoy00/Whisper-Chat` (private initially, public later). **License: MIT.**
- **Naming:** brand **Whisper**; technical names `e2ee-core` (core) and
  `whisper-relay` (relay). Rename done 2026-08-05.

---

## 12. Risks & Realism

| Risk | Mitigation |
|---|---|
| Crypto is hard | vodozemac + TDD + review + audit |
| Metadata visible to the server | **Accepted product choice** (Signal model) — documented honestly |
| Scope creep | Phases 6–8 kept out of the MVP |
| DoS / spam | Rate limiting, size cap, signed hello (next), TTL |
| Store rules (scanning pressure) | Clean architecture; E2EE protected by law (EU) |
| Anti-reverse is not absolute | The bar is raised, not the impossible promised |

**Open questions:**
1. Web (browser) support? (WASM possible — not in MVP)
2. Desktop push notifications in the MVP?

---

## 13. Status & Next Steps (TODO)

**✅ Done (2026-08-05):**
- [x] Phase 0: workspace, CI, AGENTS.md, .gitignore
- [x] Phase 1: `e2ee-core` (vodozemac) — 23/23 tests
- [x] Phase 2: relay + SQLite + fetch_since + rate limit + signed hello (spoofing protection) — 25/25 tests, smoke 19/19
- [x] Code hardening: release profiles, systemd template, .env.example
- [x] Rename `ghost-relay` → `whisper-relay` (crate, `WHISPER_*` env vars, deploy unit)

**⏳ Next up:**
- [ ] Install MSVC Build Tools + Windows SDK (for Windows linking — `cargo test --workspace` locally)
- [ ] Phase 4: E2EE integration (prekey exchange → session → encryption → relay → decryption in the desktop)

---

*Whisper — "I've got this." 🔒*
