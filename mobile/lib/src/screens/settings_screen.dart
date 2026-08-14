import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:image_picker/image_picker.dart';
import 'package:path_provider/path_provider.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../i18n.dart';
import '../rust/api/backup.dart' as backup;
import '../rust/api/whisper.dart' as core;
import '../theme.dart';
import 'profile_screen.dart';

/// Settings with tabs — mirrors the desktop SettingsDialog (General, Privacy,
/// Notifications, Logs, About).
class SettingsScreen extends StatefulWidget {
  final core.WhisperClient client;
  final String peerId;
  const SettingsScreen({
    super.key,
    required this.client,
    required this.peerId,
  });

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  int _tab = 0;
  bool _copied = false;
  bool _presenceVisible = true;
  bool _readReceipts = true;
  bool _typing = true;
  String _username = '';
  final _log = <String>[];
  Timer? _logPoller;

  @override
  void initState() {
    super.initState();
    _loadUsername();
    _syncPrivacy();
    _logPoller = Timer.periodic(const Duration(milliseconds: 500), (_) async {
      final events = await widget.client.takeEvents();
      for (final e in events) {
        _log.add(
            '${DateTime.now().toIso8601String().substring(11, 19)} '
            '[${e.kind}] ${e.peerId} ${e.text ?? ''} ${e.error ?? ''}');
      }
      if (_log.length > 200) _log.removeRange(0, _log.length - 200);
      if (mounted) setState(() {});
    });
  }

  @override
  void dispose() {
    _logPoller?.cancel();
    super.dispose();
  }

  Future<void> _loadUsername() async {
    // No persisted username locally; a signed profile may exist on the relay.
    // Best-effort: keep it empty — the profile screen shows what the relay
    // reports.
  }

  Future<void> _syncPrivacy() async {
    await widget.client.setPrivacy(presenceVisible: _presenceVisible);
  }

  Future<void> _copy() async {
    await Clipboard.setData(ClipboardData(text: widget.peerId));
    setState(() => _copied = true);
    Future.delayed(const Duration(milliseconds: 1600), () {
      if (mounted) setState(() => _copied = false);
    });
  }

  Future<void> _changeLanguage(String lang) async {
    final scope = LanguageScope.maybeOf(context);
    await L10n.save(lang);
    scope?.onLanguageChanged(lang);
    setState(() {});
  }

