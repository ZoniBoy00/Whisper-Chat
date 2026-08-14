# Whisper (Whisper-Chat) — Technical Roadmap

> Whisper — "a whisper". Messages are whispers: only the sender and the
> recipient can hear them. Privacy-first, end-to-end-encrypted (E2EE)
> general-purpose chat — a WhatsApp/Signal/Telegram replacement without
> backdoors or scanning mechanisms.
>
> **Date:** 2026-08-09 (updated: **E2EE backup encryption DONE** — password-sealed backups with Argon2id → AES-256-GCM, passwordless autobackup blocked; **one release blocker left:** real two-machine E2EE test on Hetzner)
> **Status:** Phases 0–6.10 done — full local MVP. **One release blocker left:** real two-machine E2EE test on Hetzner (E2EE backup encryption — the previous 🔴 blocker — fixed 2026-08-09)
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
| **6** | **Groups:** Megolm E2EE groups — owner/admin roles, multi-sender (every member sends with an own Megolm session), ownership transfer, key sharing over 1:1 E2EE, WhatsApp/Signal-style group chat + info UI | 2 | ✅ **done** |
| **6.5** | **Disappearing messages:** per-chat TTL (off/5s/30s/1m/1h/1d), E2EE TTL inside the encrypted payload, auto-delete both ends + live countdown | 1–2 | ✅ **done** |
| **6.6** | **i18n + notifications:** EN/FI translations, native notifications + sounds, unread badges, pinned chats, in-chat message search, day separators, context menus, auto-reconnect | 1–2 | ✅ **done** |
| **6.7** | **Group multi-sender:** every member sends with their own outbound Megolm session (created on first group-key receipt); connection toasts | 1 | ✅ **done** |
| **6.8** | **Friend system (anti-spam):** signed friend requests + accept/decline/remove, server-enforced `not_contacts` gating (1:1 envelopes, pre-keys, group member adds), Contacts tab with live Online / Last seen + one-click removal | 2 | ✅ **done** |
| **6.9** | **Message interactions:** emoji reactions (E2EE state-signal envelopes, reaction pills on bubbles, shared ReactionPicker — context menu + quick-react button), quoted replies (tagged plaintext payload, reply bar in the composer) | 2 | ✅ **done** |
| **6.10** | **Invite links + safety numbers + join links:** `whisper://invite` (share, profile popup, OS deep links via single-instance + deep-link plugins) and `whisper://join` (group join links with secret token, name + avatar embedded, join dialog), Signal-style safety numbers (60-digit + short tag, QR code, local verified flag) | 1–2 | ✅ **done** |
| **6.11** | **Multi-device sync:** one identity on several devices (Signal-style key backup / signed device list) | 2 | 🔒 After MVP |
| **7** | **Media + calls:** encrypted file transfer (AES-GCM key exchange, `/media` extension) — design in [`docs/MEDIA-SYSTEM.md`](MEDIA-SYSTEM.md), WebRTC (DTLS-SRTP) + coturn, voice messages | 2 | 🔒 After MVP |
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
| **UI without secrets:** the React layer never handles keys — only encrypted blobs | desktop | ✅ done (architecture: all crypto lives in e2ee-core, the UI only serializes/renders) |
| **Mobile hardening:** Android R8/ProGuard + native strip, iOS strip + code signing | mobile | 🔒 phase 8 |
| **Integrity/tamper-evidence** (binary integrity check, discretionary) | desktop | 🔒 phase 9 |
| **Cargo audit + dependency discipline** (minimized dependencies = smaller attack surface) | whole project | ✅ done (audit job in CI — `rustsec/audit-check@v2.0.0`) |

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

## 10. Infra & Repo

- **Relay:** Hetzner VPS + systemd (`whisper-relay.service`), Cloudflare
  Tunnel or direct port.
- **CI:** GitHub Actions — `cargo test --workspace`, `clippy -D warnings`,
  `fmt` on every push.
- **Repo:** `ZoniBoy00/Whisper-Chat` (private initially, public later). **License: MIT.**
- **Naming:** brand **Whisper**; technical names `e2ee-core` (core) and
  `whisper-relay` (relay). Rename done 2026-08-05.

