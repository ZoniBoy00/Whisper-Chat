import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../rust/api/whisper.dart' as core;
import '../theme.dart';
import '../widgets/avatar.dart';
import 'settings_screen.dart';

/// One chat line in a conversation (1:1 or group).
class ChatLine {
  final String peer;
  String text;
  final bool outgoing;
  final DateTime at;
  String? messageId;
  String? quoteText;
  String? expiresIn;
  String? edited;
  String? reaction;
  ChatLine(
    this.peer,
    this.text, {
    required this.outgoing,
    this.messageId,
    this.quoteText,
    this.expiresIn,
    this.edited,
    this.reaction,
  }) : at = DateTime.now();
}

/// A directory search hit (username | peer_id | display_name | avatar).
class SearchHit {
  final String username;
  final String peerId;
  final String displayName;
  SearchHit(this.username, this.peerId, this.displayName);
}

/// A group we belong to (group_id -> name).
class WhisperGroup {
  final String id;
  final String name;
  WhisperGroup(this.id, this.name);
}

class MainScreen extends StatefulWidget {
  final core.WhisperClient client;
  final String peerId;
  const MainScreen({
    super.key,
    required this.client,
    required this.peerId,
  });

  @override
  State<MainScreen> createState() => _MainScreenState();
}

enum _View { chats, contacts, groups }

class _MainScreenState extends State<MainScreen> {
  final _messages = <String, List<ChatLine>>{};
  final _presence = <String, String>{};
  final _groups = <WhisperGroup>[];
  final _groupInvites = <String>[]; // "groupId|name|inviter"
  String? _activePeer;
  bool _connected = false;
  String _status = 'Not connected';
  List<String> _contacts = [];
  List<String> _pendingRequests = [];
  _View _view = _View.chats;
  Timer? _poller;
  final _searchCtrl = TextEditingController();
  String _query = '';
  List<SearchHit> _searchResults = [];
  bool _searching = false;

  @override
  void initState() {
    super.initState();
    _connect();
    _poller = Timer.periodic(const Duration(milliseconds: 500), (_) async {
      final events = await widget.client.takeEvents();
      for (final e in events) {
        _handleEvent(e);
      }
    });
  }

  @override
  void dispose() {
    _poller?.cancel();
    _searchCtrl.dispose();
    super.dispose();
  }

  Future<void> _connect() async {
    setState(() => _status = 'Connecting…');
    try {
      final prefs = await SharedPreferences.getInstance();
      final json = prefs.getString('identity_json') ?? '';
      await widget.client.connect(relayUrl: null, identityJson: json);
      setState(() {
        _connected = true;
        _status = 'Connected';
      });
      await widget.client.refreshContacts();
      await widget.client.refreshFriendRequests();
      await widget.client.refreshGroupInvites();
    } catch (err) {
      setState(() => _status = '$err');
    }
  }

  void _handleEvent(core.ChatEvent e) {
    setState(() {
      switch (e.kind) {
        case 'connected':
          _connected = true;
          _status = 'Connected';
        case 'disconnected':
          _connected = false;
          _status = 'Disconnected';
        case 'message':
          _messages
              .putIfAbsent(e.peerId, () => [])
              .add(ChatLine(e.peerId, e.text ?? '', outgoing: false));
        case 'message_sent':
          _messages
              .putIfAbsent(e.peerId, () => [])
              .add(ChatLine(e.peerId, e.text ?? '', outgoing: true));
        case 'message_quote':
          _applyMeta(e.peerId, (m) => m.quoteText = e.text);
        case 'message_meta':
          final parts = (e.text ?? '').split('|');
          if (parts.isNotEmpty && parts[0].isNotEmpty) {
            _applyMeta(e.peerId, (m) => m.messageId = parts[0]);
          }
          if (parts.length > 1 && parts[1].isNotEmpty) {
            _applyMeta(e.peerId, (m) => m.expiresIn = parts[1]);
          }
        case 'reaction':
          final parts = (e.text ?? '').split('|');
          if (parts.length == 2) {
            _applyMeta(e.peerId, (m) => m.reaction = '${parts[1]} ');
          }
        case 'message_edited':
          final parts = (e.text ?? '').split('|');
          if (parts.length == 2) {
            _applyMeta(e.peerId, (m) {
              m.text = parts[1];
              m.edited = 'edited';
            });
          }
        case 'message_deleted':
          _removeByMessageId(e.peerId, e.text ?? '');
        case 'contacts':
          _contacts =
              (e.text ?? '').split('\n').where((p) => p.isNotEmpty).toList();
        case 'friend_requests':
          _pendingRequests =
              (e.text ?? '').split('\n').where((p) => p.isNotEmpty).toList();
        case 'presence_online':
          _presence[e.peerId] = 'Online';
        case 'presence_offline':
          _presence[e.peerId] =
              'Last seen ${_formatLastSeen(e.text)}';
        case 'search_results':
          _searching = false;
          _searchResults = (e.text ?? '')
              .split('\n')
              .where((l) => l.isNotEmpty)
              .map((l) {
            final p = l.split('|');
            return SearchHit(
              p.length > 0 ? p[0] : '',
              p.length > 1 ? p[1] : '',
              p.length > 2 ? p[2] : '',
            );
          }).toList();
        case 'group_created':
          _groups.add(WhisperGroup(e.peerId, e.text ?? ''));
        case 'group_info':
          final p = (e.text ?? '').split('|');
          final existing = _groups.indexWhere((g) => g.id == e.peerId);
          if (existing >= 0) {
            _groups[existing] = WhisperGroup(e.peerId, p.isNotEmpty ? p[0] : e.peerId);
          } else {
            _groups.add(WhisperGroup(e.peerId, p.isNotEmpty ? p[0] : e.peerId));
          }
        case 'group_invite_received':
          final p = (e.text ?? '').split('|');
          _groupInvites.add('${e.peerId}|${p.isNotEmpty ? p[0] : ''}|${p.length > 1 ? p[1] : ''}');
        case 'group_invites':
          _groupInvites
            ..clear()
            ..addAll((e.text ?? '')
                .split('\n')
                .where((l) => l.isNotEmpty));
        case 'group_renamed':
          final existing = _groups.indexWhere((g) => g.id == e.peerId);
          if (existing >= 0) {
            _groups[existing] = WhisperGroup(e.peerId, e.text ?? '');
          }
        case 'error':
          _status = e.error ?? 'Error';
        default:
          break;
      }
    });
  }

