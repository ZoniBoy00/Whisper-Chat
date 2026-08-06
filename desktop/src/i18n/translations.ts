import type { TranslationParams } from "./types";

/**
 * All user-facing strings, keyed by language. Keys are dot-scoped so related
 * strings group naturally. Values are either plain templates with `{param}`
 * interpolation or functions for grammar that needs real pluralization (the
 * Finnish partitive forms). `TranslationKey` is derived from the English
 * dictionary, so a key missing from the Finnish one is a compile error.
 */
export const translations = {
  en: {
    // ---- App shell -------------------------------------------------------
    "app.identity_load_failed": "Could not load your identity.",
    "app.retry": "Retry",

    // ---- Splash ----------------------------------------------------------
    "splash.tagline": "End-to-end encrypted",

    // ---- Onboarding ------------------------------------------------------
    "onboarding.welcome_title": "Welcome to Whisper",
    "onboarding.welcome_subtitle":
      "Your conversations are whispers — only you and the recipient can hear them.",
    "onboarding.trust_e2e": "E2E encrypted",
    "onboarding.trust_zero_knowledge": "Zero-knowledge",
    "onboarding.trust_keys": "Keys on device",
    "onboarding.name_label": "What should people call you?",
    "onboarding.name_hint":
      "Your display name is public profile data — like a WhatsApp profile name. Leave it blank to stay anonymous and be known by your Whisper ID, which you\u2019ll receive after creating your identity.",
    "onboarding.name_placeholder": "e.g. Alice",
    "onboarding.creating_identity": "Creating your identity\u2026",
    "onboarding.create_identity": "Create Identity",
    "onboarding.restore_identity": "Restore Identity",
    "onboarding.restore_identity_hint":
      "Restoring from an identity file is coming soon. For now your identity file is stored locally in the app data folder and never leaves this device.",

    // ---- Time-ago strings (format.ts) ------------------------------------
    "time.just_now": "just now",
    "time.minutes_ago": ({ n }: TranslationParams) =>
      `${n} minute${n === 1 ? "" : "s"} ago`,
    "time.hours_ago": ({ n }: TranslationParams) =>
      `${n} hour${n === 1 ? "" : "s"} ago`,
    "time.days_ago": ({ n }: TranslationParams) =>
      `${n} day${n === 1 ? "" : "s"} ago`,

    // ---- Shared / common strings -----------------------------------------
    "common.settings": "Settings",
    "common.confirm_again": "Click again to confirm",
    "common.reset_identity": "Reset identity",
    "common.copy": "Copy",
    "common.copied": "Copied",
    "common.copied_to_clipboard": "Copied to clipboard",
    "common.invite_copied": "Invite link copied",
    "common.share_invite": "Share invite",
    "common.copy_whisper_id": "Copy Whisper ID",
    "common.online": "Online",
    "common.end_to_end_encrypted": "End-to-end encrypted",
    "common.owner": "Owner",
    "common.admin": "Admin",
    "common.member": "Member",
    "common.close_dialog": "Close dialog",
    "common.members_count": ({ n }: TranslationParams) =>
      `${n} member${n === 1 ? "" : "s"}`,
    "common.last_seen_unavailable": "Last seen unavailable",
    "common.new_group": "New group",
    "common.group_info": "Group info",
    "common.message": "Message",
    "common.your_whisper_id": "Your Whisper ID",
    "common.whisper_id": "Whisper ID",
    "common.new_message_from": "New message from {name}",
    "common.send_message": "Send message",
    "common.remove_contact": "Remove contact",
    "common.close_settings": "Close settings",

    // ---- Sidebar ---------------------------------------------------------
    "sidebar.identity_options": "Identity options",
    "sidebar.identity_local_note":
      "Identity is stored locally in the app data folder. It never leaves this device.",
    "sidebar.search_label": "Search by name, @username or Whisper ID",
    "sidebar.search_placeholder": "Search by name, @username or ID",
    "sidebar.conversations": "Conversations",
    "sidebar.start_new_chat": "Start a new chat",
    "sidebar.no_users_found": "No users found",
    "sidebar.no_users_found_hint":
      "No registered usernames or IDs match your search.",
    "sidebar.search_results": "Search results",
    "sidebar.whisper_user": "Whisper user",
    "sidebar.no_conversations": "No conversations yet",
    "sidebar.no_conversations_hint":
      "Start a chat with a friend by their Whisper ID.",
    "sidebar.new_chat": "New Chat",
    "sidebar.no_conversations_found": "No conversations found",
    "sidebar.no_conversations_found_hint":
      "No names or Whisper IDs match your search.",
    "sidebar.actions_for": "Actions for {name}",
    "sidebar.view_profile": "View Profile",
    "sidebar.view_group_info": "View Group Info",
    "sidebar.copy_peer_id": "Copy Peer ID",
    "sidebar.pin_chat": "Pin chat",
    "sidebar.unpin_chat": "Unpin chat",
    "sidebar.pinned": "Pinned",
    "sidebar.unread_messages": ({ n }: TranslationParams) =>
      `${n} unread message${n === 1 ? "" : "s"}`,
    "sidebar.connected": "Connected",
    "sidebar.e2ee_suffix": "· end-to-end encrypted",
    "sidebar.reconnecting": "Reconnecting\u2026",
    "sidebar.reconnect_attempt":
      "Attempt {attempt} · retrying in {seconds}s",
    "sidebar.connecting": "Connecting\u2026",
    "sidebar.disconnected": "Disconnected",
    "sidebar.reconnect": "Reconnect",
    "sidebar.tab_chats": "Chats",
    "sidebar.tab_contacts": "Contacts",
    "sidebar.no_contacts": "No contacts yet",
    "sidebar.no_contacts_hint":
      "Add someone by their Whisper ID to start chatting.",

    // ---- Chat surface ----------------------------------------------------
    "chat.select_conversation": "Select a conversation",
    "chat.select_conversation_hint":
      "Pick a conversation from the sidebar to start whispering. Every message is end-to-end encrypted — not even Whisper can read it.",
    "chat.view_profile_aria": "View {name}'s profile",
    "chat.view_group_info_aria": "View {name} group info",
    "chat.last_seen_prefix": "Last seen ",
    "chat.typing": "typing",
    "chat.typing_name": "{name} typing",
    "chat.typing_many": "{n} members typing",
    "chat.member_joined": "{name} joined the group",
    "chat.member_left": "{name} left the group",
    "chat.messages_with": "Messages with {name}",
    "chat.actions_for_message": "Actions for message from {name}",
    "chat.copy_text": "Copy Text",
    "chat.delete_for_me": "Delete for me",
    "chat.reply": "Reply",
    "chat.add_reaction": "Add reaction",
    "chat.react_to_message": "React to message",
    "chat.date_today": "Today",
    "chat.date_yesterday": "Yesterday",
    "chat.new_messages": "New messages",
    "chat.search_open_aria": "Search in chat",
    "chat.search_close_aria": "Close search",
    "chat.search_placeholder": "Search in chat",
    "chat.search_aria": "Search messages",
    "chat.search_no_results": "No matches found",
    "chat.expiry_toggle": "Disappearing messages: {label}",
    "chat.expiry_title": "Disappearing messages",
    "chat.expiry_off": "Off",
    "chat.expiry_5s": "5 seconds",
    "chat.expiry_30s": "30 seconds",
    "chat.expiry_1m": "1 minute",
    "chat.expiry_1h": "1 hour",
    "chat.expiry_1d": "1 day",
    "chat.search_prev_aria": "Previous match",
    "chat.search_next_aria": "Next match",

    // ---- Message bubble --------------------------------------------------
    "bubble.read": "Read",
    "bubble.delivered": "Delivered",
    "bubble.sent": "Sent",
    "bubble.react": "React",
    "bubble.disappearing": "Disappearing message",

    // ---- Safety number ---------------------------------------------------
    "safety.title": "Safety number",
    "safety.loading": "Computing…",
    "safety.unknown_key":
      "Start a chat with this contact first — the safety number is derived from your keys.",
    "safety.verified": "Verified",
    "safety.qr_alt": "Safety number QR code",
    "safety.short": "Short: {tag}",
    "safety.verify_hint":
      "Compare this number with {name} through another channel. If it matches, the chat is secure.",
    "safety.verify": "Mark as verified",
    "safety.unverify": "Remove verification",

    // ---- Invite preview dialog -------------------------------------------
    "invite.hint":
      "This person invited you to Whisper. Add them to start chatting — every message is end-to-end encrypted.",
    "invite.add_friend": "Add friend",
    "invite.request_sent": "Friend request sent!",

    // ---- Group invites ----------------------------------------------------
    "invites.title": "Group invites",
    "invites.from": "from {peer}",
    "invites.accept": "Accept",
    "invites.accept_aria": "Accept invite to {group}",
    "invites.decline": "Decline",
    "invites.decline_aria": "Decline invite to {group}",
    "invites.received": "You were invited to {group}",
    "invites.outcome_accepted": "{peer} accepted your invite",
    "invites.outcome_declined": "{peer} declined your invite",

    // ---- Composer --------------------------------------------------------
    "composer.type_a_message": "Type a message",
    "composer.message_aria": "Message",
    "composer.enter_for_newline": "Enter for a new line · Ctrl+Enter to send",
    "composer.replying_to": "Replying to {name}",
    "composer.cancel_reply": "Cancel reply",
    "composer.yourself": "You",
    "composer.unknown_sender": "Unknown sender",

    // ---- Add-contact dialog ----------------------------------------------
    "addContact.title": "Add contact",
    "addContact.hint":
      "Enter a friend\u2019s Whisper ID to send them a friend request. Once they accept, you can message each other end-to-end encrypted.",
    "addContact.invalid_peer_id":
      "Enter a valid 24-character Whisper ID (hex digits only).",
    "addContact.sending": "Sending\u2026",
    "addContact.send_request": "Send request",
    "addContact.request_sent_title": "Request sent",
    "addContact.request_sent_hint":
      "Waiting for {peerId} to accept. You\u2019ll be notified when they do.",
    "addContact.done": "Done",

    // ---- Contacts / friend requests --------------------------------------
    "contacts.title": "Requests",
    "contacts.accept": "Accept",
    "contacts.accept_aria": "Accept {name}\u2019s friend request",
    "contacts.decline": "Decline",
    "contacts.decline_aria": "Decline {name}\u2019s friend request",
    "contacts.pending": "Pending",
    "contacts.request_sent": "Request sent",
    "contacts.already_contacts": "You\u2019re already contacts",
    "contacts.already_pending": "A request is already pending",
    "contacts.not_in_contacts": "You must be contacts first",
    "contacts.you_are_contacts": "You are now contacts with {name}",
    "contacts.request_received": "New friend request from {name}",
    "contacts.request_declined": "{name} declined your friend request",
    "contacts.contact_removed": "Contact removed",
    "contacts.cannot_add_self": "You can\u2019t add yourself as a contact",
    "contacts.not_found": "No user found with that Whisper ID",
    "contacts.rate_limited": "Too many requests \u2014 try again later",

    // ---- Profile dialog --------------------------------------------------
    "profile.close_profile": "Close profile",
    "profile.confirm_remove": "Confirm remove",
    "profile.remove_contact_hint":
      "Removes this contact and its messages on this device only — the other side and the relay are unaffected. Click again to confirm.",

    // ---- Group-info dialog ----------------------------------------------
    "groupInfo.loading_members": "Loading members\u2026",
    "groupInfo.group_members": "Group members",
    "groupInfo.make_admin": "Make admin",
    "groupInfo.make_admin_aria": "Make {peerId} an admin",
    "groupInfo.demote_from_admin": "Demote from admin",
    "groupInfo.demote_aria": "Demote {peerId}",
    "groupInfo.remove_from_group": "Remove from group",
    "groupInfo.remove_from_group_aria": "Remove {peerId} from the group",
    "groupInfo.leave_group": "Leave group",
    "groupInfo.rename_group": "Rename group",
    "common.confirm": "Confirm",
    "common.cancel": "Cancel",
    "toast.group_renamed": "Group renamed",
    "groupInfo.copy_join_link": "Copy join link",
    "groupInfo.join_link_hint":
      "Anyone with this link can join the group. Treat it like a password.",

    // ---- Join-group dialog ------------------------------------------------
    "join.title": "Join group",
    "join.hint":
      "You were invited to join this group. Join to start chatting — every message is end-to-end encrypted.",
    "join.join": "Join",
    "join.joining": "Joining…",
    "join.invalid_link": "This invite link is invalid or expired.",
    "join.already_member": "You are already a member of this group.",
    "join.group_not_found": "Group not found.",
    "groupInfo.leave_group_owner_hint":
      "You are the owner. If you leave, this group will have no owner.",
    "groupInfo.transfer_ownership": "Transfer ownership",
    "groupInfo.transfer_ownership_hint":
      "You are the owner. Transfer ownership to another member — you will become an admin.",
    "groupInfo.transfer_owner_select_aria": "Choose the new group owner",
    "groupInfo.transfer_owner_placeholder": "Choose a member…",
    "groupInfo.close_group_info": "Close group info",
    "groupInfo.add_member": "Add member",
    "groupInfo.add_member_hint":
      "Pick from your accepted contacts. Every existing member shares their encryption key with them end-to-end.",
    "groupInfo.add_member_placeholder": "Choose a contact…",
    "groupInfo.no_contacts_to_add": "You have no accepted contacts to add yet.",
    "groupInfo.invalid_peer_id_24":
      "Enter a valid 24-character Whisper ID (hex digits only).",
    "groupInfo.member_already_in_group": "That peer is already in this group.",
    "groupInfo.change_photo": "Change photo",
    "groupInfo.change_photo_hint":
      "Set a group photo. PNG, JPEG or WebP, up to 2 MB.",
    "groupInfo.photo_type_error": "Choose a PNG, JPEG or WebP image.",
    "groupInfo.photo_size_error": "Photo must be 2 MB or smaller.",

    // ---- New-group dialog ------------------------------------------------
    "newGroup.hint":
      "Members get the group key end-to-end encrypted — Whisper can never read it.",
    "newGroup.group_name": "Group name",
    "newGroup.add_members_by_id": "Add members by Whisper ID",
    "newGroup.add": "Add",
    "newGroup.invalid_peer_id_24":
      "Enter a valid 24-character Whisper ID (hex digits only).",
    "newGroup.already_owner": "You are already the owner of this group.",
    "newGroup.member_already_added": "That member is already in the list.",
    "newGroup.group_name_required": "Give the group a name.",
    "newGroup.group_name_too_long": "Group names must be 64 characters or fewer.",
    "newGroup.add_member_required": "Add at least one member.",
    "newGroup.not_contact": "You can only add accepted contacts to a group.",
    "newGroup.pick_contact": "Pick a contact",
    "newGroup.no_contacts_to_add": "No accepted contacts yet",
    "newGroup.selected_members": "Selected members",
    "newGroup.remove_member_aria": "Remove member {peerId}",
    "newGroup.creating_group": "Creating group\u2026",
    "newGroup.create_group": "Create group",

    // ---- Settings tabs ---------------------------------------------------
    "settings.sections_aria": "Settings sections",
    "settings.tab_general": "General",
    "settings.tab_privacy": "Privacy",
    "settings.tab_notifications": "Notifications",
    "settings.tab_logs": "Logs",
    "settings.tab_about": "About",

    // ---- Settings: General ----------------------------------------------
    "general.profile": "Profile",
    "general.username": "Username",
    "general.username_chars_error":
      "Usernames use lowercase letters, digits and underscores only.",
    "general.username_length_error": "Usernames must be 3\u201332 characters.",
    "general.username_reserved_error": "That username is reserved.",
    "general.registered": "Registered",
    "general.change": "Change",
    "general.pick_new_handle": "Pick a new public handle.",
    "general.choose_username":
      "Choose your username — people can find you by it.",
    "general.username_placeholder": "e.g. alice_42",
    "general.registering": "Registering\u2026",
    "general.register": "Register",
    "general.username_hint":
      "3\u201332 characters, lowercase letters, digits and underscores. Reserved: admin, whisper, support, mod, system, root.",
    "general.avatar": "Avatar",
    "general.avatar_hint":
      "Shown next to your messages. PNG, JPEG or WebP, up to 2 MB.",
    "general.choose_another": "Choose another",
    "general.upload_avatar": "Upload avatar",
    "general.avatar_type_error": "Choose a PNG, JPEG or WebP image.",
    "general.avatar_size_error": "Avatar must be 2 MB or smaller.",
    "general.saved": "Saved",
    "general.saving": "Saving\u2026",
    "general.save": "Save",
    "general.display_name": "Display name",
    "general.what_should_people_call_you": "What should people call you?",
    "general.display_name_too_long":
      "Display name must be 64 characters or fewer.",
    "general.display_name_hint":
      "Public profile data — shown to people who start a chat with you. 64 characters max.",
    "general.appearance": "Appearance",
    "general.theme": "Theme",
    "general.theme_hint": "Dark is the default; your choice is remembered.",
    "general.dark": "Dark",
    "general.light": "Light",
    "general.language": "Language",
    "general.language_hint": "The language of the user interface.",
    "general.identity": "Identity",
    "general.identity_reset_hint":
      "Keys never leave this device. Resetting creates a fresh identity with a brand-new peer ID.",
    "general.register_username_first":
      "Register a username before uploading an avatar.",

    // ---- Settings: General — startup ------------------------------------
    "general.startup": "Startup",
    "general.autostart_title": "Open Whisper on system startup",
    "general.autostart_desc":
      "Registers Whisper to launch automatically when you sign in to your computer.",
    "general.minimize_to_tray_title": "Minimize to tray on close",
    "general.minimize_to_tray_desc":
      "Closing the window hides Whisper to the system tray instead of quitting. Use the tray icon to bring it back or to quit.",

    // ---- Settings: General — messaging ----------------------------------
    "general.messaging": "Messaging",
    "general.enter_to_send_title": "Enter to send",
    "general.enter_to_send_desc":
      "Press Enter to send a message. Turn off to use Enter for a new line (Ctrl+Enter still sends).",
    "general.message_font_title": "Message font size",
    "general.message_font_desc": "Scales the text inside message bubbles.",
    "general.font_small": "Small",
    "general.font_normal": "Normal",
    "general.font_large": "Large",

    // ---- Settings: General — identity backup ----------------------------
    "general.identity_backup_hint":
      "Back up your identity file so you can restore your Whisper ID and keys on another device or after a reinstall.",
    "general.backup_identity": "Backup identity",
    "general.backup_everything": "Backup everything",
    "general.restore_everything": "Restore everything",
    "general.autobackup": "Automatic backups",
    "general.autobackup_title": "Automatic backups",
    "general.autobackup_desc":
      "Write a full backup (identity + history) into a cloud-synced folder every day.",
    "general.autobackup_pick_folder": "Choose backup folder…",
    "general.autobackup_now": "Back up now",
    "general.restore_identity": "Restore identity",
    "general.restore_identity_warn":
      "Restoring replaces your current identity and requires an app restart.",

    // ---- Settings: Notifications ----------------------------------------
    "notifications.desktop_title": "Show desktop notifications",
    "notifications.desktop_desc":
      "Shows a native system notification for new messages while the window isn\u2019t focused. If the system notification permission was denied, the toggle stays on but nothing is shown.",
    "notifications.preview_title": "Preview message text in notifications",
    "notifications.preview_desc":
      "When off, notifications only say \u201cNew message from @name\u201d without the message content.",
    "notifications.sound_title": "Notification sound",
    "notifications.sound_desc":
      "Plays a short chime for new incoming messages — even while the window is focused. Turn off to stay silent.",
    "notifications.test_sound": "Test sound",

    // ---- Settings: Privacy ----------------------------------------------
    "privacy.intro":
      "Control what others can see about you — everything here is end-to-end protected by the relay.",
    "privacy.presence_title": "Show online status & last seen",
    "privacy.presence_desc":
      "When off, others always see you as offline with no last-seen — even while you\u2019re here.",
    "privacy.receipts_title": "Read receipts",
    "privacy.receipts_desc":
      "When off, we don\u2019t send receipts when you read messages. Receipts others send you are still shown — you can\u2019t stop others from seeing you\u2019ve read them.",
    "privacy.typing_title": "Typing indicator",
    "privacy.typing_desc": "When off, the peer never sees that you\u2019re typing.",

    // ---- Settings: Privacy — history ------------------------------------
    "privacy.history": "History",
    "privacy.clear_history_title": "Clear chat history",
    "privacy.clear_history_desc":
      "Deletes every message on this device. Contacts and encryption sessions are kept.",
    "privacy.clear_history_confirm":
      "Click again to confirm — this cannot be undone.",

    // ---- Settings: About -------------------------------------------------
    "about.tagline": "your conversations are whispers",
    "about.version": "Version 0.1.0 · MIT",
    "about.e2ee_zero_knowledge": "End-to-end encrypted · Zero-knowledge relay",
    "about.keys_on_device": "Keys never leave this device",

    // ---- Settings: Logs --------------------------------------------------
    "logs.intro":
      "Client-side logs help you diagnose issues. Logs stay on this device — they are never sent anywhere.",
    "logs.refresh": "Refresh",
    "logs.copy": "Copy logs",
    "logs.open_folder": "Open folder",
    "logs.load_failed": "Could not load client logs.",
    "logs.empty": "No log entries yet.",
    "logs.filter_all": "All",
    "logs.filter_errors": "Errors",
    "logs.list_aria": "Recent client logs",

    // ---- Chat list previews ----------------------------------------------
    "chatList.you_prefix": "You: ",

    // ---- Toasts (in-app notifications) -----------------------------------
    "toast.dismiss": "Dismiss notification",
    "toast.avatar_updated": "Avatar updated",
    "toast.username_registered": "Username registered",
    "toast.display_name_saved": "Display name saved",
    "toast.settings_saved": "Settings saved",
    "toast.group_created": "Group created",
    "toast.group_left": "You left the group",
    "toast.member_promoted": "Member promoted to admin",
    "toast.member_demoted": "Member demoted to regular member",
    "toast.member_removed": "Member removed from the group",
    "toast.group_transferred": "Group ownership transferred",
    "toast.group_joined": "Joined group",
    "toast.member_added": "Member added",
    "toast.invite_sent": "Invite sent",
    "toast.group_avatar_updated": "Group photo updated",
    "toast.group_removed": "You were removed from {name}",
    "toast.history_cleared": "Chat history cleared",
    "toast.identity_exported": "Identity backed up",
    "toast.identity_imported": "Identity restored",
    "toast.backup_exported": "Full backup saved",
    "toast.backup_imported": "Backup restored",
    "toast.backup_import_restart": "Restarting to apply…",
    "toast.identity_import_restart":
      "Restarting to apply your restored identity\u2026",
    "toast.autostart_enabled": "Will open Whisper at startup",
    "toast.autostart_disabled": "Whisper will no longer open at startup",
  },

  fi: {
    // ---- App shell -------------------------------------------------------
    "app.identity_load_failed": "Henkilöllisyyttä ei voitu ladata.",
    "app.retry": "Yritä uudelleen",

    // ---- Splash ----------------------------------------------------------
    "splash.tagline": "End-to-end-salattu",

    // ---- Onboarding ------------------------------------------------------
    "onboarding.welcome_title": "Tervetuloa Whisperiin",
    "onboarding.welcome_subtitle":
      "Keskustelusi ovat kuiskauksia — vain sinä ja vastaanottaja voitte kuulla ne.",
    "onboarding.trust_e2e": "E2E-salattu",
    "onboarding.trust_zero_knowledge": "Zero-knowledge",
    "onboarding.trust_keys": "Avaimet laitteella",
    "onboarding.name_label": "Millä nimellä sinua kutsutaan?",
    "onboarding.name_hint":
      "Näyttönimesi on julkista profiilitietoa — kuten WhatsApp-profiilin nimi. Jätä tyhjäksi pysyäksesi anonyymina ja ollaksesi tunnettu Whisper-ID:lläsi, jonka saat henkilöllisyyden luomisen jälkeen.",
    "onboarding.name_placeholder": "esim. Alice",
    "onboarding.creating_identity": "Luodaan henkilöllisyyttäsi…",
    "onboarding.create_identity": "Luo henkilöllisyys",
    "onboarding.restore_identity": "Palauta henkilöllisyys",
    "onboarding.restore_identity_hint":
      "Henkilöllisyystiedoston palautus on tulossa pian. Toistaiseksi henkilöllisyystiedostosi tallennetaan paikallisesti sovelluksen datakansioon, eikä se koskaan poistu laitteeltasi.",

    // ---- Time-ago strings (format.ts) ------------------------------------
    "time.just_now": "juuri nyt",
    "time.minutes_ago": ({ n }: TranslationParams) =>
      n === 1 ? "1 minuutti sitten" : `${n} minuuttia sitten`,
    "time.hours_ago": ({ n }: TranslationParams) =>
      n === 1 ? "1 tunti sitten" : `${n} tuntia sitten`,
    "time.days_ago": ({ n }: TranslationParams) =>
      n === 1 ? "1 päivä sitten" : `${n} päivää sitten`,

    // ---- Shared / common strings -----------------------------------------
    "common.settings": "Asetukset",
    "common.confirm_again": "Vahvista napsauttamalla uudelleen",
    "common.reset_identity": "Nollaa henkilöllisyys",
    "common.copy": "Kopioi",
    "common.copied": "Kopioitu",
    "common.copied_to_clipboard": "Kopioitu leikepöydälle",
    "common.invite_copied": "Kutsulinkki kopioitu",
    "common.share_invite": "Jaa kutsu",
    "common.copy_whisper_id": "Kopioi Whisper-ID",
    "common.online": "Paikalla",
    "common.end_to_end_encrypted": "End-to-end-salattu",
    "common.owner": "Omistaja",
    "common.admin": "Ylläpitäjä",
    "common.member": "Jäsen",
    "common.close_dialog": "Sulje valintaikkuna",
    "common.members_count": ({ n }: TranslationParams) =>
      n === 1 ? "1 jäsen" : `${n} jäsentä`,
    "common.last_seen_unavailable": "Viimeksi nähty ei saatavilla",
    "common.new_group": "Uusi ryhmä",
    "common.group_info": "Ryhmän tiedot",
    "common.message": "Viesti",
    "common.your_whisper_id": "Sinun Whisper-ID:si",
    "common.whisper_id": "Whisper-ID",
    "common.new_message_from": "Uusi viesti henkilöltä {name}",
    "common.send_message": "Lähetä viesti",
    "common.remove_contact": "Poista yhteystieto",
    "common.close_settings": "Sulje asetukset",

    // ---- Sidebar ---------------------------------------------------------
    "sidebar.identity_options": "Henkilöllisyyden asetukset",
    "sidebar.identity_local_note":
      "Henkilöllisyys tallennetaan paikallisesti sovelluksen datakansioon. Se ei koskaan poistu tältä laitteelta.",
    "sidebar.search_label": "Hae nimellä, @käyttäjänimellä tai Whisper-ID:llä",
    "sidebar.search_placeholder": "Hae nimellä, @käyttäjänimellä tai ID:llä",
    "sidebar.conversations": "Keskustelut",
    "sidebar.start_new_chat": "Aloita uusi keskustelu",
    "sidebar.no_users_found": "Käyttäjiä ei löytynyt",
    "sidebar.no_users_found_hint":
      "Mikään rekisteröity käyttäjänimi tai ID ei vastaa hakua.",
    "sidebar.search_results": "Hakutulokset",
    "sidebar.whisper_user": "Whisper-käyttäjä",
    "sidebar.no_conversations": "Ei keskusteluja vielä",
    "sidebar.no_conversations_hint":
      "Aloita keskustelu ystävän kanssa hänen Whisper-ID:llään.",
    "sidebar.new_chat": "Uusi keskustelu",
    "sidebar.no_conversations_found": "Keskusteluja ei löytynyt",
    "sidebar.no_conversations_found_hint":
      "Mikään nimi tai Whisper-ID ei vastaa hakua.",
    "sidebar.actions_for": "Toiminnot: {name}",
    "sidebar.view_profile": "Näytä profiili",
    "sidebar.view_group_info": "Näytä ryhmän tiedot",
    "sidebar.copy_peer_id": "Kopioi peer-ID",
    "sidebar.pin_chat": "Kiinnitä keskustelu",
    "sidebar.unpin_chat": "Poista kiinnitys",
    "sidebar.pinned": "Kiinnitetty",
    "sidebar.unread_messages": ({ n }: TranslationParams) =>
      n === 1 ? "1 lukematon viesti" : `${n} lukematonta viestiä`,
    "sidebar.connected": "Yhdistetty",
    "sidebar.e2ee_suffix": "· end-to-end-salattu",
    "sidebar.reconnecting": "Yhdistetään uudelleen…",
    "sidebar.reconnect_attempt": "Yritys {attempt} · yritetään uudelleen {seconds} s",
    "sidebar.connecting": "Yhdistetään…",
    "sidebar.disconnected": "Ei yhteyttä",
    "sidebar.reconnect": "Yhdistä",
    "sidebar.tab_chats": "Keskustelut",
    "sidebar.tab_contacts": "Kaverit",
    "sidebar.no_contacts": "Ei kavereita vielä",
    "sidebar.no_contacts_hint":
      "Lisää joku Whisper-tunnuksella aloittaaksesi keskustelun.",

    // ---- Chat surface ----------------------------------------------------
    "chat.select_conversation": "Valitse keskustelu",
    "chat.select_conversation_hint":
      "Valitse keskustelu sivupalkista aloittaaksesi kuiskailun. Jokainen viesti on end-to-end-salattu — edes Whisper ei voi lukea sitä.",
    "chat.view_profile_aria": "Näytä {name}:n profiili",
    "chat.view_group_info_aria": "Näytä ryhmän {name} tiedot",
    "chat.last_seen_prefix": "Nähty ",
    "chat.typing": "kirjoittaa",
    "chat.typing_name": "{name} kirjoittaa",
    "chat.typing_many": "{n} jäsentä kirjoittaa",
    "chat.member_joined": "{name} liittyi ryhmään",
    "chat.member_left": "{name} poistui ryhmästä",
    "chat.messages_with": "Viestit: {name}",
    "chat.actions_for_message": "Viestin toiminnot: {name}",
    "chat.copy_text": "Kopioi teksti",
    "chat.delete_for_me": "Poista minulta",
    "chat.date_today": "Tänään",
    "chat.date_yesterday": "Eilen",
    "chat.new_messages": "Uusia viestejä",
    "chat.search_open_aria": "Hae keskustelusta",
    "chat.search_close_aria": "Sulje haku",
    "chat.search_placeholder": "Hae keskustelusta",
    "chat.search_aria": "Hae viestejä",
    "chat.search_no_results": "Ei osumia",
    "chat.expiry_toggle": "Katoavat viestit: {label}",
    "chat.expiry_title": "Katoavat viestit",
    "chat.expiry_off": "Pois",
    "chat.expiry_5s": "5 sekuntia",
    "chat.expiry_30s": "30 sekuntia",
    "chat.expiry_1m": "1 minuutti",
    "chat.expiry_1h": "1 tunti",
    "chat.expiry_1d": "1 päivä",
    "chat.search_prev_aria": "Edellinen osuma",
    "chat.search_next_aria": "Seuraava osuma",
    "chat.reply": "Vastaa",
    "chat.add_reaction": "Lisää reaktio",
    "chat.react_to_message": "Reagoi viestiin",

    // ---- Message bubble --------------------------------------------------
    "bubble.read": "Luettu",
    "bubble.delivered": "Toimitettu",
    "bubble.sent": "Lähetetty",
    "bubble.react": "Reagoi",
    "bubble.disappearing": "Katoava viesti",

    // ---- Safety number ---------------------------------------------------
    "safety.title": "Varmistusnumero",
    "safety.loading": "Lasketaan…",
    "safety.unknown_key":
      "Aloita ensin keskustelu tämän kontaktin kanssa — varmistusnumero johdetaan avaimistanne.",
    "safety.verified": "Varmistettu",
    "safety.qr_alt": "Varmistusnumeron QR-koodi",
    "safety.short": "Lyhyt: {tag}",
    "safety.verify_hint":
      "Vertaa tätä numeroa {name}:n kanssa toisen kanavan kautta. Jos se täsmää, keskustelu on suojattu.",
    "safety.verify": "Merkitse varmistetuksi",
    "safety.unverify": "Poista varmistus",

    // ---- Invite preview dialog -------------------------------------------
    "invite.hint":
      "Tämä henkilö kutsui sinut Whisperiin. Lisää hänet kaveriksi aloittaaksesi keskustelun — jokainen viesti on end-to-end-salattu.",
    "invite.add_friend": "Lisää kaveriksi",
    "invite.request_sent": "Kaveripyyntö lähetetty!",

    // ---- Group invites ----------------------------------------------------
    "invites.title": "Ryhmäkutsut",
    "invites.from": "kutsuja: {peer}",
    "invites.accept": "Hyväksy",
    "invites.accept_aria": "Hyväksy kutsu ryhmään {group}",
    "invites.decline": "Hylkää",
    "invites.decline_aria": "Hylkää kutsu ryhmään {group}",
    "invites.received": "Sinut kutsuttiin ryhmään {group}",
    "invites.outcome_accepted": "{peer} hyväksyi kutsusi",
    "invites.outcome_declined": "{peer} hylkäsi kutsusi",

    // ---- Composer --------------------------------------------------------
    "composer.type_a_message": "Kirjoita viesti",
    "composer.message_aria": "Viesti",
    "composer.replying_to": "Vastataan: {name}",
    "composer.cancel_reply": "Peruuta vastaus",
    "composer.yourself": "Sinä",
    "composer.unknown_sender": "Tuntematon lähettäjä",
    "composer.enter_for_newline": "Enter rivinvaihtoon · Ctrl+Enter lähettää",

    // ---- Add-contact dialog ----------------------------------------------
    "addContact.title": "Lisää yhteystieto",
    "addContact.hint":
      "Anna ystävän Whisper-ID lähettääksesi hänelle kaveripyynnön. Kun hän hyväksyy, voitte viestiä toisillenne end-to-end-salattuna.",
    "addContact.invalid_peer_id":
      "Anna kelvollinen 24-merkkinen Whisper-ID (vain heksadesimaalimerkkejä).",
    "addContact.sending": "Lähetetään…",
    "addContact.send_request": "Lähetä pyyntö",
    "addContact.request_sent_title": "Pyyntö lähetetty",
    "addContact.request_sent_hint":
      "Odotetaan, että {peerId} hyväksyy pyynnön. Saat ilmoituksen, kun hän tekee niin.",
    "addContact.done": "Valmis",

    // ---- Contacts / friend requests --------------------------------------
    "contacts.title": "Pyynnöt",
    "contacts.accept": "Hyväksy",
    "contacts.accept_aria": "Hyväksy {name}:n kaveripyyntö",
    "contacts.decline": "Hylkää",
    "contacts.decline_aria": "Hylkää {name}:n kaveripyyntö",
    "contacts.pending": "Odottaa",
    "contacts.request_sent": "Pyyntö lähetetty",
    "contacts.already_contacts": "Olette jo kavereita",
    "contacts.already_pending": "Pyyntö on jo odottamassa",
    "contacts.not_in_contacts": "Sinun on oltava ensin kavereita",
    "contacts.you_are_contacts": "Olette nyt kavereita: {name}",
    "contacts.request_received": "Uusi kaveripyyntö: {name}",
    "contacts.request_declined": "{name} hylkäsi kaveripyyntösi",
    "contacts.contact_removed": "Yhteystieto poistettu",
    "contacts.cannot_add_self": "Et voi lisätä itseäsi yhteystiedoksi",
    "contacts.not_found": "Käyttäjää ei löytynyt tällä Whisper-ID:llä",
    "contacts.rate_limited": "Liian monta pyyntöä — yritä myöhemmin uudelleen",

    // ---- Profile dialog --------------------------------------------------
    "profile.close_profile": "Sulje profiili",
    "profile.confirm_remove": "Vahvista poisto",
    "profile.remove_contact_hint":
      "Poistaa tämän yhteystiedon ja sen viestit vain tältä laitteelta — toinen osapuoli ja rele eivät vaikuta. Vahvista napsauttamalla uudelleen.",

    // ---- Group-info dialog ----------------------------------------------
    "groupInfo.loading_members": "Ladataan jäseniä…",
    "groupInfo.group_members": "Ryhmän jäsenet",
    "groupInfo.make_admin": "Tee ylläpitäjäksi",
    "groupInfo.make_admin_aria": "Tee {peerId} ylläpitäjäksi",
    "groupInfo.demote_from_admin": "Poista ylläpitäjyys",
    "groupInfo.demote_aria": "Alenna {peerId}",
    "groupInfo.remove_from_group": "Poista ryhmästä",
    "groupInfo.remove_from_group_aria": "Poista {peerId} ryhmästä",
    "groupInfo.leave_group": "Poistu ryhmästä",
    "groupInfo.rename_group": "Nimeä ryhmä uudelleen",
    "common.confirm": "Vahvista",
    "common.cancel": "Peruuta",
    "toast.group_renamed": "Ryhmä nimetty uudelleen",
    "groupInfo.copy_join_link": "Kopioi liittymislinkki",
    "groupInfo.join_link_hint":
      "Kuka tahansa tämän linkin saaneista voi liittyä ryhmään. Kohtele sitä kuin salasanaa.",

    // ---- Join-group dialog ------------------------------------------------
    "join.title": "Liity ryhmään",
    "join.hint":
      "Sinut on kutsuttu liittymään tähän ryhmään. Liity aloittaaksesi keskustelun — jokainen viesti on end-to-end-salattu.",
    "join.join": "Liity",
    "join.joining": "Liitytään…",
    "join.invalid_link": "Tämä kutsulinkki on virheellinen tai vanhentunut.",
    "join.already_member": "Olet jo tämän ryhmän jäsen.",
    "join.group_not_found": "Ryhmää ei löytynyt.",
    "groupInfo.leave_group_owner_hint":
      "Olet omistaja. Jos poistut, ryhmä jää ilman omistajaa.",
    "groupInfo.transfer_ownership": "Siirrä omistajuus",
    "groupInfo.transfer_ownership_hint":
      "Olet omistaja. Siirrä omistajuus toiselle jäsenelle — sinusta tulee ylläpitäjä.",
    "groupInfo.transfer_owner_select_aria": "Valitse uusi ryhmän omistaja",
    "groupInfo.transfer_owner_placeholder": "Valitse jäsen…",
    "groupInfo.close_group_info": "Sulje ryhmän tiedot",
    "groupInfo.add_member": "Lisää jäsen",
    "groupInfo.add_member_hint":
      "Valitse hyväksyttyjen yhteystietojesi joukosta. Jokainen nykyinen jäsen jakaa oman salausavaimensa uudelle jäsenelle päästä päähän.",
    "groupInfo.add_member_placeholder": "Valitse yhteystieto…",
    "groupInfo.no_contacts_to_add": "Sinulla ei ole vielä hyväksyttyjä yhteystietoja.",
    "groupInfo.invalid_peer_id_24":
      "Anna kelvollinen 24-merkkinen Whisper-ID (vain heksadesimaalimerkkejä).",
    "groupInfo.member_already_in_group": "Tämä käyttäjä on jo ryhmässä.",
    "groupInfo.change_photo": "Vaihda kuva",
    "groupInfo.change_photo_hint":
      "Aseta ryhmäkuva. PNG, JPEG tai WebP, enintään 2 Mt.",
    "groupInfo.photo_type_error": "Valitse PNG-, JPEG- tai WebP-kuva.",
    "groupInfo.photo_size_error": "Kuvan on oltava enintään 2 Mt.",

    // ---- New-group dialog ------------------------------------------------
    "newGroup.hint":
      "Jäsenet saavat ryhmäavaimen end-to-end-salattuna — Whisper ei voi koskaan lukea sitä.",
    "newGroup.group_name": "Ryhmän nimi",
    "newGroup.add_members_by_id": "Lisää jäseniä Whisper-ID:llä",
    "newGroup.add": "Lisää",
    "newGroup.invalid_peer_id_24":
      "Anna kelvollinen 24-merkkinen Whisper-ID (vain heksadesimaalimerkkejä).",
    "newGroup.already_owner": "Olet jo tämän ryhmän omistaja.",
    "newGroup.member_already_added": "Tämä jäsen on jo listalla.",
    "newGroup.group_name_required": "Anna ryhmälle nimi.",
    "newGroup.group_name_too_long": "Ryhmän nimen on oltava enintään 64 merkkiä.",
    "newGroup.add_member_required": "Lisää vähintään yksi jäsen.",
    "newGroup.not_contact": "Ryhmään voi lisätä vain hyväksyttyjä kavereita.",
    "newGroup.pick_contact": "Valitse yhteystieto",
    "newGroup.no_contacts_to_add": "Ei hyväksyttyjä yhteystietoja",
    "newGroup.selected_members": "Valitut jäsenet",
    "newGroup.remove_member_aria": "Poista jäsen {peerId}",
    "newGroup.creating_group": "Luodaan ryhmää…",
    "newGroup.create_group": "Luo ryhmä",

    // ---- Settings tabs ---------------------------------------------------
    "settings.sections_aria": "Asetusten osiot",
    "settings.tab_general": "Yleiset",
    "settings.tab_privacy": "Yksityisyys",
    "settings.tab_notifications": "Ilmoitukset",
    "settings.tab_logs": "Lokit",
    "settings.tab_about": "Tietoja",

    // ---- Settings: General ----------------------------------------------
    "general.profile": "Profiili",
    "general.username": "Käyttäjänimi",
    "general.username_chars_error":
      "Käyttäjänimissä saa olla vain pieniä kirjaimia, numeroita ja alaviivoja.",
    "general.username_length_error": "Käyttäjänimen on oltava 3–32 merkkiä.",
    "general.username_reserved_error": "Tämä käyttäjänimi on varattu.",
    "general.registered": "Rekisteröity",
    "general.change": "Vaihda",
    "general.pick_new_handle": "Valitse uusi julkinen tunnus.",
    "general.choose_username":
      "Valitse käyttäjänimesi — ihmiset löytävät sinut sen avulla.",
    "general.username_placeholder": "esim. alice_42",
    "general.registering": "Rekisteröidään…",
    "general.register": "Rekisteröi",
    "general.username_hint":
      "3–32 merkkiä, pieniä kirjaimia, numeroita ja alaviivoja. Varatut: admin, whisper, support, mod, system, root.",
    "general.avatar": "Profiilikuva",
    "general.avatar_hint": "Näytetään viestiesi yhteydessä. PNG, JPEG tai WebP, enintään 2 Mt.",
    "general.choose_another": "Valitse toinen",
    "general.upload_avatar": "Lataa profiilikuva",
    "general.avatar_type_error": "Valitse PNG-, JPEG- tai WebP-kuva.",
    "general.avatar_size_error": "Profiilikuvan on oltava enintään 2 Mt.",
    "general.saved": "Tallennettu",
    "general.saving": "Tallennetaan…",
    "general.save": "Tallenna",
    "general.display_name": "Näyttönimi",
    "general.what_should_people_call_you": "Millä nimellä sinua kutsutaan?",
    "general.display_name_too_long": "Näyttönimen on oltava enintään 64 merkkiä.",
    "general.display_name_hint":
      "Julkinen profiilitieto — näytetään henkilöille, jotka aloittavat keskustelun kanssasi. Enintään 64 merkkiä.",
    "general.appearance": "Ulkoasu",
    "general.theme": "Teema",
    "general.theme_hint": "Tumma on oletus; valintasi muistetaan.",
    "general.dark": "Tumma",
    "general.light": "Vaalea",
    "general.language": "Kieli",
    "general.language_hint": "Käyttöliittymän kieli.",
    "general.identity": "Henkilöllisyys",
    "general.identity_reset_hint":
      "Avaimet eivät koskaan poistu tästä laitteesta. Nollaus luo uuden henkilöllisyyden ja aivan uuden peer-ID:n.",
    "general.register_username_first":
      "Rekisteröi käyttäjänimi ennen profiilikuvan lataamista.",

    // ---- Settings: General — startup ------------------------------------
    "general.startup": "Käynnistys",
    "general.autostart_title": "Avaa Whisper järjestelmän käynnistyessä",
    "general.autostart_desc":
      "Rekisteröi Whisperin käynnistymään automaattisesti, kun kirjaudut tietokoneellesi.",
    "general.minimize_to_tray_title": "Pienennä ilmoitusalueelle suljettaessa",
    "general.minimize_to_tray_desc":
      "Ikkunan sulkeminen piilottaa Whisperin ilmoitusalueelle lopettamisen sijaan. Tuo ikkuna takaisin tai sulje sovellus ilmoitusalueen kuvakkeen valikosta.",

    // ---- Settings: General — messaging ----------------------------------
    "general.messaging": "Viestit",
    "general.enter_to_send_title": "Enter lähettää viestin",
    "general.enter_to_send_desc":
      "Lähetä viesti Enter-näppäimellä. Poista käytöstä käyttääksesi Enteriä rivinvaihtoon (Ctrl+Enter lähettää silti).",
    "general.message_font_title": "Viestien fonttikoko",
    "general.message_font_desc": "Skaalaa viestikuplien tekstin kokoa.",
    "general.font_small": "Pieni",
    "general.font_normal": "Normaali",
    "general.font_large": "Suuri",

    // ---- Settings: General — identity backup ----------------------------
    "general.identity_backup_hint":
      "Varmuuskopioi henkilöllisyystiedostosi, jotta voit palauttaa Whisper-ID:n ja avaimesi toiselle laitteelle tai uudelleenasennuksen jälkeen.",
    "general.backup_identity": "Varmuuskopioi henkilöllisyys",
    "general.backup_everything": "Varmuuskopioi kaikki",
    "general.restore_everything": "Palauta kaikki",
    "general.autobackup": "Automaattiset varmuuskopiot",
    "general.autobackup_title": "Automaattiset varmuuskopiot",
    "general.autobackup_desc":
      "Kirjoita täysi varmuuskopio (henkilöllisyys + historia) pilveen synkattuun kansioon päivittäin.",
    "general.autobackup_pick_folder": "Valitse varmuuskopiokansio…",
    "general.autobackup_now": "Varmuuskopioi nyt",
    "general.restore_identity": "Palauta henkilöllisyys",
    "general.restore_identity_warn":
      "Palautus korvaa nykyisen henkilöllisyytesi ja edellyttää sovelluksen uudelleenkäynnistystä.",

    // ---- Settings: Notifications ----------------------------------------
    "notifications.desktop_title": "Näytä työpöytäilmoitukset",
    "notifications.desktop_desc":
      "Näyttää natiivin järjestelmäilmoituksen uusista viesteistä, kun ikkuna ei ole fokuksessa. Jos järjestelmän ilmoituslupa on evätty, kytkin pysyy päällä, mutta mitään ei näytetä.",
    "notifications.preview_title": "Näytä viestin teksti ilmoituksissa",
    "notifications.preview_desc":
      "Kun pois päältä, ilmoituksissa sanotaan vain \u201cUusi viesti @nimeltä\u201d ilman viestin sisältöä.",
    "notifications.sound_title": "Ilmoitusääni",
    "notifications.sound_desc":
      "Soittaa lyhyen äänimerkin uusista saapuvista viesteistä — myös ikkunan ollessa fokuksessa. Poista käytöstä pysyäksesi hiljaa.",
    "notifications.test_sound": "Testaa ääni",

    // ---- Settings: Privacy ----------------------------------------------
    "privacy.intro":
      "Hallinnoi, mitä muut voivat nähdä sinusta — kaikki tämä on end-to-end-suojattu releen välityksellä.",
    "privacy.presence_title": "Näytä paikallaolotila ja viimeksi nähty",
    "privacy.presence_desc":
      "Kun pois päältä, muut näkevät sinut aina offline-tilassa ilman viimeksi nähtyä — vaikka olisit juuri nyt paikalla.",
    "privacy.receipts_title": "Lukukuittaukset",
    "privacy.receipts_desc":
      "Kun pois päältä, emme lähetä kuittauksia lukiessasi viestejä. Muiden lähettämät kuittaukset näytetään silti — et voi estää muita näkemästä, että olet lukenut viestit.",
    "privacy.typing_title": "Kirjoitusilmoitus",
    "privacy.typing_desc":
      "Kun pois päältä, toinen osapuoli ei koskaan näe, että kirjoitat.",

    // ---- Settings: Privacy — history ------------------------------------
    "privacy.history": "Historia",
    "privacy.clear_history_title": "Tyhjennä keskusteluhistoria",
    "privacy.clear_history_desc":
      "Poistaa kaikki viestit tältä laitteelta. Yhteystiedot ja salausistunnot säilyvät.",
    "privacy.clear_history_confirm":
      "Vahvista napsauttamalla uudelleen — tätä ei voi kumota.",

    // ---- Settings: About -------------------------------------------------
    "about.tagline": "keskustelusi ovat kuiskauksia",
    "about.version": "Versio 0.1.0 · MIT",
    "about.e2ee_zero_knowledge": "End-to-end-salattu · Zero-knowledge-rele",
    "about.keys_on_device": "Avaimet eivät koskaan poistu tästä laitteesta",

    // ---- Settings: Logs --------------------------------------------------
    "logs.intro":
      "Asiakaspuolen lokit auttavat vianetsinnässä. Lokit pysyvät tällä laitteella — niitä ei lähetetä koskaan minnekään.",
    "logs.refresh": "Päivitä",
    "logs.copy": "Kopioi lokit",
    "logs.open_folder": "Avaa kansio",
    "logs.load_failed": "Asiakaslokeja ei voitu ladata.",
    "logs.empty": "Ei lokimerkintöjä vielä.",
    "logs.filter_all": "Kaikki",
    "logs.filter_errors": "Virheet",
    "logs.list_aria": "Viimeisimmät asiakaslokit",

    // ---- Chat list previews ----------------------------------------------
    "chatList.you_prefix": "Sinä: ",

    // ---- Toasts (in-app notifications) -----------------------------------
    "toast.dismiss": "Sulje ilmoitus",
    "toast.avatar_updated": "Profiilikuva päivitetty",
    "toast.username_registered": "Käyttäjänimi rekisteröity",
    "toast.display_name_saved": "Näyttönimi tallennettu",
    "toast.settings_saved": "Asetukset tallennettu",
    "toast.group_created": "Ryhmä luotu",
    "toast.group_left": "Poistuit ryhmästä",
    "toast.member_promoted": "Jäsen ylennetty ylläpitäjäksi",
    "toast.member_demoted": "Ylläpitäjyys poistettu",
    "toast.member_removed": "Jäsen poistettu ryhmästä",
    "toast.group_transferred": "Ryhmän omistajuus siirretty",
    "toast.group_joined": "Liityit ryhmään",
    "toast.member_added": "Jäsen lisätty",
    "toast.invite_sent": "Kutsu lähetetty",
    "toast.group_avatar_updated": "Ryhmäkuva päivitetty",
    "toast.group_removed": "Sinut poistettiin ryhmästä {name}",
    "toast.history_cleared": "Keskusteluhistoria tyhjennetty",
    "toast.identity_exported": "Henkilöllisyys varmuuskopioitu",
    "toast.identity_imported": "Henkilöllisyys palautettu",
    "toast.backup_exported": "Täysi varmuuskopio tallennettu",
    "toast.backup_imported": "Varmuuskopio palautettu",
    "toast.backup_import_restart": "Käynnistetään uudelleen…",
    "toast.identity_import_restart":
      "Käynnistetään uudelleen palautetun henkilöllisyyden käyttöönottamiseksi…",
    "toast.autostart_enabled": "Whisper avautuu kirjautumisen yhteydessä",
    "toast.autostart_disabled": "Whisper ei enää avaudu kirjautumisen yhteydessä",
  },
};
