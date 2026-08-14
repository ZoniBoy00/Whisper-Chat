import 'dart:async';

import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'src/rust/api/whisper.dart' as core;
import 'src/rust/frb_generated.dart';

void main() {
  runApp(const WhisperApp());
}

class WhisperApp extends StatelessWidget {
  const WhisperApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Whisper',
      theme: ThemeData.dark(useMaterial3: true),
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
  final _log = <String>[];
  final _peerInput = TextEditingController();
  final _textInput = TextEditingController();
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
    super.dispose();
  }

  Future<void> _init() async {
    await RustLib.init();
    // Load or create the identity.
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
    // Poll the event queue.
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
        case 'message_sent':
          _messages.add(ChatLine(e.peerId, e.text ?? '', outgoing: true));
        case 'error':
          _status = 'Error: ${e.error ?? ''}';
          _log.add('ERR: ${e.error}');
        case 'contacts':
          _contacts = (e.text ?? '').split('\n').where((p) => p.isNotEmpty).toList();
        case 'friend_requests':
          _pendingRequests =
              (e.text ?? '').split('\n').where((p) => p.isNotEmpty).toList();
        case 'session_established':
          _log.add('Session established with ${e.peerId}');
        default:
          _log.add('${e.kind} ${e.peerId}');
      }
    });
  }

  Future<void> _connect() async {
    final client = _client;
    final json = (await SharedPreferences.getInstance()).getString('identity_json');
    if (client == null || json == null) return;
    setState(() => _status = 'Connecting…');
    try {
      await client.connect(relayUrl: null, identityJson: json);
      setState(() => _status = 'Connected (hello + prekeys sent)');
    } catch (err) {
      setState(() => _status = 'Connect failed: $err');
    }
  }

  Future<void> _addFriend() async {
    final peer = _peerInput.text.trim();
    if (peer.isEmpty) return;
    try {
      await _client?.sendFriendRequest(target: peer);
      setState(() => _status = 'Friend request sent to $peer');
      _peerInput.clear();
    } catch (err) {
      setState(() => _status = 'Friend request failed: $err');
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
      setState(() => _status = 'Send failed: $err');
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Whisper'),
        actions: [
          IconButton(
            icon: const Icon(Icons.refresh),
            tooltip: 'Refresh contacts',
            onPressed: _refresh,
          ),
        ],
      ),
      body: Column(
        children: [
          // Status + identity.
          Container(
            padding: const EdgeInsets.all(12),
            color: Theme.of(context).colorScheme.surfaceContainerHighest,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text('Status: $_status',
                    style: const TextStyle(fontWeight: FontWeight.bold)),
                const SizedBox(height: 4),
                Text('My peer ID: ${_peerId ?? "—"}',
                    style: const TextStyle(fontSize: 12)),
                const SizedBox(height: 8),
                Wrap(
                  spacing: 8,
                  children: [
                    FilledButton.icon(
                      onPressed: _connect,
                      icon: const Icon(Icons.wifi),
                      label: Text(_connected ? 'Reconnect' : 'Connect'),
                    ),
                    OutlinedButton(
                      onPressed: _addFriend,
                      child: const Text('Send friend request'),
                    ),
                  ],
                ),
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: TextField(
                    controller: _peerInput,
                    decoration: const InputDecoration(
                      labelText: 'Peer ID (24 hex chars)',
                      border: OutlineInputBorder(),
                      isDense: true,
                    ),
                    style: const TextStyle(fontSize: 12),
                  ),
                ),
              ],
            ),
          ),
          // Friend requests.
          if (_pendingRequests.isNotEmpty)
            Container(
              width: double.infinity,
              padding: const EdgeInsets.all(8),
              color: Colors.amber.withValues(alpha: 0.08),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text('Pending friend requests:',
                      style: TextStyle(fontWeight: FontWeight.bold)),
                  for (final p in _pendingRequests)
                    Row(
                      children: [
                        Expanded(
                            child: Text(p,
                                style: const TextStyle(fontSize: 12))),
                        TextButton(
                          onPressed: () => _accept(p),
                          child: const Text('Accept'),
                        ),
                      ],
                    ),
                ],
              ),
            ),
          // Contacts.
          if (_contacts.isNotEmpty)
            Container(
              width: double.infinity,
              padding: const EdgeInsets.all(8),
              color: Colors.green.withValues(alpha: 0.06),
              child: Wrap(
                spacing: 8,
                children: [
                  const Text('Contacts:', style: TextStyle(fontWeight: FontWeight.bold)),
                  for (final c in _contacts)
                    ActionChip(
                      label: Text(c, style: const TextStyle(fontSize: 11)),
                      visualDensity: VisualDensity.compact,
                      onPressed: () => _peerInput.text = c,
                    ),
                ],
              ),
            ),
          // Messages.
          Expanded(
            child: ListView.builder(
              padding: const EdgeInsets.all(12),
              itemCount: _messages.length,
              itemBuilder: (context, i) {
                final m = _messages[i];
                return Align(
                  alignment:
                      m.outgoing ? Alignment.centerRight : Alignment.centerLeft,
                  child: Container(
                    margin: const EdgeInsets.symmetric(vertical: 3),
                    padding: const EdgeInsets.symmetric(
                        horizontal: 12, vertical: 8),
                    decoration: BoxDecoration(
                      color: m.outgoing
                          ? Theme.of(context).colorScheme.primary
                          : Theme.of(context).colorScheme.surfaceContainerHighest,
                      borderRadius: BorderRadius.circular(14),
                    ),
                    child: Text(m.text,
                        style: const TextStyle(fontSize: 14)),
                  ),
                );
              },
            ),
          ),
          // Composer.
          Padding(
            padding: const EdgeInsets.all(8),
            child: Row(
              children: [
                Expanded(
                  child: TextField(
                    controller: _textInput,
                    decoration: const InputDecoration(
                      hintText: 'Message…',
                      border: OutlineInputBorder(),
                      isDense: true,
                    ),
                    onSubmitted: (_) => _send(),
                  ),
                ),
                const SizedBox(width: 8),
                IconButton.filled(
                  onPressed: _send,
                  icon: const Icon(Icons.send),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
