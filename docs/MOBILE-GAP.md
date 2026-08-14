# Whisper Mobile — Gap Analysis & Task List

> Updated: 2026-08-14 (commit 78e4a35)
> Status: Task list for OpenCode — what still needs to be done/fixed in the mobile app

## ✅ Already done (no longer on the list)

| Feature | Status |
|---|---|
| Backup/restore (Argon2id 19MiB + AES-256-GCM, v2 format, tamper-proof) | ✅ 5 unit tests |
| Safety numbers (`safetyNumber`, `shortSafetyNumber`) | ✅ |
| Avatar + username registration (`registerProfile`, `setAvatar`) | ✅ |
| Logs tab | ✅ |
| Flutter tests (l10n, theme) + CI "Mobile (Flutter)" job | ✅ |
| mobile/README | ✅ |
| Groups, reactions, edit/delete, receipts, i18n skeleton, disappearing-messages UI | ✅ |

---

## 🔴 CRITICAL BUGS

### B1. Mobile Whisper ID is not found by the desktop search (and vice versa)
- The mobile peer ID (e.g. `317b8da4...`) cannot be found when searched from the desktop app
- To verify:
  - (a) is the profile always registered with the relay (registerProfile) — also when no username has been set
  - (b) does `search_users` support peer-ID prefix search, not just username prefix
  - (c) does search work in both directions: mobile→desktop and desktop→mobile
- **This is release-blocker level** — the two-machine E2EE test requires that both parties can find each other

### B2. Scrolling in Settings resizes everything
- Scrolling in the Settings screen causes text/elements to change size
- Likely cause: Flutter layout (MediaQuery.textScaler, unstable ListView/SingleChildScrollView, or rebuild-driven scaling)
- Fix: lock font sizes, remove dynamic scaling, test on different screen sizes/DPIs

---

## 🔴 MISSING FEATURES (desktop parity)

### P1. Changing your own username
- `signUsername` is only for registration. The desktop "Change username" (update_profile message) is missing from mobile
- Once a name is set, it cannot be changed

### P2. Push notifications (FCM/APNs)
- "Notification options are coming to mobile soon" placeholder
- **Requires creating a Firebase project (Joni's action)** — OpenCode cannot create a Firebase account
- Only "you have a message" push (roadmap §8)

### P3. Disappearing messages — protocol wiring
- The UI dialog exists (main_screen), but **verify that the choice (Off/5s/30s/1h...) is actually attached to the outgoing message** (disappear field), not just UI decoration

### P4. Profile view of another user
- `getProfile` is callable, but there is no proper profile screen (desktop: ProfileDialog — avatar, username, peer ID, safety numbers link)

### P5. Group avatar
- The server supports it (`group_avatar_set`), but there is no mobile UI for it

---

## 🟡 UI/UX IMPROVEMENTS

### U1. "Your Whisper ID" header
- Takes up space in the main view → move it to Settings (desktop PeerIdCard pattern)

### U2. "Select a conversation" right panel
- Leftover from desktop → remove on mobile (useless, wastes space)

### U3. Whisper ID display
- Show truncated + copy button (not the full-length string taking up space)

### U4. Exact desktop palette match
- Compare theme.dart against the desktop CSS (colors, fonts, spacing, bubbles, avatars)

### U5. i18n completeness
- Language switching works, but review that ALL strings are translated (desktop has 1036 lines of translations vs mobile's i18n.dart)

---

## 🔧 TECHNICAL / QUALITY

### T1. whisper_core dead-code warnings
- 7 total (e.g. `FriendRequestIncoming`-Debug) — clean up, keep clippy clean

### T2. Expand Flutter tests
- Only 2 small tests (l10n, theme). Add widget tests: onboarding, settings, chat view, backup encrypt/decrypt roundtrip

---

## 💡 Recommended order

1. **B1 (peer search)** — release blocker, makes the E2EE test impossible
2. **B2 (scroll resizing)** — visible bug in every use
3. **P1 (username change)** — user-reported missing feature
4. **P3 (disappear wiring)** — protocol is ready, just expose it
5. **P4 (profile view)** + U1-U3 (UI cleanup)
6. **T1-T2** (quality gate)
7. **P2 (push)** — biggest remaining feature, requires a Firebase project (Joni's action)
8. **P5 (group avatar)** + U4-U5 (polish)
