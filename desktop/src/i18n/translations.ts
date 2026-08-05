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

    // ---- Chat surface ----------------------------------------------------
    "chat.select_conversation": "Select a conversation",
    "chat.select_conversation_hint":
      "Pick a conversation from the sidebar to start whispering. Every message is end-to-end encrypted — not even Whisper can read it.",
    "chat.view_profile_aria": "View {name}'s profile",
    "chat.view_group_info_aria": "View {name} group info",
    "chat.last_seen_prefix": "Last seen ",
    "chat.typing": "typing\u2026",
    "chat.messages_with": "Messages with {name}",
    "chat.actions_for_message": "Actions for message from {name}",
    "chat.copy_text": "Copy Text",
    "chat.delete_for_me": "Delete for me",
    "chat.date_today": "Today",
    "chat.date_yesterday": "Yesterday",
    "chat.new_messages": "New messages",
    "chat.search_open_aria": "Search in chat",
    "chat.search_close_aria": "Close search",
    "chat.search_placeholder": "Search in chat",
    "chat.search_aria": "Search messages",
    "chat.search_no_results": "No matches found",
    "chat.search_prev_aria": "Previous match",
    "chat.search_next_aria": "Next match",

    // ---- Message bubble --------------------------------------------------
    "bubble.read": "Read",
    "bubble.delivered": "Delivered",
    "bubble.sent": "Sent",

    // ---- Composer --------------------------------------------------------
    "composer.type_a_message": "Type a message",
    "composer.message_aria": "Message",
    "composer.enter_for_newline": "Enter for a new line · Ctrl+Enter to send",

    // ---- Add-contact dialog ----------------------------------------------
    "addContact.hint":
      "Paste a friend\u2019s Whisper ID. The session is established with their published pre-keys and every message is end-to-end encrypted.",
    "addContact.invalid_peer_id":
      "Enter a valid 16-character Whisper ID (hex digits only).",
    "addContact.starting_session": "Starting session\u2026",
    "addContact.start_chat": "Start chat",

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
    "groupInfo.leave_group_owner_hint":
      "You are the owner. If you leave, this group will have no owner.",
    "groupInfo.transfer_ownership": "Transfer ownership",
    "groupInfo.transfer_ownership_hint":
      "You are the owner. Transfer ownership to another member — you will become an admin.",
    "groupInfo.transfer_owner_select_aria": "Choose the new group owner",
    "groupInfo.transfer_owner_placeholder": "Choose a member…",
    "groupInfo.close_group_info": "Close group info",

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
    "toast.history_cleared": "Chat history cleared",
    "toast.identity_exported": "Identity backed up",
    "toast.identity_imported": "Identity restored",
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

    // ---- Chat surface ----------------------------------------------------
    "chat.select_conversation": "Valitse keskustelu",
    "chat.select_conversation_hint":
      "Valitse keskustelu sivupalkista aloittaaksesi kuiskailun. Jokainen viesti on end-to-end-salattu — edes Whisper ei voi lukea sitä.",
    "chat.view_profile_aria": "Näytä {name}:n profiili",
    "chat.view_group_info_aria": "Näytä ryhmän {name} tiedot",
    "chat.last_seen_prefix": "Nähty ",
    "chat.typing": "kirjoittaa…",
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
    "chat.search_prev_aria": "Edellinen osuma",
    "chat.search_next_aria": "Seuraava osuma",

    // ---- Message bubble --------------------------------------------------
    "bubble.read": "Luettu",
    "bubble.delivered": "Toimitettu",
    "bubble.sent": "Lähetetty",

    // ---- Composer --------------------------------------------------------
    "composer.type_a_message": "Kirjoita viesti",
    "composer.message_aria": "Viesti",
    "composer.enter_for_newline": "Enter rivinvaihtoon · Ctrl+Enter lähettää",

    // ---- Add-contact dialog ----------------------------------------------
    "addContact.hint":
      "Liitä ystävän Whisper-ID. Istunto muodostetaan hänen julkaisemillaan pre-avaimeilla, ja jokainen viesti on end-to-end-salattu.",
    "addContact.invalid_peer_id":
      "Anna kelvollinen 16-merkkinen Whisper-ID (vain heksadesimaalimerkkejä).",
    "addContact.starting_session": "Aloitetaan istunto…",
    "addContact.start_chat": "Aloita keskustelu",

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
    "groupInfo.leave_group_owner_hint":
      "Olet omistaja. Jos poistut, ryhmä jää ilman omistajaa.",
    "groupInfo.transfer_ownership": "Siirrä omistajuus",
    "groupInfo.transfer_ownership_hint":
      "Olet omistaja. Siirrä omistajuus toiselle jäsenelle — sinusta tulee ylläpitäjä.",
    "groupInfo.transfer_owner_select_aria": "Valitse uusi ryhmän omistaja",
    "groupInfo.transfer_owner_placeholder": "Valitse jäsen…",
    "groupInfo.close_group_info": "Sulje ryhmän tiedot",

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
    "toast.history_cleared": "Keskusteluhistoria tyhjennetty",
    "toast.identity_exported": "Henkilöllisyys varmuuskopioitu",
    "toast.identity_imported": "Henkilöllisyys palautettu",
    "toast.identity_import_restart":
      "Käynnistetään uudelleen palautetun henkilöllisyyden käyttöönottamiseksi…",
    "toast.autostart_enabled": "Whisper avautuu kirjautumisen yhteydessä",
    "toast.autostart_disabled": "Whisper ei enää avaudu kirjautumisen yhteydessä",
  },
};