---

## 11. Risks & Realism

| Risk | Mitigation |
|---|---|
| Crypto is hard | vodozemac + TDD + review + audit |
| Metadata visible to the server | **Accepted product choice** (Signal model) — documented honestly |
| Scope creep | Phases 6–8 kept out of the MVP |
| DoS / spam | Rate limiting, size cap, signed hello ✅, `not_contacts` gating ✅, TTL |
| Store rules (scanning pressure) | Clean architecture; E2EE protected by law (EU) |
| Anti-reverse is not absolute | The bar is raised, not the impossible promised |

**Open questions:**
1. Web (browser) support? (WASM possible — not in MVP)
2. Desktop push notifications in the MVP?

---

## 12. Future Features (Idea Backlog)

A running wishlist of WhatsApp/Signal/Telegram-style features, prioritized by
impact and effort. Pull items into the phase table when they get scheduled.

| Priority | Feature | Notes |
|---|---|---|
| ✅ Done | **Emoji reactions** | E2EE state-signal envelopes, pills on bubbles, shared ReactionPicker (context menu + quick-react button) — 1:1 and groups |
| ✅ Done | **Quoted replies** | tagged plaintext payload, composer reply bar, quoted bubble rendering |
| ✅ Done | **Invite links + join links** | `whisper://invite` (profile preview popup) + `whisper://join` (group links with secret token, name + avatar in the link, join dialog) |
| ✅ Done | **Safety number verification** | 60-digit + short tag + QR code + local verified flag (no server trust) |
| ✅ Done | **Disappearing messages** | per-chat timer (off/5s/30s/1m/1h/1d), E2EE TTL, auto-delete both ends + live countdown |
| ✅ Done | **Message editing** | E2EE edit envelope; composer edit bar; "(edited)" marker |
| ✅ Done | **Delete for everyone** | E2EE delete envelope; local copy + recipients purge |
| 🔥 High | **Media: images & files** | `docs/MEDIA-SYSTEM.md` is ready — AES-GCM key exchange + `/media` extension; image sharing is ~80% of what people do on a messenger, its absence makes the app feel unfinished |
| 🔥 High | **Voice messages** | opus/WebM chunks over the encrypted media channel — top requested feature; built on the media channel above (no media server needed, just encrypted chunks) |
| 🔥 High | **Chat export** | Signal-style plaintext/JSON export of a conversation — "your data is yours" + a GDPR answer; small effort since the backup package machinery already exists |
| 💪 Medium | **Mute conversations** | per-chat notification mute (15m/1h/8h/forever) |
| 💪 Medium | **Archived chats** | fold old conversations out of the list (unarchive on new message) |
| 💪 Medium | **Chat backgrounds/themes** | per-chat wallpaper + accent (client-side only) |
| ✅ Done | **Message font scale** | small/normal/large setting drives `data-message-scale` → `.wp-msg` bubble font (13/15/17px) |
| 💪 Medium | **Device list + revoke** | Signal-style session health report (verify/revoke actions) — **prerequisite for multi-device (6.11): do it before, not after** |
| 🧠 Nice-to-have | **Status/Stories** | ephemeral 24h photo/text status (WhatsApp-style) |
| 🧠 Nice-to-have | **Pinned messages in groups** | admin pins a message to the top |
| 🧠 Nice-to-have | **Group admin controls** | only-admins-can-send, group name/photo lock, disband group |
| 🧠 Nice-to-have | **Global message search** | per-chat search exists — extend across conversations |
| 🧠 Nice-to-have | **Spell-check / autocorrect** | client-side only (never sent) |
| 🧠 Nice-to-have | **Message padding** | random-size padding to flatten traffic patterns (metadata) |
| 🧠 Nice-to-have | **Web client (WASM)** | e2ee-core → wasm-bindgen; read-only companion or full client — **deliberately deferred**: browser crypto = JS-held keys = larger XSS surface; the Tauri desktop is the differentiator |
| 🧠 Nice-to-have | **Testnet mode** | a shared demo relay for trying the app without self-hosting |
| 🔒 Security | **E2EE PIN backup (phase 2)** | Beyond the shipped E2EE backup encryption (§13 Next up): split identity and data — password-protected "vault" (DB + sessions) + separate Signal-style recovery key for the identity; or derive the DB key from a password/PIN instead of the identity so a leaked identity alone opens nothing |
| 🧠 Nice-to-have | **Multi-account / profile switcher** | one install, several identities (power-user feature) |
| 🧠 Nice-to-have | **Email / recovery-key onboarding** | "save your recovery key" flow for users who find a bare key confusing — keeps the no-phone-number/no-account selling point (no email accounts, no extra metadata) |