  void _applyMeta(String peer, void Function(ChatLine) apply) {
    final list = _messages[peer];
    if (list != null && list.isNotEmpty) {
      apply(list.last);
    }
  }

  void _removeByMessageId(String peer, String messageId) {
    final list = _messages[peer];
    if (list == null) return;
    list.removeWhere((m) => m.messageId == messageId);
  }

  String _formatLastSeen(String? unixSeconds) {
    final t = int.tryParse(unixSeconds ?? '');
    if (t == null || t <= 0) return 'recently';
    final dt = DateTime.fromMillisecondsSinceEpoch(t * 1000);
    return '${dt.day}.${dt.month}. ${dt.hour}:${dt.minute.toString().padLeft(2, '0')}';
  }

  Future<void> _openChat(String peer) async {
    setState(() => _activePeer = peer);
    await widget.client.watchPresence(peerId: peer);
    await widget.client.getProfile(peerId: peer);
  }

  Future<void> _acceptRequest(String peer) async {
    await widget.client.acceptFriendRequest(peer: peer);
    await widget.client.refreshContacts();
    await widget.client.refreshFriendRequests();
  }

  Future<void> _declineRequest(String peer) async {
    await widget.client.declineFriendRequest(peer: peer);
    await widget.client.refreshFriendRequests();
  }

  Future<void> _acceptGroupInvite(String entry) async {
    final parts = entry.split('|');
    if (parts.isEmpty) return;
    await widget.client.acceptGroupInvite(groupId: parts[0]);
    await widget.client.refreshGroupInvites();
    await widget.client.getGroupInfo(groupId: parts[0]);
  }

  Future<void> _declineGroupInvite(String entry) async {
    final parts = entry.split('|');
    if (parts.isEmpty) return;
    await widget.client.declineGroupInvite(groupId: parts[0]);
    await widget.client.refreshGroupInvites();
  }

  Future<void> _onSearch(String query) async {
    setState(() {
      _query = query;
      _searching = query.trim().length >= 3;
      if (!_searching) _searchResults = [];
    });
    if (_searching) {
      await widget.client.searchUsers(query: query.trim());
    }
  }

  Future<void> _startChatWith(SearchHit hit) async {
    // Starting a chat with a non-contact triggers a friend request first.
    if (!_contacts.contains(hit.peerId)) {
      await widget.client.sendFriendRequest(target: hit.peerId);
      _status = 'Friend request sent to ${hit.username}';
    }
    _openChat(hit.peerId);
    setState(() {
      _query = '';
      _searchResults = [];
      _searchCtrl.clear();
    });
  }

