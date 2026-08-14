import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../i18n.dart';
import '../rust/api/whisper.dart' as core;
import '../theme.dart';

/// Settings with tabs — mirrors the desktop SettingsDialog (General, Privacy,
/// Notifications, About).
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

  @override
  void initState() {
    super.initState();
    // Privacy toggles are local-only for now (the relay supports set_privacy;
    // receipts/typing need e2ee-core payloads on send).
    _syncPrivacy();
  }

  Future<void> _syncPrivacy() async {
    // Best-effort push of the presence-visible preference.
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
                  label: t('settings.about'),
                  active: _tab == 3,
                  onTap: () => setState(() => _tab = 3),
                ),
              ],
            ),
          ),
          Expanded(
            child: switch (_tab) {
              0 => _buildGeneral(t),
              1 => _buildPrivacy(t),
              2 => _buildNotifications(t),
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
              Row(
                children: [
                  ClipRRect(
                    borderRadius: BorderRadius.circular(10),
                    child: Image.asset('assets/whisper-logo.png',
                        width: 40, height: 40),
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Text(
                      'Whisper ID',
                      style: const TextStyle(
                        color: Wp.text,
                        fontSize: 15,
                        fontWeight: FontWeight.w600,
                      ),
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
              const SizedBox(height: 8),
              SelectableText(
                widget.peerId,
                style: const TextStyle(
                  color: Wp.accent,
                  fontSize: 13,
                  fontFamily: 'monospace',
                  letterSpacing: 0.8,
                ),
              ),
              const SizedBox(height: 8),
              Text(
                t('settings.identity_sub'),
                style:
                    TextStyle(color: Wp.textFaint, fontSize: 11, height: 1.4),
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
  const _ListRow({
    required this.icon,
    required this.title,
    required this.subtitle,
    this.trailing,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
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
