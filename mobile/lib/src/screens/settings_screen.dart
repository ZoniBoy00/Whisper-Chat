import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/whisper.dart' as core;
import '../theme.dart';

/// Settings screen — mirrors the desktop SettingsDialog (General tab: identity,
/// language) in a single mobile-friendly view.
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
  bool _copied = false;

  Future<void> _copy() async {
    await Clipboard.setData(ClipboardData(text: widget.peerId));
    setState(() => _copied = true);
    Future.delayed(const Duration(milliseconds: 1600), () {
      if (mounted) setState(() => _copied = false);
    });
  }

  Future<void> _refresh() async {
    await widget.client.refreshContacts();
    await widget.client.refreshFriendRequests();
    if (mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('Contacts refreshed')),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Wp.bg,
      appBar: AppBar(
        title: const Text('Settings'),
      ),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          _SectionLabel('IDENTITY'),
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
                    const Expanded(
                      child: Text(
                        'Whisper identity',
                        style: TextStyle(
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
                  'Keys never leave this device. Share your peer ID to let '
                  'others contact you.',
                  style: TextStyle(color: Wp.textFaint, fontSize: 11, height: 1.4),
                ),
              ],
            ),
          ),
          const SizedBox(height: 20),
          _SectionLabel('CONNECTION'),
          _Card(
            child: Column(
              children: [
                _ListRow(
                  icon: Icons.wifi,
                  title: 'Relay server',
                  subtitle: 'wss://whisper-test.homelab.cfd/ws',
                  trailing: const Icon(Icons.lock, size: 16, color: Wp.textFaint),
                ),
                const Divider(color: Wp.line, height: 1),
                _ListRow(
                  icon: Icons.refresh,
                  title: 'Refresh contacts',
                  subtitle: 'Re-sync contacts and friend requests',
                  onTap: _refresh,
                ),
              ],
            ),
          ),
          const SizedBox(height: 20),
          _SectionLabel('ABOUT'),
          _Card(
            child: Column(
              children: [
                _ListRow(
                  icon: Icons.shield_outlined,
                  title: 'End-to-end encrypted',
                  subtitle: 'X3DH + Double Ratchet (vodozemac)',
                ),
                const Divider(color: Wp.line, height: 1),
                _ListRow(
                  icon: Icons.public,
                  title: 'Zero-knowledge relay',
                  subtitle: 'Sees ciphertext only',
                ),
                const Divider(color: Wp.line, height: 1),
                _ListRow(
                  icon: Icons.code,
                  title: 'Whisper mobile',
                  subtitle: 'Flutter + shared Rust e2ee-core',
                ),
              ],
            ),
          ),
        ],
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
