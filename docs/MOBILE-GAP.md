# Whisper Mobile — Complete Gap List (desktop parity)

> Updated: 2026-08-14 (after commit a7aa542)
> Goal: make the Flutter mobile client a **full copy of the desktop version** —
> same look, same features, same UX. Everything below is missing or partial.

## 🔴 CRITICAL — must fix before anything else

| # | Item | Status |
|---|---|---|
| C1 | **Group messages are NOT truly E2EE** — `send_group_message` uses a placeholder XOR "cipher" (`xor_with_key` in whisper_core), not Megolm. The relay fans out ciphertext anyone who knows the group id + a peer id could decrypt. **Must implement Megolm key-sharing** (e2ee-core `OutboundGroup`/`InboundGroup`, share session key over 1:1 E2EE — desktop does this) | ❌ |
| C2 | **No auto-reconnect** — a dropped connection stays dead until manual reconnect. Desktop has a backoff loop (2s→30s) + "reconnecting" UI | ❌ |
| C3 | **Avatar upload needs a registered username first** — desktop allows avatar-only changes. Server `set_avatar` requires the username+signature; desktop re-asserts a persisted username. Mobile should persist its username locally and re-assert on connect | ⚠️ |
| C4 | **Whisper ID not shown as a proper PeerIdCard** — desktop PeerIdCard (copy + invite link + username). Mobile header is compact now but there is no invite-link share and no dedicated card in Settings | ⚠️ |

## 🟠 FEATURES — desktop parity (missing entirely)

| # | Feature | Notes |
|---|---|---|
| F1 | **Media: images & files** (MEDIA-SYSTEM.md is ready) — upload, download, render in bubbles, `/media/{hash}` avatars | ❌ |
| F2 | **Push notifications (FCM)** — "you have a message" only (roadmap §8). Requires a Firebase project (Joni's action) | ❌ |
| F3 | **Deep links** — open `whisper://invite` and `whisper://join` links (InvitePreviewDialog, JoinGroupDialog). Android intent-filter + parsing | ❌ |
| F4 | **Typing indicators** — send (on composer focus) + receive (show "typing…" in header/list) | ❌ |
| F5 | **Day separators** in chat (desktop groups messages by day) | ❌ |
| F6 | **In-chat message search** (desktop has a search field + highlighted matches) | ❌ |
| F7 | **Unread badges + pinned chats** (desktop: per-chat unread count, pin to top, both client-side persisted) | ❌ |
| F8 | **Clear history** (Settings → Privacy → "Clear message history") | ❌ |
| F9 | **Reaction pills under bubbles** (desktop shows grouped pills with counts + "mine" highlight, click to toggle) | ❌ |
| F10 | **Group avatar** (server supports `group_avatar_set`; mobile has no UI) | ❌ |
| F11 | **Group member management UI** — add/remove/promote/demote members from the group info screen (backend commands exist) | ❌ |
| F12 | **Verified flag for safety numbers** (desktop: mark a peer verified, persisted, shown in profile) | ❌ |
| F13 | **Notification sounds** + per-chat mute | ❌ |
| F14 | **Invite/join link copying from group info** (desktop has copy join link — mobile has the button but no paste/import flow) | ⚠️ |

## 🟡 UI / UX — look & feel parity

| # | Item | Status |
|---|---|---|
| U1 | **Real avatars everywhere** — desktop renders uploaded avatar images (chat list, bubbles, headers, profile). Mobile only shows letter avatars | ❌ |
| U2 | **Disappearing message live countdown** — desktop shows a ticking timer (45s, 1:30, 2h 5m) on the bubble + auto-purge UI | ⚠️ (icon only) |
| U3 | **Reaction picker inline** — desktop has a `SmilePlus` button on each bubble; mobile only via long-press | ⚠️ |
| U4 | **Composer extras** — emoji button, attach button (media), reply banner styling match | ⚠️ |
| U5 | **Bubble sender avatars** in group chats (desktop shows the sender's avatar beside incoming group messages) | ❌ |
| U6 | **Read-state ticks** — desktop: single tick (sent) → double (delivered) → blue (read). Mobile always shows double | ⚠️ |
| U7 | **Conversation row context menu** (desktop right-click: pin, mark read, profile, remove contact / leave group) | ❌ |
| U8 | **Exact desktop palette/spacing** — compare theme.dart against desktop `--wp-*` CSS: fonts (font-display), borders, shadows, hover states, radius | ⚠️ |
| U9 | **i18n completeness** — desktop has 1036 lines of translations; mobile covers only core strings. Translate every string | ❌ |
| U10 | **Onboarding avatar upload** (desktop onboarding sets display name + avatar) | ❌ |
| U11 | **Settings completeness** — autostart, backup-password set/reuse flow (BackupPasswordDialog), about version | ⚠️ |
| U12 | **Reconnect banner** ("Reconnecting… attempt N") when the connection drops | ❌ |
| U13 | **Logo component parity** — desktop Logo.tsx; mobile uses the asset directly (fine, but verify splash/header/sidebar match) | ⚠️ |

## 🟢 QUALITY / TECH

| # | Item | Status |
|---|---|---|
| Q1 | **Expand Flutter tests** — widget tests for onboarding, settings, chat view; backup encrypt/decrypt roundtrip test in Dart | ❌ (5 tests) |
| Q2 | **whisper_core unit tests** for relay wire handling (contacts, groups, presence parsing) | ❌ |
| Q3 | **CI: build the Android APK** in the Mobile job (currently only analyze+test) | ❌ |
| Q4 | **Reconnect + session persistence** — persist sessions/identity across restarts (currently identity only) | ❌ |
| Q5 | **Offline queue visibility** — show pending/delivered states while disconnected | ❌ |
| Q6 | **Release signing** — debug keys only; add a real signing config before distribution | ❌ |

## 💡 Recommended order

1. **C1 Megolm groups** (security) → **C2 reconnect** → **C3 username persistence**
2. **F9 reaction pills + F1 media** (the two most visible "chat app" features)
3. **U1 avatars → U6 read-state → U5 sender avatars → U2 countdown** (bubble-level polish)
4. **F3 deep links → F4 typing → F5 day separators → F7 unread/pin → F6 search**
5. **F12 verified + C4 PeerIdCard + U9 i18n completeness**
6. **F2 push (needs Firebase)** + **F10 group avatar** + **F11 member management**
7. **Q1–Q6 quality gate** before any distribution