---

## 13. Status & Next Steps (TODO)

**✅ Done (2026-08-06):**
- [x] Phase 6.9 follow-up: message editing + delete-for-everyone — `EditPayload`/`DeletePayload` E2EE control envelopes (Double Ratchet 1:1 + Megolm groups), `edited` flag + store migration, context-menu "Edit" with a composer edit bar, "(edited)" marker, own-message guard (`MessageNotFound`), `chat-message-edited`/`chat-message-deleted` events
- [x] Phase 6.5: disappearing messages — per-chat timer (Off/5s/30s/1m/1h/1d) in the chat header, `expires_in_seconds` carried inside the encrypted `TextPayload` (the relay stays zero-knowledge), `messages.expires_at` + migration, 1s background purge loop, `chat-message-deleted` event, live countdown on both the sender's and the recipient's bubbles, `chat_settings` table persisting timers
- [x] Group read receipts: Megolm `ReadPayload` read-by sets, `mark_read` fires only when messages are actually visible on screen (never on receipt), `readByCount` in the UI
- [x] Group rename (`rename_group` wire) + WhatsApp-style "X joined/left the group" system messages + live member counts via `group-updated`
- [x] Daily log files: `whisper-YYYY-MM-DD.log` under `%APPDATA%/com.whisper.desktop/logs/`, local-time timestamps on client AND relay (LocalTimer), no ANSI colors in files, "Open logs folder" button in Settings
- [x] Automatic backups: autobackup toggle/directory/keep-count with a daily scheduler + "Back up now"; full export/import of identity + history as one JSON package
- [x] Restart-proof sessions: hydrated group keys are re-shared before the first group send (`key_shared_this_session`), group sessions persist after every Megolm decrypt, lazy 1:1 session heal — **plus the seq-seed fix**: envelope seqs now start from wall-clock millis so a restart never collides with the recipient's (sender, seq) dedupe set (previously every message of a fresh run was silently dropped as a "duplicate" until the counter passed the old peak — the "third message finally arrives" bug)
- [x] CSP (`default-src 'self'` + explicit img/ws/ipc exceptions) in `tauri.conf.json`
- [x] CI hardening: `rustsec/audit-check@v2.0.0`, workflow + audit `permissions` (contents: read, checks/issues: write)
- [x] Phase 6.9: emoji reactions (E2EE state-signal envelopes, pills + shared ReactionPicker for context menu AND quick-react button, viewport-clamped + portal-rendered) and quoted replies (tagged plaintext payload `{"kind":"text",..}`, composer reply bar) — both 1:1 and groups
- [x] Phase 6.10: invite links (`whisper://invite?peer=..` + name/user hints, share via clipboard with toasts, profile preview popup) and safety numbers (60-digit + 8-hex short tag via SHA-256 over sorted identity keys, QR code, local verified flag)
- [x] OS-level deep links: `tauri-plugin-deep-link` + `tauri-plugin-single-instance` — clicking a `whisper://` link in a browser opens Whisper with the invite/join pre-loaded (Windows: HKCU scheme registration; every dev instance registers its own exe so links reach the running dev server)
- [x] Group invites: `group_invite` / accept / decline wire protocol, Sidebar "Group invites" section, invitee sees group name + inviter, inviter learns the outcome
- [x] Group join links: `whisper://join?group=..&token=..&name=..&avatar=..` — any member can copy the link (Group info), the relay authorizes joins by a secret token, join dialog shows group name + photo
- [x] Group typing with names: Megolm typing payloads attributed to the writer — "ZoniBoy typing…" / "3 members typing…" in the group header
- [x] Live member counts: the relay fans `group_member_left`/`group_avatar_set` to all members and clients emit `group-updated` on roster changes
- [x] Group metadata persistence: new `group_meta` table keeps group names + avatars across restarts
- [x] Avatar self-heal: avatars retry when the relay (re)connects (img key bump forces a real re-request)
- [x] Toasts render inside dialogs (native `<dialog>` top layer) so feedback is never hidden behind Settings
- [x] Copy-toasts for Whisper ID / invite link / group join link
- [x] Bugfixes: group transfer wire field (`peer_id` → `new_owner_peer_id`, was `bad_message`), optimistic group messages render plain text (not the JSON payload), `already_member` routes to the join queue, message-id sharing makes reactions resolve across devices, picker transform-ancestor clipping

