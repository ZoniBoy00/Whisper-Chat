import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../i18n.dart';
import '../rust/api/whisper.dart' as core;
import '../theme.dart';
import '../widgets/avatar.dart';

/// Group info: roster with roles + actions (invite, join link, leave).
/// Fetches `get_group_info` on open and listens for `group_info` events.
class GroupInfoScreen extends StatefulWidget {
  final core.WhisperClient client;
  final String groupId;
  final String name;
  const GroupInfoScreen({
    super.key,
    required this.client,
    required this.groupId,
    required this.name,
  });

  @override
  State<GroupInfoScreen> createState() => _GroupInfoScreenState();
}

class _GroupInfoScreenState extends State<GroupInfoScreen> {
  final _members = <(String, String)>[]; // (peer_id, role)
  String _owner = '';
  String _name = '';
  String? _joinLink;
  bool _copied = false;
  bool _loaded = false;

  @override
  void initState() {
    super.initState();
    _name = widget.name;
    _load();
  }

  Future<void> _load() async {
    await widget.client.getGroupInfo(groupId: widget.groupId);
    await widget.client.getGroupJoinLink(groupId: widget.groupId);
    // Poll for the group_info reply.
    for (var i = 0; i < 20; i++) {
      await Future.delayed(const Duration(milliseconds: 250));
      final events = await widget.client.takeEvents();
      var gotInfo = false;
      for (final e in events) {
        if (e.kind == 'group_info' && e.peerId == widget.groupId) {
          final parts = (e.text ?? '').split('|');
          if (parts.isNotEmpty) _name = parts[0];
          if (parts.length > 1) _owner = parts[1];
          if (parts.length > 2 && parts[2].isNotEmpty) {
            _members
              ..clear()
              ..addAll(parts[2]
                  .split('\n')
                  .where((l) => l.isNotEmpty)
                  .map((l) {
                    final p = l.split(':');
                    return (p.length > 0 ? p[0] : '', p.length > 1 ? p[1] : 'member');
                  }));
          }
          gotInfo = true;
        }
        if (e.kind == 'group_join_link') {
          _joinLink = e.text;
        }
      }
      if (gotInfo) break;
    }
    if (mounted) setState(() => _loaded = true);
  }

