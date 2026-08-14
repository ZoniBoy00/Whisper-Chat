import 'dart:async';

import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'src/rust/api/whisper.dart' as core;
import 'src/rust/frb_generated.dart';
import 'src/theme.dart';

void main() {
  runApp(const WhisperApp());
}

class WhisperApp extends StatelessWidget {
  const WhisperApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Whisper',
      debugShowCheckedModeBanner: false,
      theme: whisperTheme(),
      home: const HomeScreen(),
    );
  }
}

/// One chat line in the UI.
class ChatLine {
  final String peer;
  final String text;
  final bool outgoing;
  ChatLine(this.peer, this.text, {required this.outgoing});
}

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  final _messages = <ChatLine>[];
  final _peerInput = TextEditingController();
  final _textInput = TextEditingController();
  final _scroll = ScrollController();
  Timer? _poller;

  core.WhisperClient? _client;
  String? _peerId;
  bool _connected = false;
  String _status = 'Starting…';
  List<String> _contacts = [];
  List<String> _pendingRequests = [];

  @override
  void initState() {
    super.initState();
    _init();
  }

  @override
  void dispose() {
    _poller?.cancel();
    _peerInput.dispose();
    _textInput.dispose();
    _scroll.dispose();
    super.dispose();
  }

  Future<void> _init() async {
    await RustLib.init();
    final prefs = await SharedPreferences.getInstance();
    final stored = prefs.getString('identity_json');
    if (stored != null) {
      final info = await core.identityFromJson(json: stored);
      setState(() => _peerId = info.peerId);
    } else {
      final info = await core.identityCreate();
      await prefs.setString('identity_json', info.json);
      setState(() => _peerId = info.peerId);
    }
    _client = await core.WhisperClient.newInstance();
    setState(() => _status = 'Ready — press Connect');
    _poller = Timer.periodic(const Duration(milliseconds: 500), (_) => _poll());
  }

  Future<void> _poll() async {
    final client = _client;
    if (client == null) return;
    final events = await client.takeEvents();
    for (final e in events) {
      _handleEvent(e);
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
          _messages.add(ChatLine(e.peerId, e.text ?? '', outgoing: false));
          _scrollToBottom();
        case 'message_sent':
          _messages.add(ChatLine(e.peerId, e.text ?? '', outgoing: true));
          _scrollToBottom();
        case 'error':
          _status = e.error ?? 'Error';
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

  void _scrollToBottom() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_scroll.hasClients) {
        _scroll.jumpTo(_scroll.position.maxScrollExtent);
      }
    });
  }

  Future<void> _connect() async {
    final client = _client;
    final json =
        (await SharedPreferences.getInstance()).getString('identity_json');
    if (client == null || json == null) return;
    setState(() => _status = 'Connecting…');
    try {
      await client.connect(relayUrl: null, identityJson: json);
      setState(() => _status = 'Connected');
    } catch (err) {
      setState(() => _status = '$err');
    }
  }

  Future<void> _addFriend() async {
    final peer = _peerInput.text.trim();
    if (peer.isEmpty) return;
    try {
      await _client?.sendFriendRequest(target: peer);
      setState(() => _status = 'Friend request sent');
      _peerInput.clear();
    } catch (err) {
      setState(() => _status = '$err');
    }
  }

  Future<void> _accept(String peer) async {
    await _client?.acceptFriendRequest(peer: peer);
    await _client?.refreshContacts();
    await _client?.refreshFriendRequests();
  }

  Future<void> _refresh() async {
    await _client?.refreshContacts();
    await _client?.refreshFriendRequests();
  }

  Future<void> _send() async {
    final text = _textInput.text.trim();
    final peer = _peerInput.text.trim();
    if (text.isEmpty || peer.isEmpty) return;
    try {
      await _client?.sendMessage(peerId: peer, text: text);
      _textInput.clear();
    } catch (err) {
      setState(() => _status = '$err');
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: Row(
          children: [
            ClipRRect(
              borderRadius: BorderRadius.circular(8),
              child: Image.asset(
                'assets/whisper-logo.png',
                width: 26,
                height: 26,
              ),
            ),
            const SizedBox(width: 10),
            const Text(
              'Whisper',
              style: TextStyle(
                fontSize: 20,
                fontWeight: FontWeight.w600,
                letterSpacing: -0.3,
              ),
            ),
          ],
        ),
        actions: [
          _statusDot(),
          IconButton(
            icon: const Icon(Icons.refresh, size: 20),
            tooltip: 'Refresh contacts',
            onPressed: _refresh,
          ),
          const SizedBox(width: 4),
        ],
      ),
      body: Column(
        children: [
          _buildIdentityCard(),
          if (_pendingRequests.isNotEmpty) _buildRequestsCard(),
          if (_contacts.isNotEmpty) _buildContactsCard(),
          Expanded(child: _buildMessageList()),
          _buildComposer(),
        ],
      ),
    );
  }

  Widget _statusDot() {
    return Padding(
      padding: const EdgeInsets.only(right: 6),
      child: Container(
        width: 9,
        height: 9,
        decoration: BoxDecoration(
          shape: BoxShape.circle,
          color: _connected ? Wp.online : Wp.textFaint,
          boxShadow: _connected
              ? [
                  BoxShadow(
                    color: Wp.online.withValues(alpha: 0.5),
                    blurRadius: 6,
                  ),
                ]
              : null,
        ),
      ),
    );
  }

  Widget _buildIdentityCard() {
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
      decoration: const BoxDecoration(
        color: Wp.panel,
        border: Border(bottom: BorderSide(color: Wp.line)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Expanded(
                child: Text(
                  _status,
                  style: TextStyle(
                    color: _connected ? Wp.online : Wp.textDim,
                    fontSize: 13,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ),
              FilledButton(
                onPressed: _connect,
                style: FilledButton.styleFrom(
                  backgroundColor: Wp.accent,
                  foregroundColor: Wp.accentFg,
                  padding: const EdgeInsets.symmetric(
                      horizontal: 16, vertical: 8),
                  textStyle: const TextStyle(
                      fontSize: 12, fontWeight: FontWeight.w600),
                ),
                child: Text(_connected ? 'Reconnect' : 'Connect'),
              ),
            ],
          ),
          const SizedBox(height: 10),
          Text(
            'My peer ID',
            style: TextStyle(
              color: Wp.textFaint,
              fontSize: 10,
              letterSpacing: 1.1,
              fontWeight: FontWeight.w600,
            ),
          ),
          const SizedBox(height: 2),
          SelectableText(
            _peerId ?? '—',
            style: const TextStyle(color: Wp.textDim, fontSize: 12),
          ),
          const SizedBox(height: 10),
          Row(
            children: [
              Expanded(
                child: TextField(
                  controller: _peerInput,
                  style: const TextStyle(fontSize: 12, color: Wp.text),
                  decoration: const InputDecoration(
                    hintText: 'Peer ID (24 hex)',
                    isDense: true,
                  ),
                ),
              ),
              const SizedBox(width: 8),
              OutlinedButton.icon(
                onPressed: _addFriend,
                style: OutlinedButton.styleFrom(
                  foregroundColor: Wp.textDim,
                  side: const BorderSide(color: Wp.line),
                  padding: const EdgeInsets.symmetric(
                      horizontal: 14, vertical: 10),
                  textStyle: const TextStyle(
                      fontSize: 12, fontWeight: FontWeight.w600),
                ),
                icon: const Icon(Icons.person_add_alt, size: 16),
                label: const Text('Add'),
              ),
            ],
          ),
        ],
      ),
    );
  }

  Widget _buildRequestsCard() {
    return Container(
      width: double.infinity,
      color: const Color(0x1437C03A),
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text(
            'Friend requests',
            style: TextStyle(
              fontSize: 11,
              fontWeight: FontWeight.w700,
              color: Wp.online,
              letterSpacing: 0.5,
            ),
          ),
          for (final p in _pendingRequests)
            Row(
              children: [
                Expanded(
                  child: Text(
                    p,
                    style: const TextStyle(fontSize: 11, color: Wp.textDim),
                  ),
                ),
                TextButton(
                  onPressed: () => _accept(p),
                  style: TextButton.styleFrom(
                    foregroundColor: Wp.accent,
                    textStyle: const TextStyle(
                        fontSize: 12, fontWeight: FontWeight.w600),
                  ),
                  child: const Text('Accept'),
                ),
              ],
            ),
        ],
      ),
    );
  }

  Widget _buildContactsCard() {
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      decoration: const BoxDecoration(
        color: Wp.panel,
        border: Border(bottom: BorderSide(color: Wp.line)),
      ),
      child: Wrap(
        spacing: 8,
        runSpacing: 6,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          const Text(
            'CONTACTS',
            style: TextStyle(
              fontSize: 10,
              fontWeight: FontWeight.w700,
              color: Wp.textFaint,
              letterSpacing: 1.1,
            ),
          ),
          for (final c in _contacts)
            ActionChip(
              label: Text(
                c.length > 18 ? '${c.substring(0, 18)}…' : c,
                style: const TextStyle(fontSize: 10, color: Wp.textDim),
              ),
              backgroundColor: Wp.panel3,
              side: const BorderSide(color: Wp.line),
              visualDensity: VisualDensity.compact,
              onPressed: () => _peerInput.text = c,
            ),
        ],
      ),
    );
  }

  Widget _buildMessageList() {
    if (_messages.isEmpty) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Image.asset('assets/whisper-logo.png', width: 64, height: 64),
            const SizedBox(height: 14),
            Text(
              'No messages yet',
              style: TextStyle(color: Wp.textFaint, fontSize: 13),
            ),
            const SizedBox(height: 4),
            Text(
              'Add a contact and whisper something',
              style: TextStyle(color: Wp.textFaint, fontSize: 11),
            ),
          ],
        ),
      );
    }
    return ListView.builder(
      controller: _scroll,
      padding: const EdgeInsets.all(14),
      itemCount: _messages.length,
      itemBuilder: (context, i) => _buildBubble(_messages[i]),
    );
  }

  Widget _buildBubble(ChatLine m) {
    return Align(
      alignment: m.outgoing ? Alignment.centerRight : Alignment.centerLeft,
      child: Container(
        margin: const EdgeInsets.symmetric(vertical: 3),
        padding: const EdgeInsets.symmetric(horizontal: 13, vertical: 9),
        constraints: BoxConstraints(
          maxWidth: MediaQuery.of(context).size.width * 0.75,
        ),
        decoration: BoxDecoration(
          gradient: m.outgoing
              ? const LinearGradient(
                  colors: [Wp.bubbleOut, Wp.bubbleOut2],
                )
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

  Widget _buildComposer() {
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
              controller: _textInput,
              maxLines: 1,
              style: const TextStyle(fontSize: 14, color: Wp.text),
              decoration: const InputDecoration(
                hintText: 'Message…',
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
