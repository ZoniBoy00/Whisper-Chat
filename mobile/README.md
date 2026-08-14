# Whisper Mobile 📱

Privacy-first, end-to-end-encrypted messaging — the mobile client, sharing
the same audited `e2ee-core` Rust crypto stack as the desktop app.

## Stack

- **Flutter** (Material 3, dark theme ported from the desktop `--wp-*` palette)
- **`whisper_core`** (Rust, `flutter_rust_bridge`) — identity, X3DH + Double
  Ratchet sessions, Megolm groups, full relay wire protocol
- **Shared relay** — zero-knowledge, `wss://whisper-test.homelab.cfd/ws`

## Features

- E2EE 1:1 chat (handshake, Double Ratchet, forward/backward secrecy)
- Profiles: signed usernames, display names, avatars, directory search
- Contacts: friend requests (accept/decline), online presence / last seen
- Groups: create, invites, join links, roles (owner/admin), group info
- Reactions, quoted replies, edit, delete, disappearing messages
- Safety numbers (60-digit E2EE verification)
- Password-encrypted backups (Argon2id + AES-256-GCM) — export/import
- i18n EN/FI

## Development

```sh
flutter pub get
flutter_rust_bridge_codegen generate   # after touching rust/src/api/*
flutter analyze
flutter test
flutter run                              # on a device/emulator
```

The relay URL is hardcoded (mirroring the desktop client) — it can only
change with a build.