  Future<void> _changeDisplayName() async {
    final controller = TextEditingController();
    final name = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: Wp.panel,
        title: const Text('Display name',
            style: TextStyle(color: Wp.text, fontSize: 17)),
        content: TextField(
          controller: controller,
          autofocus: true,
          style: const TextStyle(color: Wp.text),
          decoration: const InputDecoration(
            hintText: 'What friends will see',
            hintStyle: TextStyle(color: Wp.textFaint),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('Cancel', style: TextStyle(color: Wp.textDim)),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(ctx, controller.text.trim()),
            style: FilledButton.styleFrom(backgroundColor: Wp.accent),
            child: const Text('Save', style: TextStyle(color: Wp.accentFg)),
          ),
        ],
      ),
    );
    if (name != null && name.isNotEmpty) {
      await widget.client.setDisplayName(displayName: name);
    }
  }

  Future<void> _changeAvatar() async {
    final picker = ImagePicker();
    final image = await picker.pickImage(
        source: ImageSource.gallery, maxWidth: 512, maxHeight: 512);
    if (image == null) return;
    final bytes = await image.readAsBytes();
    if (bytes.length > 2 * 1024 * 1024) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('Avatar must be under 2 MiB')),
        );
      }
      return;
    }
    // Avatar upload reuses RegisterProfile with the avatar field — it needs
    // a username binding; if none is registered yet, show a hint.
    if (_username.isEmpty) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
              content: Text('Register a username first to upload an avatar')),
        );
      }
      return;
    }
    final prefs = await SharedPreferences.getInstance();
    final identityJson = prefs.getString('identity_json') ?? '';
    final sig = await core.signUsername(
        json: identityJson, username: _username);
    await widget.client.setAvatar(
        username: _username, signature: sig, avatarB64: base64Encode(bytes));
  }

  Future<void> _openMyProfile() async {
    final prefs = await SharedPreferences.getInstance();
    final identityJson = prefs.getString('identity_json') ?? '';
    if (!mounted) return;
    await Navigator.push(
      context,
      MaterialPageRoute(
        builder: (_) => ProfileScreen(
          client: widget.client,
          identityJson: identityJson,
          myPeerId: widget.peerId,
          peerId: widget.peerId,
          isSelf: true,
        ),
      ),
    );
  }

  /// Export a password-encrypted backup (identity + settings) to the device.
  Future<void> _exportBackup() async {
    final prefs = await SharedPreferences.getInstance();
    final identityJson = prefs.getString('identity_json') ?? '';
    final password = await _askPassword('Export backup');
    if (password == null) return;
    try {
      final body = jsonEncode({
        'identity': identityJson,
        'peer_id': widget.peerId,
        'created_at': DateTime.now().toIso8601String(),
      });
      final package = await backup.backupEncrypt(
          plaintext: body, password: password);
      // Write to the app documents directory.
      final dir = await getApplicationDocumentsDirectory();
      final file = File(
          '${dir.path}/whisper-backup-${DateTime.now().millisecondsSinceEpoch}.json');
      await file.writeAsString(package);
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Backup saved to ${file.path}')),
        );
      }
    } catch (err) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Backup failed: $err')),
        );
      }
    }
  }

  /// Import a backup: paste the package JSON + password, restore identity.
  Future<void> _importBackup() async {
    final packageController = TextEditingController();
    final password = await _askPassword('Import backup');
    if (password == null) return;
    final packageJson = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: Wp.panel,
        title: const Text('Import backup',
            style: TextStyle(color: Wp.text, fontSize: 17)),
        content: TextField(
          controller: packageController,
          maxLines: 6,
          style: const TextStyle(color: Wp.text, fontSize: 12),
          decoration: const InputDecoration(
            hintText: 'Paste the backup JSON here',
            hintStyle: TextStyle(color: Wp.textFaint),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('Cancel', style: TextStyle(color: Wp.textDim)),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(ctx, packageController.text.trim()),
            style: FilledButton.styleFrom(backgroundColor: Wp.accent),
            child: const Text('Restore', style: TextStyle(color: Wp.accentFg)),
          ),
        ],
      ),
    );
    if (packageJson == null || packageJson.isEmpty) return;
    try {
      final body = await backup.backupDecrypt(
          packageJson: packageJson, password: password);
      final parsed = jsonDecode(body) as Map<String, dynamic>;
      final identity = parsed['identity'] as String?;
      if (identity == null) throw Exception('no identity in backup');
      final prefs = await SharedPreferences.getInstance();
      await prefs.setString('identity_json', identity);
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('Backup restored — restart the app')),
        );
      }
    } catch (err) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Restore failed: $err')),
        );
      }
    }
  }

  Future<String?> _askPassword(String title) async {
    final controller = TextEditingController();
    return showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: Wp.panel,
        title: Text(title, style: const TextStyle(color: Wp.text, fontSize: 17)),
        content: TextField(
          controller: controller,
          obscureText: true,
          style: const TextStyle(color: Wp.text),
          decoration: const InputDecoration(
            hintText: 'Password (min 8 chars)',
            hintStyle: TextStyle(color: Wp.textFaint),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('Cancel', style: TextStyle(color: Wp.textDim)),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(ctx, controller.text),
            style: FilledButton.styleFrom(backgroundColor: Wp.accent),
            child: const Text('OK', style: TextStyle(color: Wp.accentFg)),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = LanguageScope.of(context).t;
    return Scaffold(
      backgroundColor: Wp.bg,
      appBar: AppBar(title: Text(t('settings'))),
      body: Column(
        children: [
          // Tabs.
          Container(
            padding: const EdgeInsets.fromLTRB(12, 8, 12, 8),
            decoration: const BoxDecoration(
              border: Border(bottom: BorderSide(color: Wp.line)),
            ),
            child: Row(
              children: [
                _SettingsTab(
                  label: t('settings.general'),
                  active: _tab == 0,
                  onTap: () => setState(() => _tab = 0),
                ),
                _SettingsTab(
                  label: t('settings.privacy'),
                  active: _tab == 1,
                  onTap: () => setState(() => _tab = 1),
                ),
                _SettingsTab(
                  label: t('settings.notifications'),
                  active: _tab == 2,
                  onTap: () => setState(() => _tab = 2),
                ),
                _SettingsTab(
                  label: 'Logs',
                  active: _tab == 3,
                  onTap: () => setState(() => _tab = 3),
                ),
                _SettingsTab(
                  label: t('settings.about'),
                  active: _tab == 4,
                  onTap: () => setState(() => _tab = 4),
                ),
              ],
            ),
          ),
          Expanded(
            child: switch (_tab) {
              0 => _buildGeneral(t),
              1 => _buildPrivacy(t),
              2 => _buildNotifications(t),
              3 => _buildLogs(),
              _ => _buildAbout(t),
            },
          ),
        ],
      ),
    );
  }

  Widget _buildGeneral(String Function(String) t) {
    return ListView(
      padding: const EdgeInsets.all(16),
      children: [
        _SectionLabel(t('settings.identity')),
        _Card(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              // My profile (opens the full profile screen).
              InkWell(
                onTap: _openMyProfile,
                child: Padding(
                  padding: const EdgeInsets.all(14),
                  child: Row(
                    children: [
                      ClipRRect(
                        borderRadius: BorderRadius.circular(10),
                        child: Image.asset('assets/whisper-logo.png',
                            width: 40, height: 40),
                      ),
                      const SizedBox(width: 12),
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Text(
                              'My profile',
                              style: const TextStyle(
                                color: Wp.text,
                                fontSize: 15,
                                fontWeight: FontWeight.w600,
                              ),
                            ),
                            Text(
                              widget.peerId,
                              style: const TextStyle(
                                color: Wp.textFaint,
                                fontSize: 11,
                                fontFamily: 'monospace',
                              ),
                            ),
                          ],
                        ),
                      ),
                      IconButton(
                        onPressed: _copy,
                        icon: Icon(
                          _copied ? Icons.check : Icons.copy,
                          size: 18,
                          color: _copied ? Wp.online : Wp.textDim,
                        ),
                      ),
                    ],
                  ),
                ),
              ),
              const Divider(color: Wp.line, height: 1),
              _ListRow(
                icon: Icons.badge_outlined,
                title: t('display_name'),
                subtitle: 'Change what friends see',
                onTap: _changeDisplayName,
              ),
              const Divider(color: Wp.line, height: 1),
              _ListRow(
                icon: Icons.photo_camera_outlined,
                title: 'Avatar',
                subtitle: 'Upload a profile picture',
                onTap: _changeAvatar,
              ),
              const Divider(color: Wp.line, height: 1),
              _ListRow(
                icon: Icons.shield_outlined,
                title: 'Safety number',
                subtitle: 'View your E2EE identity',
                onTap: _openMyProfile,
              ),
            ],
          ),
        ),
        const SizedBox(height: 20),
        _SectionLabel(t('settings.connection')),
        _Card(
          child: _ListRow(
            icon: Icons.wifi,
            title: t('settings.relay_server'),
            subtitle: 'wss://whisper-test.homelab.cfd/ws',
            trailing: const Icon(Icons.lock, size: 16, color: Wp.textFaint),
          ),
        ),
        const SizedBox(height: 20),
        _SectionLabel('BACKUP'),
        _Card(
          child: Column(
            children: [
              _ListRow(
                icon: Icons.file_download_outlined,
                title: 'Export backup',
                subtitle: 'Password-encrypted (Argon2id + AES-256-GCM)',
                onTap: _exportBackup,
              ),
              const Divider(color: Wp.line, height: 1),
              _ListRow(
                icon: Icons.file_upload_outlined,
                title: 'Import backup',
                subtitle: 'Restore identity from a backup',
                onTap: _importBackup,
              ),
            ],
          ),
        ),
        const SizedBox(height: 20),
        _SectionLabel(t('settings.language')),
        _Card(
          child: Row(
            children: [
              for (final lang in ['en', 'fi'])
                Expanded(
                  child: InkWell(
                    onTap: () => _changeLanguage(lang),
                    child: Container(
                      padding: const EdgeInsets.symmetric(vertical: 12),
                      color: LanguageScope.of(context).lang == lang
                          ? Wp.accent
                          : Colors.transparent,
                      child: Text(
                        lang == 'en' ? 'English' : 'Suomi',
                        textAlign: TextAlign.center,
                        style: TextStyle(
                          color: LanguageScope.of(context).lang == lang
                              ? Wp.accentFg
                              : Wp.textDim,
                          fontWeight: FontWeight.w600,
                          fontSize: 13,
                        ),
                      ),
                    ),
                  ),
                ),
            ],
          ),
        ),
      ],
    );
  }

  Widget _buildPrivacy(String Function(String) t) {
    return ListView(
      padding: const EdgeInsets.all(16),
      children: [
        _Card(
          child: Column(
            children: [
              _ToggleRow(
                icon: Icons.visibility_outlined,
                title: t('settings.presence_visible'),
                subtitle: t('settings.presence_visible_sub'),
                value: _presenceVisible,
                onChanged: (v) async {
                  setState(() => _presenceVisible = v);
                  await widget.client.setPrivacy(presenceVisible: v);
                },
              ),
              const Divider(color: Wp.line, height: 1),
              _ToggleRow(
                icon: Icons.done_all,
                title: t('settings.read_receipts'),
                subtitle: t('settings.read_receipts_sub'),
                value: _readReceipts,
                onChanged: (v) => setState(() => _readReceipts = v),
              ),
              const Divider(color: Wp.line, height: 1),
              _ToggleRow(
                icon: Icons.keyboard_outlined,
                title: t('settings.typing'),
                subtitle: t('settings.typing_sub'),
                value: _typing,
                onChanged: (v) => setState(() => _typing = v),
              ),
            ],
          ),
        ),
      ],
    );
  }

  Widget _buildNotifications(String Function(String) t) {
    return ListView(
      padding: const EdgeInsets.all(16),
      children: [
        _Card(
          child: Padding(
            padding: const EdgeInsets.all(16),
            child: Text(
              t('settings.notifications_sub'),
              style: TextStyle(color: Wp.textDim, fontSize: 13),
            ),
          ),
        ),
      ],
    );
  }

  /// Live event log — mirrors the desktop Logs tab (debugging aid).
  Widget _buildLogs() {
    if (_log.isEmpty) {
      return Center(
        child: Text(
          'No log entries yet',
          style: TextStyle(color: Wp.textFaint, fontSize: 13),
        ),
      );
    }
    return ListView.builder(
      padding: const EdgeInsets.all(12),
      itemCount: _log.length,
      itemBuilder: (context, i) => Padding(
        padding: const EdgeInsets.symmetric(vertical: 2),
        child: Text(
          _log[i],
          style: const TextStyle(
            color: Wp.textDim,
            fontSize: 11,
            fontFamily: 'monospace',
          ),
        ),
      ),
    );
  }

  Widget _buildAbout(String Function(String) t) {
    return ListView(
      padding: const EdgeInsets.all(16),
      children: [
        _Card(
          child: Column(
            children: [
              _ListRow(
                icon: Icons.shield_outlined,
                title: t('settings.e2ee'),
                subtitle: t('settings.e2ee_sub'),
              ),
              const Divider(color: Wp.line, height: 1),
              _ListRow(
                icon: Icons.public,
                title: t('settings.zk'),
                subtitle: t('settings.zk_sub'),
              ),
              const Divider(color: Wp.line, height: 1),
              _ListRow(
                icon: Icons.code,
                title: t('settings.version'),
                subtitle: t('settings.version_sub'),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class _SettingsTab extends StatelessWidget {
  final String label;
  final bool active;
  final VoidCallback onTap;
  const _SettingsTab({
    required this.label,
    required this.active,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return Expanded(
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(10),
        child: Container(
          padding: const EdgeInsets.symmetric(vertical: 8),
          decoration: BoxDecoration(
            color: active ? Wp.panel3 : Colors.transparent,
            borderRadius: BorderRadius.circular(10),
          ),
          child: Text(
            label,
            textAlign: TextAlign.center,
            style: TextStyle(
              color: active ? Wp.text : Wp.textDim,
              fontSize: 12,
              fontWeight: FontWeight.w600,
            ),
          ),
        ),
      ),
    );
  }
}

class _SectionLabel extends StatelessWidget {
  final String text;
  const _SectionLabel(this.text);

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(left: 4, bottom: 8),
      child: Text(
        text,
        style: TextStyle(
          color: Wp.textFaint,
          fontSize: 10,
          fontWeight: FontWeight.w700,
          letterSpacing: 1.2,
        ),
      ),
    );
  }
}

class _Card extends StatelessWidget {
  final Widget child;
  const _Card({required this.child});

  @override
  Widget build(BuildContext context) {
    return Container(
      decoration: BoxDecoration(
        color: Wp.panel,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: Wp.line),
      ),
      child: child,
    );
  }
}

class _ListRow extends StatelessWidget {
  final IconData icon;
  final String title;
  final String subtitle;
  final Widget? trailing;
  final VoidCallback? onTap;
  const _ListRow({
    required this.icon,
    required this.title,
    required this.subtitle,
    this.trailing,
    this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
        child: Row(
          children: [
            Icon(icon, size: 20, color: Wp.accent),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    title,
                    style: const TextStyle(
                      color: Wp.text,
                      fontSize: 13,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  const SizedBox(height: 1),
                  Text(
                    subtitle,
                    style: TextStyle(color: Wp.textFaint, fontSize: 11),
                  ),
                ],
              ),
            ),
            ?trailing,
          ],
        ),
      ),
    );
  }
}

class _ToggleRow extends StatelessWidget {
  final IconData icon;
  final String title;
  final String subtitle;
  final bool value;
  final ValueChanged<bool> onChanged;
  const _ToggleRow({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.value,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
      child: Row(
        children: [
          Icon(icon, size: 20, color: Wp.accent),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  title,
                  style: const TextStyle(
                    color: Wp.text,
                    fontSize: 13,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                Text(
                  subtitle,
                  style: TextStyle(color: Wp.textFaint, fontSize: 11),
                ),
              ],
            ),
          ),
          Switch(
            value: value,
            onChanged: onChanged,
            activeTrackColor: Wp.accent,
          ),
        ],
      ),
    );
  }
}
