import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// Lightweight EN/FI localization — mirrors the desktop i18n system.
/// The UI reads strings through `L10n.of(context).t('key')`.
class L10n {
  final String lang;
  const L10n(this.lang);

  static const supported = ['en', 'fi'];

  String t(String key) {
    final table = lang == 'fi' ? _fi : _en;
    return table[key] ?? _en[key] ?? key;
  }

  static Future<String> load() async {
    final prefs = await SharedPreferences.getInstance();
    final lang = prefs.getString('language');
    return (lang != null && supported.contains(lang)) ? lang : 'en';
  }

  static Future<void> save(String lang) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString('language', lang);
  }

  static const _en = {
    // App shell
    'app.title': 'Whisper',
    'your_whisper_id': 'Your Whisper ID',
    'search_users': 'Search users',
    'tab.chats': 'Chats',
    'tab.contacts': 'Contacts',
    'tab.groups': 'Groups',
    'section.conversations': 'Conversations',
    'section.contacts': 'Contacts',
    'section.groups': 'Groups',
    'new_chat': 'New chat',
    'new_group': 'New group',
    'friend_requests': 'Friend requests',
    'group_invites': 'Group invites',
    'accept': 'Accept',
    'decline': 'Decline',
    'online': 'Online',
    'group': 'Group',
    'no_conversations': 'No conversations yet',
    'no_contacts': 'No contacts yet',
    'no_groups': 'No groups yet',
    'create_group_hint': 'Create one with the + button',
    'search_hint': 'Search for a user to start',
    'accept_requests_hint': 'Accept friend requests to add contacts',
    'select_conversation': 'Select a conversation',
    'e2ee_note': 'Your messages are end-to-end encrypted',
    'no_messages': 'No messages yet',
    'message_hint': 'Message',
    'connected': 'Connected',
    'connecting': 'Connecting…',
    'not_connected': 'Not connected',
    'disconnected': 'Disconnected',
    'copy_peer_id': 'Copy peer ID',
    'settings': 'Settings',
    'reply': 'Reply',
    'react': 'React',
    'edit': 'Edit',
    'delete': 'Delete',
    'cancel': 'Cancel',
    'save': 'Save',
    'send': 'Send',
    'open': 'Open',
    'create': 'Create',
    'reconnecting': 'Reconnecting…',
    // Onboarding
    'welcome_title': 'Welcome to Whisper',
    'welcome_sub': 'Set up your profile. You can change these later in Settings.',
    'display_name': 'Display name',
    'display_name_hint': 'What friends will see',
    'username_optional': 'Username (optional)',
    'username_hint': 'lowercase_letters_123',
    'continue': 'Continue',
    'setting_up': 'Setting up…',
    'your_peer_id': 'YOUR PEER ID',
    'share_peer_hint': 'Share this ID so others can add you.',
    // Settings
    'settings.general': 'General',
    'settings.privacy': 'Privacy',
    'settings.notifications': 'Notifications',
    'settings.about': 'About',
    'settings.language': 'Language',
    'settings.identity': 'IDENTITY',
    'settings.identity_sub': 'Keys never leave this device. Share your peer ID to let others contact you.',
    'settings.connection': 'CONNECTION',
    'settings.relay_server': 'Relay server',
    'settings.privacy.title': 'Privacy',
    'settings.presence_visible': 'Show my online status',
    'settings.presence_visible_sub': 'When hidden, others always see you as offline.',
    'settings.read_receipts': 'Send read receipts',
    'settings.read_receipts_sub': 'Let others know when you read their messages.',
    'settings.typing': 'Send typing indicators',
    'settings.typing_sub': 'Let others see when you are composing.',
    'settings.notifications.title': 'Notifications',
    'settings.notifications_sub': 'Notification options are coming to mobile soon.',
    'settings.about.title': 'About',
    'settings.e2ee': 'End-to-end encrypted',
    'settings.e2ee_sub': 'X3DH + Double Ratchet (vodozemac)',
    'settings.zk': 'Zero-knowledge relay',
    'settings.zk_sub': 'Sees ciphertext only',
    'settings.version': 'Whisper mobile',
    'settings.version_sub': 'Flutter + shared Rust e2ee-core',
    // Group info
    'group.info': 'Group info',
    'group.members': 'Members',
    'group.owner': 'owner',
    'group.admin': 'admin',
    'group.member': 'member',
    'group.leave': 'Leave group',
    'group.copy_link': 'Copy join link',
    'group.invite_contact': 'Invite contact',
    // Misc
    'copied': 'Copied!',
    'friend_request_sent': 'Friend request sent',
  };

  static const _fi = {
    'app.title': 'Whisper',
    'your_whisper_id': 'Sinun Whisper-tunnuksesi',
    'search_users': 'Hae käyttäjiä',
    'tab.chats': 'Keskustelut',
    'tab.contacts': 'Kontaktit',
    'tab.groups': 'Ryhmät',
    'section.conversations': 'Keskustelut',
    'section.contacts': 'Kontaktit',
    'section.groups': 'Ryhmät',
    'new_chat': 'Uusi keskustelu',
    'new_group': 'Uusi ryhmä',
    'friend_requests': 'Kaveripyynnöt',
    'group_invites': 'Ryhmäkutsut',
    'accept': 'Hyväksy',
    'decline': 'Hylkää',
    'online': 'Paikalla',
    'group': 'Ryhmä',
    'no_conversations': 'Ei keskusteluja vielä',
    'no_contacts': 'Ei kontakteja vielä',
    'no_groups': 'Ei ryhmiä vielä',
    'create_group_hint': 'Luo ryhmä +-painikkeella',
    'search_hint': 'Hae käyttäjää aloittaaksesi',
    'accept_requests_hint': 'Hyväksy kaveripyynnöt lisätäksesi kontakteja',
    'select_conversation': 'Valitse keskustelu',
    'e2ee_note': 'Viestisi ovat päästä päähän salattuja',
    'no_messages': 'Ei viestejä vielä',
    'message_hint': 'Viesti',
    'connected': 'Yhdistetty',
    'connecting': 'Yhdistetään…',
    'not_connected': 'Ei yhteyttä',
    'disconnected': 'Yhteys katkesi',
    'copy_peer_id': 'Kopioi tunnus',
    'settings': 'Asetukset',
    'reply': 'Vastaa',
    'react': 'Reagoi',
    'edit': 'Muokkaa',
    'delete': 'Poista',
    'cancel': 'Peruuta',
    'save': 'Tallenna',
    'send': 'Lähetä',
    'open': 'Avaa',
    'create': 'Luo',
    'reconnecting': 'Yhdistetään uudelleen…',
    'welcome_title': 'Tervetuloa Whisperiin',
    'welcome_sub': 'Aseta profiilisi. Voit muuttaa näitä myöhemmin Asetuksissa.',
    'display_name': 'Näyttönimi',
    'display_name_hint': 'Mitä ystävät näkevät',
    'username_optional': 'Käyttäjänimi (valinnainen)',
    'username_hint': 'pienet_kirjaimet_123',
    'continue': 'Jatka',
    'setting_up': 'Asetetaan…',
    'your_peer_id': 'SINUN TUNNUKSESI',
    'share_peer_hint': 'Jaa tämä tunnus, jotta muut voivat lisätä sinut.',
    'settings.general': 'Yleinen',
    'settings.privacy': 'Yksityisyys',
    'settings.notifications': 'Ilmoitukset',
    'settings.about': 'Tietoja',
    'settings.language': 'Kieli',
    'settings.identity': 'TUNNUS',
    'settings.identity_sub': 'Avaimet eivät koskaan poistu laitteesta. Jaa tunnuksesi, jotta muut voivat ottaa yhteyttä.',
    'settings.connection': 'YHTEYS',
    'settings.relay_server': 'Relay-palvelin',
    'settings.privacy.title': 'Yksityisyys',
    'settings.presence_visible': 'Näytä paikallaoloni',
    'settings.presence_visible_sub': 'Kun piilotettu, muut näkevät sinut aina offline-tilassa.',
    'settings.read_receipts': 'Lähetä lukukuittaukset',
    'settings.read_receipts_sub': 'Kerro muille, kun olet lukenut heidän viestinsä.',
    'settings.typing': 'Lähetä kirjoitusilmoitukset',
    'settings.typing_sub': 'Anna muiden nähdä, kun kirjoitat.',
    'settings.notifications.title': 'Ilmoitukset',
    'settings.notifications_sub': 'Ilmoitusasetukset tulevat mobiiliin pian.',
    'settings.about.title': 'Tietoja',
    'settings.e2ee': 'Päästä päähän salattu',
    'settings.e2ee_sub': 'X3DH + Double Ratchet (vodozemac)',
    'settings.zk': 'Zero-knowledge-relay',
    'settings.zk_sub': 'Näkee vain salatekstiä',
    'settings.version': 'Whisper mobile',
    'settings.version_sub': 'Flutter + jaettu Rust e2ee-core',
    'group.info': 'Ryhmän tiedot',
    'group.members': 'Jäsenet',
    'group.owner': 'omistaja',
    'group.admin': 'ylläpitäjä',
    'group.member': 'jäsen',
    'group.leave': 'Poistu ryhmästä',
    'group.copy_link': 'Kopioi kutsulinkki',
    'group.invite_contact': 'Kutsu kontakti',
    'copied': 'Kopioitu!',
    'friend_request_sent': 'Kaveripyyntö lähetetty',
  };
}

/// InheritedWidget exposing the current language to the subtree.
class LanguageScope extends InheritedWidget {
  final L10n l10n;
  final ValueChanged<String> onLanguageChanged;
  const LanguageScope({
    super.key,
    required this.l10n,
    required this.onLanguageChanged,
    required super.child,
  });

  static LanguageScope? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<LanguageScope>();

  static L10n of(BuildContext context) =>
      maybeOf(context)?.l10n ?? const L10n('en');

  @override
  bool updateShouldNotify(LanguageScope oldWidget) =>
      oldWidget.l10n.lang != l10n.lang;
}
