import 'dart:async';

import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../rust/api/whisper.dart' as core;
import '../theme.dart';
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

/// The main app shell: a desktop-style sidebar (conversation list, contacts,
/// new-chat, settings) plus the chat pane. Mirrors the desktop MainView.
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

class _MainScreenState extends State<MainScreen> {
  final _messages = <String, List<ChatLine>>{};
  String? _activePeer;
  bool _connected = false;
  String _status = 'Not connected';
  List<String> _contacts = [];
  List<String> _pendingRequests = [];
  bool _showContacts = false;
  Timer? _poller;

  @override
  void initState() {
    super.initState();
    // Auto-connect using the hardcoded relay URL.
    _connect();
    // Poll the event queue (messages, contacts, presence, errors).
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
    super.dispose();
  }

  Future<void> _connect() async {
    setState(() => _status = 'Connecting…');
    try {
      // Load the stored identity JSON for the handshake.
      final json = await _loadIdentityJson();
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

  Future<String> _loadIdentityJson() async {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getString('identity_json') ?? '';
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

  Future<void> _newChat() async {
    final controller = TextEditingController();
    final peer = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: Wp.panel,
        title: const Text('New chat',
            style: TextStyle(color: Wp.text, fontSize: 18)),
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
            child: const Text('Cancel',
                style: TextStyle(color: Wp.textDim)),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(ctx, controller.text.trim()),
            style: FilledButton.styleFrom(backgroundColor: Wp.accent),
            child: const Text('Open', style: TextStyle(color: Wp.accentFg)),
          ),
        ],
      ),
    );
    if (peer != null && peer.isNotEmpty) {
      _openChat(peer);
    }
  }

  Future<void> _addFriend() async {
    final controller = TextEditingController();
    final peer = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: Wp.panel,
        title: const Text('Add contact',
            style: TextStyle(color: Wp.text, fontSize: 18)),
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
            child: const Text('Cancel',
                style: TextStyle(color: Wp.textDim)),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(ctx, controller.text.trim()),
            style: FilledButton.styleFrom(backgroundColor: Wp.accent),
            child: const Text('Send request',
                style: TextStyle(color: Wp.accentFg)),
          ),
        ],
      ),
    );
    if (peer != null && peer.isNotEmpty) {
      await widget.client.sendFriendRequest(target: peer);
    }
  }

  Future<void> _accept(String peer) async {
    await widget.client.acceptFriendRequest(peer: peer);
    await widget.client.refreshContacts();
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
        builder: (_) => SettingsScreen(client: widget.client, peerId: widget.peerId),
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
  // Sidebar
  // ---------------------------------------------------------------------

  Widget _buildSidebar() {
    // Chat-list entries: every contact plus anyone we have messages with.
    final ids = <String>{
      ..._contacts,
      ..._messages.keys,
    }.toList();
    final entries = ids.map((id) {
      final msgs = _messages[id] ?? [];
      return _SidebarEntry(
        peerId: id,
        lastText: msgs.isEmpty ? null : msgs.last.text,
        active: id == _activePeer,
        unread: false,
        onTap: () => _openChat(id),
      );
    }).toList();

    return Container(
      width: 280,
      color: Wp.panel,
      child: Column(
        children: [
          // Header: logo + brand.
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 14, 16, 10),
            child: Row(
              children: [
                ClipRRect(
                  borderRadius: BorderRadius.circular(7),
                  child: Image.asset('assets/whisper-logo.png',
                      width: 26, height: 26),
                ),
                const SizedBox(width: 10),
                const Expanded(
                  child: Text(
                    'Whisper',
                    style: TextStyle(
                      color: Wp.text,
                      fontSize: 18,
                      fontWeight: FontWeight.w700,
                      letterSpacing: -0.3,
                    ),
                  ),
                ),
                Container(
                  width: 8,
                  height: 8,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    color: _connected ? Wp.online : Wp.textFaint,
                  ),
                ),
              ],
            ),
          ),
          // New chat + contacts toggle.
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
            child: Row(
              children: [
                Expanded(
                  child: FilledButton.icon(
                    onPressed: _newChat,
                    style: FilledButton.styleFrom(
                      backgroundColor: Wp.accent,
                      foregroundColor: Wp.accentFg,
                      padding: const EdgeInsets.symmetric(vertical: 10),
                      textStyle: const TextStyle(
                          fontSize: 13, fontWeight: FontWeight.w700),
                    ),
                    icon: const Icon(Icons.edit, size: 16),
                    label: const Text('New chat'),
                  ),
                ),
                const SizedBox(width: 8),
                IconButton(
                  onPressed: _addFriend,
                  tooltip: 'Add contact',
                  icon: const Icon(Icons.person_add_alt, size: 20),
                  color: Wp.textDim,
                ),
                const SizedBox(width: 2),
                IconButton(
                  onPressed: () => setState(() => _showContacts = !_showContacts),
                  tooltip: 'Contacts',
                  icon: Icon(
                    _showContacts ? Icons.chat_bubble : Icons.people,
                    size: 20,
                    color: Wp.textDim,
                  ),
                ),
              ],
            ),
          ),
          // Pending requests banner.
          if (_pendingRequests.isNotEmpty)
            Container(
              width: double.infinity,
              margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
              padding: const EdgeInsets.all(10),
              decoration: BoxDecoration(
                color: const Color(0x1437C03A),
                borderRadius: BorderRadius.circular(12),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text(
                    'FRIEND REQUESTS',
                    style: TextStyle(
                      fontSize: 9,
                      fontWeight: FontWeight.w700,
                      color: Wp.online,
                      letterSpacing: 1.0,
                    ),
                  ),
                  for (final p in _pendingRequests)
                    Row(
                      children: [
                        Expanded(
                          child: Text(
                            p.length > 20 ? '${p.substring(0, 20)}…' : p,
                            style: const TextStyle(
                                fontSize: 11, color: Wp.textDim),
                          ),
                        ),
                        TextButton(
                          onPressed: () => _accept(p),
                          style: TextButton.styleFrom(
                              foregroundColor: Wp.accent),
                          child: const Text('Accept',
                              style: TextStyle(fontSize: 12)),
                        ),
                      ],
                    ),
                ],
              ),
            ),
          // List.
          Expanded(
            child: entries.isEmpty
                ? Center(
                    child: Text(
                      'No conversations yet.\nAdd a contact to start.',
                      textAlign: TextAlign.center,
                      style: TextStyle(color: Wp.textFaint, fontSize: 12),
                    ),
                  )
                : ListView.builder(
                    itemCount: entries.length,
                    itemBuilder: (context, i) => entries[i],
                  ),
          ),
          // Footer: status + settings.
          Container(
            padding: const EdgeInsets.all(12),
            decoration: const BoxDecoration(
              border: Border(top: BorderSide(color: Wp.line)),
            ),
            child: Row(
              children: [
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
                IconButton(
                  onPressed: _openSettings,
                  icon: const Icon(Icons.settings, size: 20),
                  color: Wp.textDim,
                ),
              ],
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
            Image.asset('assets/whisper-logo.png', width: 72, height: 72),
            const SizedBox(height: 18),
            Text(
              'Select a conversation',
              style: TextStyle(color: Wp.textFaint, fontSize: 14),
            ),
          ],
        ),
      );
    }
    final msgs = _messages[peer] ?? [];
    return Column(
      children: [
        // Chat header.
        Container(
          width: double.infinity,
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
          decoration: const BoxDecoration(
            color: Wp.panel,
            border: Border(bottom: BorderSide(color: Wp.line)),
          ),
          child: Row(
            children: [
              Container(
                width: 34,
                height: 34,
                decoration: BoxDecoration(
                  gradient: const LinearGradient(
                      colors: [Wp.accent, Wp.accentStrong]),
                  borderRadius: BorderRadius.circular(10),
                ),
                child: Center(
                  child: Text(
                    peer.isEmpty ? '?' : peer[0].toUpperCase(),
                    style: TextStyle(
                      color: Wp.accentFg,
                      fontWeight: FontWeight.w700,
                      fontSize: 15,
                    ),
                  ),
                ),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      peer.length > 30
                          ? '${peer.substring(0, 30)}…'
                          : peer,
                      style: const TextStyle(
                        color: Wp.text,
                        fontSize: 14,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    Text(
                      _contacts.contains(peer) ? 'Contact' : 'Peer',
                      style: TextStyle(color: Wp.textFaint, fontSize: 11),
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
        // Messages.
        Expanded(
          child: msgs.isEmpty
              ? Center(
                  child: Text(
                    'No messages yet.\nSay hello!',
                    textAlign: TextAlign.center,
                    style: TextStyle(color: Wp.textFaint, fontSize: 13),
                  ),
                )
              : ListView.builder(
                  padding: const EdgeInsets.all(14),
                  itemCount: msgs.length,
                  itemBuilder: (context, i) => _buildBubble(msgs[i]),
                ),
        ),
        // Composer.
        _Composer(onSend: _send),
      ],
    );
  }

  Widget _buildBubble(ChatLine m) {
    return Align(
      alignment: m.outgoing ? Alignment.centerRight : Alignment.centerLeft,
      child: Container(
        margin: const EdgeInsets.symmetric(vertical: 3),
        padding: const EdgeInsets.symmetric(horizontal: 13, vertical: 9),
        constraints: BoxConstraints(
          maxWidth: MediaQuery.of(context).size.width * 0.55,
        ),
        decoration: BoxDecoration(
          gradient: m.outgoing
              ? const LinearGradient(colors: [Wp.bubbleOut, Wp.bubbleOut2])
              : null,
          color: m.outgoing ? null : Wp.bubbleIn,
          borderRadius: BorderRadius.only(
            topLeft: const Radius.circular(14),
            topRight: const Radius.circular(14),
            bottomLeft: Radius.circular(m.outgoing ? 14 : 4),
            bottomRight: Radius.circular(m.outgoing ? 4 : 14),
          ),
        ),
        child: Text(
          m.text,
          style: const TextStyle(color: Wp.text, fontSize: 14, height: 1.35),
        ),
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// Sidebar entry
// ---------------------------------------------------------------------------

class _SidebarEntry extends StatelessWidget {
  final String peerId;
  final String? lastText;
  final bool active;
  final bool unread;
  final VoidCallback onTap;
  const _SidebarEntry({
    required this.peerId,
    required this.lastText,
    required this.active,
    required this.unread,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onTap,
      child: Container(
        color: active ? Wp.panel3 : Colors.transparent,
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
        child: Row(
          children: [
            Container(
              width: 38,
              height: 38,
              decoration: BoxDecoration(
                color: Wp.panel2,
                borderRadius: BorderRadius.circular(11),
              ),
              child: Center(
                child: Text(
                  peerId.isEmpty ? '?' : peerId[0].toUpperCase(),
                  style: TextStyle(
                    color: Wp.accent,
                    fontWeight: FontWeight.w700,
                    fontSize: 15,
                  ),
                ),
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    peerId.length > 26
                        ? '${peerId.substring(0, 26)}…'
                        : peerId,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: Wp.text,
                      fontSize: 13,
                      fontWeight: unread ? FontWeight.w700 : FontWeight.w500,
                    ),
                  ),
                  if (lastText != null)
                    Text(
                      lastText!,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(color: Wp.textFaint, fontSize: 11),
                    ),
                ],
              ),
            ),
            if (unread)
              Container(
                width: 8,
                height: 8,
                decoration: const BoxDecoration(
                  shape: BoxShape.circle,
                  color: Wp.accent,
                ),
              ),
          ],
        ),
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// Composer
// ---------------------------------------------------------------------------

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
      padding: const EdgeInsets.fromLTRB(12, 8, 12, 10),
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
              style: const TextStyle(fontSize: 14, color: Wp.text),
              decoration: const InputDecoration(
                hintText: 'Message…',
                hintStyle: TextStyle(color: Wp.textFaint),
                isDense: true,
              ),
              onSubmitted: (_) => _send(),
            ),
          ),
          const SizedBox(width: 8),
          IconButton.filled(
            onPressed: _send,
            style: IconButton.styleFrom(
              backgroundColor: Wp.accent,
              foregroundColor: Wp.accentFg,
            ),
            icon: const Icon(Icons.send, size: 18),
          ),
        ],
      ),
    );
  }
}
