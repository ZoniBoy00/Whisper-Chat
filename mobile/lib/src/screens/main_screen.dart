import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../rust/api/whisper.dart' as core;
import '../theme.dart';
import '../widgets/avatar.dart';
import 'settings_screen.dart';

/// One chat line in a conversation.
class ChatLine {
  final String peer;
  final String text;
  final bool outgoing;
  final DateTime at;
  ChatLine(this.peer, this.text, {required this.outgoing})
      : at = DateTime.now();
}

/// The main app shell — a pixel-faithful port of the desktop MainView:
/// a 350px sidebar (profile header, search, Chats/Contacts tabs, conversation
/// list) beside the chat pane (header, bubbles, composer).
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

enum _View { chats, contacts }

class _MainScreenState extends State<MainScreen> {
  final _messages = <String, List<ChatLine>>{};
  String? _activePeer;
  bool _connected = false;
  String _status = 'Not connected';
  List<String> _contacts = [];
  List<String> _pendingRequests = [];
  _View _view = _View.chats;
  Timer? _poller;
  final _searchCtrl = TextEditingController();
  String _query = '';

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
        case 'contacts':
          _contacts =
              (e.text ?? '').split('\n').where((p) => p.isNotEmpty).toList();
        case 'friend_requests':
          _pendingRequests =
              (e.text ?? '').split('\n').where((p) => p.isNotEmpty).toList();
        default:
          break;
      }
    });
  }

  Future<void> _openChat(String peer) async {
    setState(() => _activePeer = peer);
    await widget.client.watchPresence(peerId: peer);
  }

  Future<void> _acceptRequest(String peer) async {
    await widget.client.acceptFriendRequest(peer: peer);
    await widget.client.refreshContacts();
    await widget.client.refreshFriendRequests();
  }

  Future<void> _declineRequest(String peer) async {
    // No decline command in the MVP core yet; accept is the primary path.
    await widget.client.refreshFriendRequests();
  }

  Future<void> _send(String text) async {
    final peer = _activePeer;
    if (peer == null || text.trim().isEmpty) return;
    await widget.client.sendMessage(peerId: peer, text: text.trim());
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
  // Sidebar (350px) — desktop Sidebar.tsx port
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
          Expanded(child: _buildList()),
          _buildConnectionFooter(),
        ],
      ),
    );
  }

  /// Slim connection-status footer at the bottom of the sidebar.
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
            Icon(_query.isNotEmpty ? Icons.search : Icons.search,
                size: 16, color: Wp.textFaint),
            const SizedBox(width: 8),
            Expanded(
              child: TextField(
                controller: _searchCtrl,
                onChanged: (v) => setState(() => _query = v),
                style: const TextStyle(color: Wp.text, fontSize: 14),
                decoration: const InputDecoration(
                  isCollapsed: true,
                  hintText: 'Search',
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
        ],
      ),
    );
  }

  Widget _buildSectionHeader() {
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
      child: Row(
        children: [
          Text(
            _view == _View.chats ? 'Conversations' : 'Contacts',
            style: const TextStyle(
              color: Wp.textFaint,
              fontSize: 12,
              fontWeight: FontWeight.w700,
              letterSpacing: 1.2,
            ),
          ),
          const Spacer(),
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
              'CONTACTS',
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
              padding: const EdgeInsets.symmetric(
                  horizontal: 12, vertical: 8),
              decoration: BoxDecoration(
                color: Colors.transparent,
                borderRadius: BorderRadius.circular(12),
              ),
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

  Widget _buildList() {
    // Filter by the search query (name/peer-id).
    final q = _query.trim().toLowerCase();
    List<String> ids;
    if (_view == _View.contacts) {
      ids = _contacts.toList();
    } else {
      ids = {
        ..._contacts,
        ..._messages.keys,
      }.toList();
    }
    ids = ids
        .where((id) => q.isEmpty || id.toLowerCase().contains(q))
        .toList();

    if (ids.isEmpty) {
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
              child: Icon(Icons.search_off,
                  size: 20, color: Wp.textFaint),
            ),
            const SizedBox(height: 10),
            Text(
              _view == _View.chats ? 'No conversations yet' : 'No contacts yet',
              style: TextStyle(color: Wp.textDim, fontSize: 13),
            ),
            Text(
              _view == _View.chats
                  ? 'Add a contact to start chatting'
                  : 'Send a friend request to add someone',
              style: TextStyle(color: Wp.textFaint, fontSize: 12),
            ),
          ],
        ),
      );
    }

    return ListView.builder(
      padding: const EdgeInsets.symmetric(vertical: 2),
      itemCount: ids.length,
      itemBuilder: (context, i) {
        final id = ids[i];
        final msgs = _messages[id] ?? [];
        final last = msgs.isEmpty ? null : msgs.last;
        return _ConversationRow(
          peerId: id,
          lastText: last?.text,
          lastTime: last?.at,
          active: id == _activePeer,
          online: _contacts.contains(id),
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

  // ---------------------------------------------------------------------
  // Chat pane — desktop ChatView.tsx port
  // ---------------------------------------------------------------------

  Widget _buildChatPane() {
    final peer = _activePeer;
    if (peer == null) {
      // Empty state: centered icon tile on wp-bg.
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
                  padding: const EdgeInsets.symmetric(
                      horizontal: 16, vertical: 12),
                  itemCount: msgs.length,
                  itemBuilder: (context, i) => _Bubble(msg: msgs[i]),
                ),
        ),
        _Composer(onSend: _send),
      ],
    );
  }

  Widget _buildChatHeader(String peer) {
    final isContact = _contacts.contains(peer);
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
      decoration: const BoxDecoration(
        color: Wp.panel,
        border: Border(bottom: BorderSide(color: Wp.line)),
      ),
      child: Row(
        children: [
          WpAvatar(name: peer, size: 36),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  _short(peer, 32),
                  style: const TextStyle(
                    color: Wp.text,
                    fontSize: 14,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                Row(
                  children: [
                    if (isContact) ...[
                      Container(
                        width: 8,
                        height: 8,
                        decoration: const BoxDecoration(
                          shape: BoxShape.circle,
                          color: Wp.online,
                        ),
                      ),
                      const SizedBox(width: 6),
                      const Text(
                        'Online',
                        style: TextStyle(
                          color: Wp.online,
                          fontSize: 13,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ] else
                      Text(
                        _short(peer, 20),
                        style:
                            TextStyle(color: Wp.textFaint, fontSize: 12),
                      ),
                  ],
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
      style: IconButton.styleFrom(
        hoverColor: Wp.panel2,
      ),
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

class _ConversationRow extends StatelessWidget {
  final String peerId;
  final String? lastText;
  final DateTime? lastTime;
  final bool active;
  final bool online;
  final VoidCallback onTap;
  const _ConversationRow({
    required this.peerId,
    required this.lastText,
    required this.lastTime,
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
            WpAvatar(name: peerId, size: 40),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    peerId.length > 24
                        ? '${peerId.substring(0, 24)}…'
                        : peerId,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      color: Wp.text,
                      fontSize: 14,
                      fontWeight: FontWeight.w500,
                    ),
                  ),
                  if (lastText != null)
                    Text(
                      lastText!,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style:
                          TextStyle(color: Wp.textFaint, fontSize: 12),
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

/// Message bubble — desktop MessageBubble.tsx port:
/// `max-w-[68%] rounded-2xl px-4 py-2.5`, own = teal gradient with rounded-br,
/// incoming = dark with rounded-bl.
class _Bubble extends StatelessWidget {
  final ChatLine msg;
  const _Bubble({required this.msg});

  @override
  Widget build(BuildContext context) {
    final time = '${msg.at.hour.toString().padLeft(2, '0')}:'
        '${msg.at.minute.toString().padLeft(2, '0')}';
    return Align(
      alignment: msg.outgoing ? Alignment.centerRight : Alignment.centerLeft,
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
          crossAxisAlignment: CrossAxisAlignment.end,
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              msg.text,
              style: const TextStyle(color: Wp.text, fontSize: 14, height: 1.35),
            ),
            const SizedBox(height: 2),
            Text(
              time,
              style: TextStyle(
                color: Wp.text.withValues(alpha: 0.5),
                fontSize: 10,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Composer — desktop Composer.tsx port: rounded-2xl field + circular send.
class _Composer extends StatefulWidget {
  final ValueChanged<String> onSend;
  const _Composer({required this.onSend});

  @override
  State<_Composer> createState() => _ComposerState();
}

class _ComposerState extends State<_Composer> {
  final _controller = TextEditingController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _send() {
    final text = _controller.text;
    if (text.trim().isEmpty) return;
    widget.onSend(text);
    _controller.clear();
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
      decoration: const BoxDecoration(
        color: Wp.panel,
        border: Border(top: BorderSide(color: Wp.line)),
      ),
      child: Row(
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
                contentPadding:
                    const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
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
                gradient: const LinearGradient(
                    colors: [Wp.accent, Wp.accentStrong]),
                boxShadow: [
                  BoxShadow(
                    color: Wp.accent.withValues(alpha: 0.25),
                    blurRadius: 8,
                  ),
                ],
              ),
              child: const Icon(Icons.send, size: 18, color: Wp.accentFg),
            ),
          ),
        ],
      ),
    );
  }
}