  Future<void> _invite() async {
    final controller = TextEditingController();
    final peer = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: Wp.panel,
        title: const Text('Invite contact',
            style: TextStyle(color: Wp.text, fontSize: 17)),
        content: TextField(
          controller: controller,
          autofocus: true,
          style: const TextStyle(color: Wp.text),
          decoration: const InputDecoration(
            hintText: 'Peer ID (24 hex)',
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
            child: const Text('Invite', style: TextStyle(color: Wp.accentFg)),
          ),
        ],
      ),
    );
    if (peer != null && peer.isNotEmpty) {
      await widget.client.inviteToGroup(groupId: widget.groupId, peerId: peer);
    }
  }

  Future<void> _copyLink() async {
    final link = _joinLink;
    if (link == null) return;
    await Clipboard.setData(ClipboardData(text: link));
    setState(() => _copied = true);
    Future.delayed(const Duration(milliseconds: 1600), () {
      if (mounted) setState(() => _copied = false);
    });
  }

  Future<void> _leave() async {
    await widget.client.leaveGroup(groupId: widget.groupId);
    if (mounted) Navigator.pop(context);
  }

  @override
  Widget build(BuildContext context) {
    final t = LanguageScope.of(context).t;
    return Scaffold(
      backgroundColor: Wp.bg,
      appBar: AppBar(title: Text(_name)),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          // Group header tile.
          Center(
            child: Column(
              children: [
                WpAvatar(name: _name, size: 72, group: true),
                const SizedBox(height: 12),
                Text(
                  _name,
                  style: const TextStyle(
                    color: Wp.text,
                    fontSize: 20,
                    fontWeight: FontWeight.w700,
                  ),
                ),
                Text(
                  '${_members.length} members',
                  style: TextStyle(color: Wp.textFaint, fontSize: 12),
                ),
              ],
            ),
          ),
          const SizedBox(height: 20),
          // Actions.
          Row(
            children: [
              Expanded(
                child: OutlinedButton.icon(
                  onPressed: _invite,
                  style: OutlinedButton.styleFrom(
                    foregroundColor: Wp.textDim,
                    side: const BorderSide(color: Wp.line),
                  ),
                  icon: const Icon(Icons.person_add_alt, size: 16),
                  label: Text(t('group.invite_contact'),
                      style: const TextStyle(fontSize: 12)),
                ),
              ),
            ],
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              Expanded(
                child: OutlinedButton.icon(
                  onPressed: _copyLink,
                  style: OutlinedButton.styleFrom(
                    foregroundColor: Wp.textDim,
                    side: const BorderSide(color: Wp.line),
                  ),
                  icon: Icon(
                    _copied ? Icons.check : Icons.link,
                    size: 16,
                    color: _copied ? Wp.online : Wp.textDim,
                  ),
                  label: Text(_copied ? t('copied') : t('group.copy_link'),
                      style: const TextStyle(fontSize: 12)),
                ),
              ),
            ],
          ),
          const SizedBox(height: 8),
          OutlinedButton.icon(
            onPressed: _leave,
            style: OutlinedButton.styleFrom(
              foregroundColor: Wp.danger,
              side: const BorderSide(color: Wp.line),
            ),
            icon: const Icon(Icons.logout, size: 16),
            label: Text(t('group.leave'), style: const TextStyle(fontSize: 12)),
          ),
          const SizedBox(height: 24),
          // Members.
          Text(
            t('group.members').toUpperCase(),
            style: const TextStyle(
              color: Wp.textFaint,
              fontSize: 10,
              fontWeight: FontWeight.w700,
              letterSpacing: 1.2,
            ),
          ),
          const SizedBox(height: 8),
          if (!_loaded)
            const Center(
              child: Padding(
                padding: EdgeInsets.all(20),
                child: CircularProgressIndicator(color: Wp.accent),
              ),
            )
          else
            for (final (peer, role) in _members)
              Container(
                margin: const EdgeInsets.only(bottom: 6),
                padding:
                    const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
                decoration: BoxDecoration(
                  color: Wp.panel,
                  borderRadius: BorderRadius.circular(12),
                  border: Border.all(color: Wp.line),
                ),
                child: Row(
                  children: [
                    WpAvatar(name: peer, size: 34),
                    const SizedBox(width: 10),
                    Expanded(
                      child: Text(
                        peer.length > 26 ? '${peer.substring(0, 26)}…' : peer,
                        style: const TextStyle(
                          color: Wp.text,
                          fontSize: 13,
                          fontFamily: 'monospace',
                        ),
                      ),
                    ),
                    if (role == 'owner')
                      _RoleBadge(role: t('group.owner'), color: Wp.accent)
                    else if (role == 'admin')
                      _RoleBadge(role: t('group.admin'), color: Wp.online)
                    else
                      _RoleBadge(role: t('group.member'), color: Wp.textFaint),
                    if (peer == _owner)
                      const Padding(
                        padding: EdgeInsets.only(left: 6),
                        child: Icon(Icons.workspace_premium,
                            size: 14, color: Wp.accent),
                      ),
                  ],
                ),
              ),
        ],
      ),
    );
  }
}

class _RoleBadge extends StatelessWidget {
  final String role;
  final Color color;
  const _RoleBadge({required this.role, required this.color});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.12),
        borderRadius: BorderRadius.circular(6),
      ),
      child: Text(
        role,
        style: TextStyle(
          color: color,
          fontSize: 10,
          fontWeight: FontWeight.w600,
        ),
      ),
    );
  }
}