  Future<void> _newGroup() async {
    final controller = TextEditingController();
    final name = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: Wp.panel,
        title: const Text('New group',
            style: TextStyle(color: Wp.text, fontSize: 17)),
        content: TextField(
          controller: controller,
          autofocus: true,
          style: const TextStyle(color: Wp.text),
          decoration: const InputDecoration(
            hintText: 'Group name',
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
            child: const Text('Create', style: TextStyle(color: Wp.accentFg)),
          ),
        ],
      ),
    );
    if (name != null && name.isNotEmpty) {
      await widget.client.createGroup(name: name);
    }
  }

  Future<void> _send(String text) async {
    final peer = _activePeer;
    if (peer == null || text.trim().isEmpty) return;
    if (peer.startsWith('group:')) {
      await widget.client.sendGroupMessage(
          groupId: peer.substring(6), text: text.trim());
    } else {
      await widget.client.sendMessage(peerId: peer, text: text.trim());
    }
  }

  Future<void> _sendQuote(String text, ChatLine target) async {
    final peer = _activePeer;
    if (peer == null || text.trim().isEmpty) return;
    final quote = '${target.peer}|${target.text}';
    await widget.client.sendMessageFull(
      peerId: peer,
      text: text.trim(),
      quote: quote,
      messageId: null,
      expiresInSeconds: null,
    );
  }

  Future<void> _react(ChatLine msg, String emoji) async {
    final peer = _activePeer;
    if (peer == null) return;
    await widget.client.sendReaction(
        peerId: peer, messageId: msg.messageId ?? '', emoji: emoji);
  }

  Future<void> _edit(ChatLine msg) async {
    final controller = TextEditingController(text: msg.text);
    final text = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: Wp.panel,
        title: const Text('Edit message',
            style: TextStyle(color: Wp.text, fontSize: 17)),
        content: TextField(
          controller: controller,
          style: const TextStyle(color: Wp.text),
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
    final peer = _activePeer;
    if (text != null && peer != null && msg.messageId != null) {
      await widget.client.editMessage(
          peerId: peer, messageId: msg.messageId!, text: text);
    }
  }

  Future<void> _delete(ChatLine msg) async {
    final peer = _activePeer;
    if (peer == null || msg.messageId == null) return;
    await widget.client.deleteMessage(
        peerId: peer, messageId: msg.messageId!);
  }

  Future<void> _openSettings() async {
    await Navigator.push(
      context,
      MaterialPageRoute(
        builder: (_) =>
            SettingsScreen(client: widget.client, peerId: widget.peerId),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Wp.bg,
      body: SafeArea(
        child: Row(
          children: [
            _buildSidebar(),
            VerticalDivider(width: 1, color: Wp.line),
            Expanded(child: _buildChatPane()),
          ],
        ),
      ),
    );
  }

  // ---------------------------------------------------------------------
  // Sidebar (350px)
  // ---------------------------------------------------------------------

  Widget _buildSidebar() {
    return Container(
      width: 350,
      color: Wp.panel,
      child: Column(
        children: [
          _buildProfileHeader(),
          _buildSearch(),
          _buildViewTabs(),
          _buildSectionHeader(),
          _buildRequests(),
          _buildGroupInvites(),
          Expanded(child: _buildList()),
          _buildConnectionFooter(),
        ],
      ),
    );
  }

  Widget _buildProfileHeader() {
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 16, 12, 12),
      child: Row(
        children: [
          const WpAvatar(name: '', size: 40),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'Your Whisper ID',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    color: Wp.text,
                    fontSize: 14,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                Text(
                  widget.peerId,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    color: Wp.textDim,
                    fontSize: 12,
                    fontFamily: 'monospace',
                  ),
                ),
              ],
            ),
          ),
          _IconBtn(
            icon: Icons.copy,
            tooltip: 'Copy peer ID',
            onTap: () =>
                Clipboard.setData(ClipboardData(text: widget.peerId)),
          ),
          _IconBtn(
            icon: Icons.settings_outlined,
            tooltip: 'Settings',
            onTap: _openSettings,
          ),
        ],
      ),
    );
  }

  Widget _buildSearch() {
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 0, 16, 12),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
        decoration: BoxDecoration(
          color: Wp.panel2,
          borderRadius: BorderRadius.circular(12),
        ),
        child: Row(
          children: [
            Icon(
              _searching ? Icons.hourglass_top : Icons.search,
              size: 16,
              color: Wp.textFaint,
            ),
            const SizedBox(width: 8),
            Expanded(
              child: TextField(
                controller: _searchCtrl,
                onChanged: _onSearch,
                style: const TextStyle(color: Wp.text, fontSize: 14),
                decoration: const InputDecoration(
                  isCollapsed: true,
                  hintText: 'Search users',
                  hintStyle: TextStyle(color: Wp.textFaint),
                  border: InputBorder.none,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildViewTabs() {
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 0, 16, 10),
      child: Row(
        children: [
          Expanded(
            child: _TabBtn(
              label: 'Chats',
              active: _view == _View.chats,
              onTap: () => setState(() => _view = _View.chats),
            ),
          ),
          const SizedBox(width: 4),
          Expanded(
            child: _TabBtn(
              label: 'Contacts',
              active: _view == _View.contacts,
              onTap: () => setState(() => _view = _View.contacts),
            ),
          ),
          const SizedBox(width: 4),
          Expanded(
            child: _TabBtn(
              label: 'Groups',
              active: _view == _View.groups,
              onTap: () => setState(() => _view = _View.groups),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildSectionHeader() {
    final label = switch (_view) {
      _View.chats => 'Conversations',
      _View.contacts => 'Contacts',
      _View.groups => 'Groups',
    };
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
      child: Row(
        children: [
          Text(
            label,
            style: const TextStyle(
              color: Wp.textFaint,
              fontSize: 12,
              fontWeight: FontWeight.w700,
              letterSpacing: 1.2,
            ),
          ),
          const Spacer(),
          if (_view == _View.groups)
            _IconBtn(
              icon: Icons.group_add,
              tooltip: 'New group',
              onTap: _newGroup,
            ),
          if (_view == _View.chats)
            _IconBtn(
              icon: Icons.edit_square,
              tooltip: 'New chat',
              onTap: () => _startNewChat(),
            ),
        ],
      ),
    );
  }

  Widget _buildRequests() {
    if (_pendingRequests.isEmpty) return const SizedBox.shrink();
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.fromLTRB(8, 0, 8, 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Padding(
            padding: EdgeInsets.symmetric(horizontal: 4, vertical: 6),
            child: Text(
              'FRIEND REQUESTS',
              style: TextStyle(
                color: Wp.accent,
                fontSize: 12,
                fontWeight: FontWeight.w700,
                letterSpacing: 1.2,
              ),
            ),
          ),
          for (final p in _pendingRequests)
            Container(
              margin: const EdgeInsets.only(bottom: 2),
              padding:
                  const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
              child: Row(
                children: [
                  WpAvatar(name: p, size: 36),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          _short(p, 16),
                          style: const TextStyle(
                            color: Wp.text,
                            fontSize: 14,
                            fontWeight: FontWeight.w500,
                          ),
                        ),
                        Text(
                          _short(p, 16),
                          style: const TextStyle(
                            color: Wp.textFaint,
                            fontSize: 12,
                            fontFamily: 'monospace',
                          ),
                        ),
                      ],
                    ),
                  ),
                  _IconBtn(
                    icon: Icons.check,
                    tooltip: 'Accept',
                    hoverColor: Wp.online,
                    onTap: () => _acceptRequest(p),
                  ),
                  _IconBtn(
                    icon: Icons.close,
                    tooltip: 'Decline',
                    hoverColor: Wp.danger,
                    onTap: () => _declineRequest(p),
                  ),
                ],
              ),
            ),
        ],
      ),
    );
  }

  Widget _buildGroupInvites() {
    if (_groupInvites.isEmpty) return const SizedBox.shrink();
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.fromLTRB(8, 0, 8, 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Padding(
            padding: EdgeInsets.symmetric(horizontal: 4, vertical: 6),
            child: Text(
              'GROUP INVITES',
              style: TextStyle(
                color: Wp.accent,
                fontSize: 12,
                fontWeight: FontWeight.w700,
                letterSpacing: 1.2,
              ),
            ),
          ),
          for (final entry in _groupInvites)
            Builder(builder: (context) {
              final parts = entry.split('|');
              final name = parts.length > 1 ? parts[1] : 'Group';
              return Container(
                margin: const EdgeInsets.only(bottom: 2),
                padding:
                    const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                child: Row(
                  children: [
                    WpAvatar(name: name, size: 36, group: true),
                    const SizedBox(width: 10),
                    Expanded(
                      child: Text(
                        name,
                        style: const TextStyle(
                          color: Wp.text,
                          fontSize: 14,
                          fontWeight: FontWeight.w500,
                        ),
                      ),
                    ),
                    _IconBtn(
                      icon: Icons.check,
                      tooltip: 'Accept',
                      hoverColor: Wp.online,
                      onTap: () => _acceptGroupInvite(entry),
                    ),
                    _IconBtn(
                      icon: Icons.close,
                      tooltip: 'Decline',
                      hoverColor: Wp.danger,
                      onTap: () => _declineGroupInvite(entry),
                    ),
                  ],
                ),
              );
            }),
        ],
      ),
    );
  }

  Widget _buildList() {
    final q = _query.trim().toLowerCase();
    if (q.isNotEmpty && _searching) {
      // Server search results override the local list.
      if (_searchResults.isEmpty && !_searching) {
        return const SizedBox.shrink();
      }
      return ListView.builder(
        itemCount: _searchResults.length,
        itemBuilder: (context, i) {
          final hit = _searchResults[i];
          return _ConversationRow(
            peerId: hit.peerId,
            name: hit.displayName.isNotEmpty
                ? hit.displayName
                : hit.username,
            subtitle: '@${hit.username}',
            lastText: null,
            lastTime: null,
            active: false,
            online: false,
            onTap: () => _startChatWith(hit),
          );
        },
      );
    }

    if (_view == _View.groups) {
      if (_groups.isEmpty) {
        return _EmptyHint(
          icon: Icons.group_outlined,
          title: 'No groups yet',
          subtitle: 'Create one with the + button',
        );
      }
      return ListView.builder(
        itemCount: _groups.length,
        itemBuilder: (context, i) {
          final g = _groups[i];
          return _ConversationRow(
            peerId: 'group:${g.id}',
            name: g.name,
            subtitle: 'Group',
            lastText: null,
            lastTime: null,
            active: _activePeer == 'group:${g.id}',
            online: false,
            onTap: () => _openChat('group:${g.id}'),
          );
        },
      );
    }

    List<String> ids;
    if (_view == _View.contacts) {
      ids = _contacts.toList();
    } else {
      ids = {..._contacts, ..._messages.keys}.toList();
    }
    ids = ids.where((id) => q.isEmpty || id.contains(q)).toList();
    if (ids.isEmpty) {
      return _EmptyHint(
        icon: _view == _View.chats ? Icons.chat_bubble_outline : Icons.people_outline,
        title: _view == _View.chats ? 'No conversations yet' : 'No contacts yet',
        subtitle: _view == _View.chats
            ? 'Search for a user to start'
            : 'Accept friend requests to add contacts',
      );
    }
    return ListView.builder(
      itemCount: ids.length,
      itemBuilder: (context, i) {
        final id = ids[i];
        final msgs = _messages[id] ?? [];
        final last = msgs.isEmpty ? null : msgs.last;
        return _ConversationRow(
          peerId: id,
          name: _short(id, 24),
          subtitle: null,
          lastText: last?.text,
          lastTime: last?.at,
          active: id == _activePeer,
          online: _presence[id] == 'Online',
          onTap: () => _openChat(id),
        );
      },
    );
  }

  Future<void> _startNewChat() async {
    final controller = TextEditingController();
    final peer = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: Wp.panel,
        title: const Text('New chat',
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
            child: const Text('Open', style: TextStyle(color: Wp.accentFg)),
          ),
        ],
      ),
    );
    if (peer != null && peer.isNotEmpty) _openChat(peer);
  }

  Widget _buildConnectionFooter() {
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
      decoration: const BoxDecoration(
        border: Border(top: BorderSide(color: Wp.line)),
      ),
      child: Row(
        children: [
          Container(
            width: 8,
            height: 8,
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              color: _connected ? Wp.online : Wp.textFaint,
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              _status,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                color: _connected ? Wp.online : Wp.textFaint,
                fontSize: 11,
              ),
            ),
          ),
        ],
      ),
    );
  }

  // ---------------------------------------------------------------------
  // Chat pane
  // ---------------------------------------------------------------------

  Widget _buildChatPane() {
    final peer = _activePeer;
    if (peer == null) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: 80,
              height: 80,
              decoration: BoxDecoration(
                color: Wp.panel,
                borderRadius: BorderRadius.circular(24),
              ),
              child: Icon(Icons.chat_bubble_outline,
                  size: 32, color: Wp.textFaint),
            ),
            const SizedBox(height: 16),
            Text(
              'Select a conversation',
              style: TextStyle(color: Wp.textFaint, fontSize: 14),
            ),
            const SizedBox(height: 4),
            Text(
              'Your messages are end-to-end encrypted',
              style: TextStyle(color: Wp.textFaint, fontSize: 12),
            ),
          ],
        ),
      );
    }
    final msgs = _messages[peer] ?? [];
    return Column(
      children: [
        _buildChatHeader(peer),
        Expanded(
          child: msgs.isEmpty
              ? Center(
                  child: Text(
                    'No messages yet',
                    style: TextStyle(color: Wp.textFaint, fontSize: 13),
                  ),
                )
              : ListView.builder(
                  padding:
                      const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
                  itemCount: msgs.length,
                  itemBuilder: (context, i) {
                    final m = msgs[i];
                    return _Bubble(
                      msg: m,
                      onLongPress: () => _messageActions(m),
                    );
                  },
                ),
        ),
        _Composer(
          onSend: _send,
          onSendQuote: (text) {
            final msgs = _messages[peer];
            if (msgs != null && msgs.isNotEmpty) {
              _sendQuote(text, msgs.last);
            }
          },
        ),
      ],
    );
  }

  Future<void> _messageActions(ChatLine m) async {
    final peer = _activePeer;
    if (peer == null) return;
    final action = await showModalBottomSheet<String>(
      context: context,
      backgroundColor: Wp.panel,
      builder: (ctx) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              leading: const Icon(Icons.sentiment_satisfied_alt,
                  color: Wp.accent),
              title: const Text('React', style: TextStyle(color: Wp.text)),
              onTap: () => Navigator.pop(ctx, 'react'),
            ),
            ListTile(
              leading: const Icon(Icons.reply, color: Wp.accent),
              title: const Text('Reply', style: TextStyle(color: Wp.text)),
              onTap: () => Navigator.pop(ctx, 'reply'),
            ),
            if (m.outgoing)
              ListTile(
                leading: const Icon(Icons.edit, color: Wp.accent),
                title: const Text('Edit', style: TextStyle(color: Wp.text)),
                onTap: () => Navigator.pop(ctx, 'edit'),
              ),
            if (m.outgoing)
              ListTile(
                leading: const Icon(Icons.delete, color: Wp.danger),
                title: const Text('Delete', style: TextStyle(color: Wp.danger)),
                onTap: () => Navigator.pop(ctx, 'delete'),
              ),
          ],
        ),
      ),
    );
    switch (action) {
      case 'react':
        final emoji = await _pickEmoji();
        if (emoji != null) _react(m, emoji);
      case 'reply':
        final controller = TextEditingController();
        final text = await showDialog<String>(
          context: context,
          builder: (ctx) => AlertDialog(
            backgroundColor: Wp.panel,
            title: Text(
              'Reply to ${_short(m.peer, 12)}',
              style: const TextStyle(color: Wp.text, fontSize: 16),
            ),
            content: TextField(
              controller: controller,
              autofocus: true,
              style: const TextStyle(color: Wp.text),
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(ctx),
                child:
                    const Text('Cancel', style: TextStyle(color: Wp.textDim)),
              ),
              FilledButton(
                onPressed: () => Navigator.pop(ctx, controller.text.trim()),
                style: FilledButton.styleFrom(backgroundColor: Wp.accent),
                child: const Text('Send', style: TextStyle(color: Wp.accentFg)),
              ),
            ],
          ),
        );
        if (text != null && text.isNotEmpty) {
          final quote = '${m.peer}|${m.text}';
          await widget.client.sendMessageFull(
            peerId: peer,
            text: text,
            quote: quote,
            messageId: null,
            expiresInSeconds: null,
          );
        }
      case 'edit':
        _edit(m);
      case 'delete':
        _delete(m);
    }
  }

  Future<String?> _pickEmoji() async {
    const emojis = ['👍', '❤️', '😂', '😮', '😢', '🙏'];
    return showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: Wp.panel,
        content: Wrap(
          spacing: 8,
          children: [
            for (final e in emojis)
              InkWell(
                onTap: () => Navigator.pop(ctx, e),
                borderRadius: BorderRadius.circular(10),
                child: Padding(
                  padding: const EdgeInsets.all(10),
                  child: Text(e, style: const TextStyle(fontSize: 26)),
                ),
              ),
          ],
        ),
      ),
    );
  }

  Widget _buildChatHeader(String peer) {
    final isGroup = peer.startsWith('group:');
    final isContact = _contacts.contains(peer);
    final presence = _presence[peer];
    final displayName = isGroup
        ? (_groups.where((g) => 'group:${g.id}' == peer).map((g) => g.name).firstOrNull ??
            'Group')
        : _short(peer, 32);
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
      decoration: const BoxDecoration(
        color: Wp.panel,
        border: Border(bottom: BorderSide(color: Wp.line)),
      ),
      child: Row(
        children: [
          WpAvatar(name: displayName, size: 36, group: isGroup),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  displayName,
                  style: const TextStyle(
                    color: Wp.text,
                    fontSize: 14,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                Text(
                  isGroup
                      ? 'Group'
                      : (presence ?? (isContact ? 'Online' : _short(peer, 20))),
                  style: TextStyle(
                    color: presence == 'Online' || (isContact && presence == null)
                        ? Wp.online
                        : Wp.textFaint,
                    fontSize: 12,
                    fontWeight:
                        presence == 'Online' ? FontWeight.w600 : FontWeight.w400,
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  String _short(String s, int max) =>
      s.length > max ? '${s.substring(0, max)}…' : s;
}

// ---------------------------------------------------------------------------
// Small widgets
// ---------------------------------------------------------------------------

class _IconBtn extends StatelessWidget {
  final IconData icon;
  final String tooltip;
  final VoidCallback onTap;
  final Color hoverColor;
  const _IconBtn({
    required this.icon,
    required this.tooltip,
    required this.onTap,
    this.hoverColor = Wp.text,
  });

  @override
  Widget build(BuildContext context) {
    return IconButton(
      onPressed: onTap,
      tooltip: tooltip,
      iconSize: 16,
      icon: Icon(icon, color: Wp.textDim),
      padding: const EdgeInsets.all(6),
      constraints: const BoxConstraints(),
      style: IconButton.styleFrom(hoverColor: Wp.panel2),
    );
  }
}

class _TabBtn extends StatelessWidget {
  final String label;
  final bool active;
  final VoidCallback onTap;
  const _TabBtn({
    required this.label,
    required this.active,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(10),
      child: Container(
        padding: const EdgeInsets.symmetric(vertical: 7),
        decoration: BoxDecoration(
          color: active ? Wp.accent : Colors.transparent,
          borderRadius: BorderRadius.circular(10),
        ),
        child: Text(
          label,
          textAlign: TextAlign.center,
          style: TextStyle(
            color: active ? Wp.accentFg : Wp.textDim,
            fontSize: 14,
            fontWeight: FontWeight.w600,
          ),
        ),
      ),
    );
  }
}

class _EmptyHint extends StatelessWidget {
  final IconData icon;
  final String title;
  final String subtitle;
  const _EmptyHint({
    required this.icon,
    required this.title,
    required this.subtitle,
  });

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            padding: const EdgeInsets.all(12),
            decoration: BoxDecoration(
              color: Wp.panel2,
              shape: BoxShape.circle,
            ),
            child: Icon(icon, size: 20, color: Wp.textFaint),
          ),
          const SizedBox(height: 10),
          Text(title,
              style: TextStyle(color: Wp.textDim, fontSize: 13)),
          const SizedBox(height: 2),
          Text(subtitle,
              style: TextStyle(color: Wp.textFaint, fontSize: 12)),
        ],
      ),
    );
  }
}