**✅ Done (2026-08-05):**
- [x] Phase 0: workspace, CI (test/clippy/fmt/smoke jobs), AGENTS.md, .gitignore
- [x] Phase 1: `e2ee-core` (vodozemac) — 48/48 tests
- [x] Phase 2: relay — SQLite offline queue, fetch_since, rate limiting, signed hello + spoofing protection, prekeys, display names, presence watch
- [x] Phase 5.5: username & profile system (signed bindings, avatars via /media, search by username/UID)
- [x] Phase 6: groups — owner/admin roles, multi-sender (own Megolm session per member), ownership transfer, group photos, member add/remove with pushes
- [x] Phase 6.6: i18n (EN/FI, 173 keys) + native notifications/sounds, unread badges, pinned chats, in-chat search, day separators, context menus, auto-reconnect, splash, toasts
- [x] Phase 6.7: group multi-sender — every member sends with an own outbound Megolm session
- [x] Phase 6.8: friend system — requests/accept/decline/remove, `not_contacts` gating (1:1, pre-keys, group adds), Contacts tab with Online/Last seen + removal
- [x] Desktop extras: settings expansion (tray, autostart, enter-to-send, font scale, identity backup/restore, clear history, test sound), log viewer (Logs tab), profile dialog, context menus, webview menu suppression
- [x] Relay ops: structured logging (`RUST_LOG`-controllable, peer-ID level only) + graceful shutdown on Ctrl+C/SIGTERM
- [x] Robustness fixes: FIFO→keyed request resolution (get_group_info), error-code→queue routing (stale groups evicted), legacy avatar sync, contact-only group cleanup

**Test counts (2026-08-14):** 350 unit tests — e2ee-core 88, whisper-relay 150
(trusted-proxy `proxy.rs` adds 9), whisper-desktop 112; smoke suite all
green. The E2EE backup fix adds 11
`backup.rs` crypto tests (roundtrip, wrong-password rejection, ciphertext /
KDF-cost / metadata tamper detection — the KDF params, version and nonce are
bound into the GCM **AAD** so weakening Argon2id costs in the file fails
decryption, legacy-format rejection, distinct-ciphertext property) plus 2
settings tests (password never serializes to the UI, patch set/clear).

**✅ Done (2026-08-09):**
- [x] 🔴 **E2EE backup encryption (release blocker — FIXED).** `build_backup_package` now seals the whole package (identity + encrypted DB + metadata) with **AES-256-GCM** under an **Argon2id**-derived key (19 MiB, t=2, p=1, fresh salt + nonce per backup) — the identity's private keys never leave the device in cleartext. The envelope's public metadata (kind, version, KDF params, nonce) is bound into the GCM **AAD**, so tampering with it — e.g. weakening the Argon2id costs to cheapen brute force — fails authentication; KDF costs are read from the file on restore (validated, `m_cost ≥ 8·p_cost`) so future cost bumps stay backward-compatible. **One password for everything**: `export_everything` takes `Option<String>` — an explicit password (min 8 chars) is validated AND persisted as the shared backup password, `None` reuses the stored one, so a manual "Backup everything" never asks again once a password exists; `import_everything` / `import_identity` require the backup password, a wrong password fails cleanly with **zero** disk changes; pre-v2 plaintext backups are **rejected** with a clear message. **Automatic backups are always password-encrypted**: a backup password is set once in Settings (write-only — `get_settings` never returns it, only `autobackup_password_set`), `write_autobackup` refuses to write without one, the scheduler logs the failure, and the UI blocks enabling autobackup (and "Back up now") until a password exists. All backup-password dialogs (export/import/set) live inside the General tab, so the `set` flag and the autobackup toggle refresh the moment a password is saved; password fields have a first-party Eye/EyeOff toggle and the WebView2 native reveal button is hidden via CSS (`::-ms-reveal`). New `backup.rs` module (pure crypto, no hand-rolled primitives — RustCrypto `aes-gcm` + `argon2`).

