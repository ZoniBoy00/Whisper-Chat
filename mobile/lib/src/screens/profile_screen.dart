import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../i18n.dart';
import '../rust/api/whisper.dart' as core;
import '../theme.dart';
import '../widgets/avatar.dart';

/// Profile view: own profile (name, avatar) or another user's, with the
/// E2EE safety number. Mirrors desktop ProfileDialog + SafetyNumberCard.
class ProfileScreen extends StatefulWidget {
  final core.WhisperClient client;
  final String identityJson;
  final String myPeerId;
  /// The peer whose profile this is (own peerId when viewing our own).
  final String peerId;
  final bool isSelf;
  final String? displayName;
  final String? curve25519Key;
  const ProfileScreen({
    super.key,
    required this.client,
    required this.identityJson,
    required this.myPeerId,
    required this.peerId,
    required this.isSelf,
    this.displayName,
    this.curve25519Key,
  });

  @override
  State<ProfileScreen> createState() => _ProfileScreenState();
}

class _ProfileScreenState extends State<ProfileScreen> {
  String? _safety;
  String? _shortSafety;
  bool _copiedPeer = false;
  bool _copiedSafety = false;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    if (widget.isSelf || widget.curve25519Key == null) return;
    try {
      final s =
          await core.safetyNumber(identityJson: widget.identityJson, theirCurve25519: widget.curve25519Key!);
      final ss = await core.shortSafetyNumber(
          identityJson: widget.identityJson, theirCurve25519: widget.curve25519Key!);
      if (mounted) {
        setState(() {
          _safety = s;
          _shortSafety = ss;
        });
      }
    } catch (_) {}
  }

  Future<void> _copyPeer() async {
    await Clipboard.setData(ClipboardData(text: widget.peerId));
    setState(() => _copiedPeer = true);
    Future.delayed(const Duration(milliseconds: 1600), () {
      if (mounted) setState(() => _copiedPeer = false);
    });
  }

  Future<void> _copySafety() async {
    final s = _safety;
    if (s == null) return;
    await Clipboard.setData(ClipboardData(text: s));
    setState(() => _copiedSafety = true);
    Future.delayed(const Duration(milliseconds: 1600), () {
      if (mounted) setState(() => _copiedSafety = false);
    });
  }

  @override
  Widget build(BuildContext context) {
    final t = LanguageScope.of(context).t;
    final name = widget.displayName ?? (widget.isSelf ? 'Your Whisper ID' : widget.peerId);
    return Scaffold(
      backgroundColor: Wp.bg,
      appBar: AppBar(title: Text(widget.isSelf ? t('your_whisper_id') : 'Profile')),
      body: ListView(
        padding: const EdgeInsets.all(20),
        children: [
          Center(
            child: WpAvatar(name: name, size: 88),
          ),
          const SizedBox(height: 14),
          Center(
            child: Text(
              name,
              style: const TextStyle(
                color: Wp.text,
                fontSize: 20,
                fontWeight: FontWeight.w700,
              ),
            ),
          ),
          const SizedBox(height: 4),
          Center(
            child: Text(
              widget.peerId,
              style: const TextStyle(
                color: Wp.textFaint,
                fontSize: 12,
                fontFamily: 'monospace',
              ),
            ),
          ),
          const SizedBox(height: 16),
          Center(
            child: OutlinedButton.icon(
              onPressed: _copyPeer,
              style: OutlinedButton.styleFrom(
                foregroundColor: Wp.textDim,
                side: const BorderSide(color: Wp.line),
              ),
              icon: Icon(
                _copiedPeer ? Icons.check : Icons.copy,
                size: 14,
                color: _copiedPeer ? Wp.online : Wp.textDim,
              ),
              label: Text(_copiedPeer ? t('copied') : t('copy_peer_id'),
                  style: const TextStyle(fontSize: 12)),
            ),
          ),
          const SizedBox(height: 28),
          if (!widget.isSelf && _safety != null) ...[
            _SectionLabel('E2EE SAFETY NUMBER'),
            Container(
              padding: const EdgeInsets.all(16),
              decoration: BoxDecoration(
                color: Wp.panel,
                borderRadius: BorderRadius.circular(14),
                border: Border.all(color: Wp.line),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      const Icon(Icons.shield_outlined,
                          size: 16, color: Wp.accent),
                      const SizedBox(width: 8),
                      Text(
                        _shortSafety ?? '',
                        style: const TextStyle(
                          color: Wp.accent,
                          fontSize: 14,
                          fontWeight: FontWeight.w700,
                        ),
                      ),
                      const Spacer(),
                      IconButton(
                        onPressed: _copySafety,
                        icon: Icon(
                          _copiedSafety ? Icons.check : Icons.copy,
                          size: 14,
                          color: _copiedSafety ? Wp.online : Wp.textDim,
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 8),
                  SelectableText(
                    _safety!,
                    style: const TextStyle(
                      color: Wp.textDim,
                      fontSize: 13,
                      fontFamily: 'monospace',
                      height: 1.5,
                    ),
                  ),
                  const SizedBox(height: 6),
                  Text(
                    'Verify this number with ${widget.displayName ?? widget.peerId} to confirm the session is end-to-end encrypted.',
                    style: TextStyle(
                      color: Wp.textFaint,
                      fontSize: 11,
                      height: 1.4,
                    ),
                  ),
                ],
              ),
            ),
          ] else if (!widget.isSelf)
            Center(
              child: Text(
                'Safety number unavailable',
                style: TextStyle(color: Wp.textFaint, fontSize: 12),
              ),
            ),
          if (widget.isSelf) ...[
            _SectionLabel('YOUR KEYS'),
            Container(
              padding: const EdgeInsets.all(16),
              decoration: BoxDecoration(
                color: Wp.panel,
                borderRadius: BorderRadius.circular(14),
                border: Border.all(color: Wp.line),
              ),
              child: Text(
                'Your identity is a cryptographic key pair stored on this '
                'device only. Others verify you via the safety number they '
                'see for this peer ID.',
                style: TextStyle(
                  color: Wp.textDim,
                  fontSize: 12,
                  height: 1.5,
                ),
              ),
            ),
          ],
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