class _ConversationRow extends StatelessWidget {
  final String peerId;
  final String name;
  final String? subtitle;
  final String? lastText;
  final DateTime? lastTime;
  final bool active;
  final bool online;
  final VoidCallback onTap;
  const _ConversationRow({
    required this.peerId,
    required this.name,
    this.subtitle,
    this.lastText,
    this.lastTime,
    required this.active,
    required this.online,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onTap,
      child: Container(
        color: active ? Wp.panel3 : Colors.transparent,
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        child: Row(
          children: [
            WpAvatar(name: name, size: 40, group: peerId.startsWith('group:')),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      color: Wp.text,
                      fontSize: 14,
                      fontWeight: FontWeight.w500,
                    ),
                  ),
                  if (subtitle != null)
                    Text(
                      subtitle!,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(color: Wp.textFaint, fontSize: 11),
                    )
                  else if (lastText != null)
                    Text(
                      lastText!,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(color: Wp.textFaint, fontSize: 12),
                    ),
                ],
              ),
            ),
            if (online)
              Container(
                width: 8,
                height: 8,
                decoration: const BoxDecoration(
                  shape: BoxShape.circle,
                  color: Wp.online,
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _Bubble extends StatelessWidget {
  final ChatLine msg;
  final VoidCallback onLongPress;
  const _Bubble({required this.msg, required this.onLongPress});

  @override
  Widget build(BuildContext context) {
    final time = '${msg.at.hour.toString().padLeft(2, '0')}:'
        '${msg.at.minute.toString().padLeft(2, '0')}';
    return Align(
      alignment: msg.outgoing ? Alignment.centerRight : Alignment.centerLeft,
      child: GestureDetector(
        onLongPress: onLongPress,
        child: Container(
          margin: const EdgeInsets.symmetric(vertical: 3),
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
          constraints: BoxConstraints(
            maxWidth: MediaQuery.of(context).size.width * 0.68,
          ),
          decoration: BoxDecoration(
            gradient: msg.outgoing
                ? const LinearGradient(
                    begin: Alignment.topLeft,
                    end: Alignment.bottomRight,
                    colors: [Wp.bubbleOut2, Wp.bubbleOut],
                  )
                : null,
            color: msg.outgoing ? null : Wp.bubbleIn,
            borderRadius: BorderRadius.only(
              topLeft: const Radius.circular(16),
              topRight: const Radius.circular(16),
              bottomLeft: Radius.circular(msg.outgoing ? 16 : 6),
              bottomRight: Radius.circular(msg.outgoing ? 6 : 16),
            ),
            boxShadow: const [
              BoxShadow(color: Color(0x33000000), blurRadius: 4),
            ],
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              if (msg.quoteText != null)
                Container(
                  margin: const EdgeInsets.only(bottom: 6),
                  padding: const EdgeInsets.symmetric(
                      horizontal: 8, vertical: 5),
                  decoration: BoxDecoration(
                    color: Colors.black.withValues(alpha: 0.15),
                    borderRadius: BorderRadius.circular(6),
                    border: const Border(
                      left: BorderSide(color: Color(0x99B8A6), width: 2),
                    ),
                  ),
                  child: Text(
                    msg.quoteText!,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: Wp.text.withValues(alpha: 0.6),
                      fontSize: 11,
                    ),
                  ),
                ),
              Text(
                msg.text,
                style:
                    const TextStyle(color: Wp.text, fontSize: 14, height: 1.35),
              ),
              if (msg.reaction != null)
                Text(
                  msg.reaction!,
                  style: const TextStyle(fontSize: 16),
                ),
              const SizedBox(height: 2),
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  if (msg.edited != null)
                    Text(
                      '${msg.edited} · ',
                      style: TextStyle(
                        color: Wp.text.withValues(alpha: 0.4),
                        fontSize: 10,
                      ),
                    ),
                  Text(
                    time,
                    style: TextStyle(
                      color: Wp.text.withValues(alpha: 0.5),
                      fontSize: 10,
                    ),
                  ),
                  if (msg.outgoing) ...[
                    const SizedBox(width: 4),
                    Icon(
                      Icons.check,
                      size: 12,
                      color: Wp.text.withValues(alpha: 0.6),
                    ),
                  ],
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _Composer extends StatefulWidget {
  final ValueChanged<String> onSend;
  final ValueChanged<String> onSendQuote;
  const _Composer({required this.onSend, required this.onSendQuote});

  @override
  State<_Composer> createState() => _ComposerState();
}

class _ComposerState extends State<_Composer> {
  final _controller = TextEditingController();
  bool _replying = false;
  String _replyTarget = '';

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _send() {
    final text = _controller.text;
    if (text.trim().isEmpty) return;
    if (_replying) {
      widget.onSendQuote(text);
    } else {
      widget.onSend(text);
    }
    _controller.clear();
    setState(() {
      _replying = false;
      _replyTarget = '';
    });
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
      decoration: const BoxDecoration(
        color: Wp.panel,
        border: Border(top: BorderSide(color: Wp.line)),
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (_replying)
            Container(
              width: double.infinity,
              margin: const EdgeInsets.only(bottom: 8),
              padding:
                  const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
              decoration: BoxDecoration(
                color: Wp.panel2,
                borderRadius: BorderRadius.circular(8),
                border: const Border(
                  left: BorderSide(color: Wp.accent, width: 2),
                ),
              ),
              child: Row(
                children: [
                  Expanded(
                    child: Text(
                      'Replying to ${_shortPeer(_replyTarget)}',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: Wp.textFaint,
                        fontSize: 11,
                      ),
                    ),
                  ),
                  InkWell(
                    onTap: () => setState(() {
                      _replying = false;
                      _replyTarget = '';
                    }),
                    child: const Icon(Icons.close,
                        size: 14, color: Wp.textFaint),
                  ),
                ],
              ),
            ),
          Row(
            crossAxisAlignment: CrossAxisAlignment.end,
            children: [
              Expanded(
                child: TextField(
                  controller: _controller,
                  minLines: 1,
                  maxLines: 5,
                  style: const TextStyle(color: Wp.text, fontSize: 14),
                  decoration: InputDecoration(
                    hintText: 'Message',
                    hintStyle: const TextStyle(color: Wp.textFaint),
                    isDense: true,
                    filled: true,
                    fillColor: Wp.panel2,
                    contentPadding: const EdgeInsets.symmetric(
                        horizontal: 16, vertical: 12),
                    border: OutlineInputBorder(
                      borderRadius: BorderRadius.circular(16),
                      borderSide: BorderSide.none,
                    ),
                    focusedBorder: OutlineInputBorder(
                      borderRadius: BorderRadius.circular(16),
                      borderSide: const BorderSide(color: Wp.accent, width: 1),
                    ),
                  ),
                  onSubmitted: (_) => _send(),
                ),
              ),
              const SizedBox(width: 12),
              InkWell(
                onTap: _send,
                borderRadius: BorderRadius.circular(22),
                child: Container(
                  width: 44,
                  height: 44,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    gradient:
                        const LinearGradient(colors: [Wp.accent, Wp.accentStrong]),
                    boxShadow: [
                      BoxShadow(
                        color: Wp.accent.withValues(alpha: 0.25),
                        blurRadius: 8,
                      ),
                    ],
                  ),
                  child:
                      const Icon(Icons.send, size: 18, color: Wp.accentFg),
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }

  String _shortPeer(String s) => s.length > 14 ? '${s.substring(0, 14)}…' : s;
}