**✅ Done (2026-08-14):**
- [x] **Relay deploy package — proxy strategy decided: direct TLS + origin cert pinning (no Cloudflare Tunnel).** Through a tunnel the client sees Cloudflare's edge certificate instead of the origin's, so the §9 pinning rule only holds with direct TLS. Shipped `server/deploy/whisper-relay.nginx.conf` (nginx + certbot: TLS 1.2/1.3, HSTS, WSS upgrade headers, long WS timeouts, overwrites `X-Forwarded-For`) and `server/deploy/Caddyfile` (Caddy: automatic Let's Encrypt, same trust model) — the operator picks one.
- [x] **Relay: trusted-proxy IP handling.** New `server/src/proxy.rs` — `TrustedProxies` parses `WHISPER_TRUSTED_PROXIES` (comma/space-separated IP list; empty = safe default where no forwarded header is ever honored). `resolve_client_ip` recovers the real client from `CF-Connecting-IP` (Cloudflare) or the left-most valid `X-Forwarded-For` entry, but ONLY when the direct peer is in the trust list — a direct client can never spoof the header and bypass rate limiting. Documented in `.env.example`; 9 unit tests (relay suite now 150); `ws_handler` resolves the client IP before handing the socket to the relay.

**⏳ Next up:**
- [ ] Deploy the relay to Hetzner (systemd unit + nginx/Caddy template ready) → real two-machine E2EE test — blocks public release
- [ ] Relay: rate-limiter bucket GC — prune idle per-IP buckets so the in-memory map cannot grow unbounded over time
- [ ] Relay: optional `/metrics` endpoint (envelope counts, rate-limit hits, active WS connections) behind an env flag for light observability
- [ ] Chat export (Signal-style plaintext/JSON) — small, high-trust, GDPR-friendly
- [ ] Media: images & files (MEDIA-SYSTEM.md ready) → voice messages on the same channel
- [ ] Production hardening: binary integrity check (devtools already disabled in release; no console)
- [ ] Public repo (open source) once remote testing is solid
- [ ] E2EE PIN backup (phase 2 — follow-up to the shipped backup encryption): separate identity and data — password-protected vault (DB + sessions) + Signal-style recovery key, or derive the DB key from a password/PIN instead of the identity so a leaked identity alone opens nothing

**✅ Done (2026-08-06):**
- [x] Contact rehydration: new `list_contacts` wire command — the client fetches its accepted contacts from the relay on every connect and merges them into memory + store, so a database reset/restore/fresh install never loses the contact list again (relay is the source of truth for friendships)
- [x] CI `concurrency` group (per-branch, cancel-in-progress) so fast pushes do not queue stale runs
- [x] At-rest encryption: Windows client now builds with `bundled-sqlcipher-vendored-openssl` — the local history DB is SQLCipher-encrypted with an identity-derived key (statically linked; users need no extra installs). Build requires NASM + Perl on PATH (MSYS2). Breaking change: pre-existing plain-SQLite databases cannot be opened (keyed DBs only)
- [x] Megolm key rotation: group outbound sessions rotate to a fresh key every 200 messages; the new key is queued on the outbox BEFORE the triggering message so recipients swap inbounds in time; backward secrecy (a leaked key cannot decrypt post-rotation messages)
- [x] Phase 6.9 follow-up: message editing + delete-for-everyone — `EditPayload`/`DeletePayload` E2EE control envelopes (Double Ratchet 1:1 + Megolm groups), `edited` flag + store migration, context-menu "Edit" with a composer edit bar, "(edited)" marker, own-message guard (`MessageNotFound`), `chat-message-edited`/`chat-message-deleted` events

---

*Whisper — "I've got this." 🔒*
