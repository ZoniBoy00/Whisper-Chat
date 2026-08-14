import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/whisper.dart' as core;
import '../theme.dart';

/// First-run onboarding: choose a display name (and optionally register a
/// username), then show the peer ID card. Mirrors desktop Onboarding.tsx.
class OnboardingScreen extends StatefulWidget {
  final core.WhisperClient client;
  final String identityJson;
  final String peerId;
  final VoidCallback onDone;
  const OnboardingScreen({
    super.key,
    required this.client,
    required this.identityJson,
    required this.peerId,
    required this.onDone,
  });

  @override
  State<OnboardingScreen> createState() => _OnboardingScreenState();
}

class _OnboardingScreenState extends State<OnboardingScreen> {
  final _nameCtrl = TextEditingController();
  final _userCtrl = TextEditingController();
  bool _busy = false;
  bool _copied = false;
  String? _error;

  @override
  void dispose() {
    _nameCtrl.dispose();
    _userCtrl.dispose();
    super.dispose();
  }

  Future<void> _continue() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final name = _nameCtrl.text.trim();
      final username = _userCtrl.text.trim().toLowerCase();
      // Set the display name first (best-effort; the relay may be offline).
      if (name.isNotEmpty) {
        await widget.client.setDisplayName(displayName: name);
      }
      // Register a username if the user provided one (signed binding).
      if (username.isNotEmpty) {
        final sig = await core.signUsername(
            json: widget.identityJson, username: username);
        await widget.client.registerProfile(
            username: username, signature: sig, displayName: name.isEmpty ? null : name);
      }
      if (mounted) widget.onDone();
    } catch (err) {
      setState(() => _error = '$err');
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _copy() async {
    await Clipboard.setData(ClipboardData(text: widget.peerId));
    setState(() => _copied = true);
    Future.delayed(const Duration(milliseconds: 1600), () {
      if (mounted) setState(() => _copied = false);
    });
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Wp.bg,
      body: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(28),
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 420),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Center(
                    child: ClipRRect(
                      borderRadius: BorderRadius.circular(20),
                      child: Image.asset(
                        'assets/whisper-logo.png',
                        width: 88,
                        height: 88,
                      ),
                    ),
                  ),
                  const SizedBox(height: 22),
                  const Text(
                    'Welcome to Whisper',
                    textAlign: TextAlign.center,
                    style: TextStyle(
                      color: Wp.text,
                      fontSize: 24,
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    'Set up your profile. You can change these later in '
                    'Settings.',
                    textAlign: TextAlign.center,
                    style: TextStyle(
                      color: Wp.textDim,
                      fontSize: 13,
                      height: 1.45,
                    ),
                  ),
                  const SizedBox(height: 24),
                  TextField(
                    controller: _nameCtrl,
                    style: const TextStyle(color: Wp.text),
                    decoration: const InputDecoration(
                      labelText: 'Display name',
                      hintText: 'What friends will see',
                    ),
                  ),
                  const SizedBox(height: 12),
                  TextField(
                    controller: _userCtrl,
                    style: const TextStyle(color: Wp.text),
                    decoration: const InputDecoration(
                      labelText: 'Username (optional)',
                      hintText: 'lowercase_letters_123',
                    ),
                  ),
                  if (_error != null) ...[
                    const SizedBox(height: 10),
                    Text(
                      _error!,
                      style: TextStyle(color: Wp.danger, fontSize: 12),
                    ),
                  ],
                  const SizedBox(height: 20),
                  FilledButton(
                    onPressed: _busy ? null : _continue,
                    style: FilledButton.styleFrom(
                      backgroundColor: Wp.accent,
                      foregroundColor: Wp.accentFg,
                      padding: const EdgeInsets.symmetric(vertical: 14),
                      textStyle: const TextStyle(
                        fontSize: 15,
                        fontWeight: FontWeight.w700,
                      ),
                    ),
                    child: Text(_busy ? 'Setting up…' : 'Continue'),
                  ),
                  const SizedBox(height: 22),
                  Container(
                    padding: const EdgeInsets.all(16),
                    decoration: BoxDecoration(
                      color: Wp.panel,
                      borderRadius: BorderRadius.circular(16),
                      border: Border.all(color: Wp.line),
                    ),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          'YOUR PEER ID',
                          style: TextStyle(
                            color: Wp.textFaint,
                            fontSize: 10,
                            letterSpacing: 1.2,
                            fontWeight: FontWeight.w700,
                          ),
                        ),
                        const SizedBox(height: 8),
                        Row(
                          children: [
                            Expanded(
                              child: SelectableText(
                                widget.peerId,
                                style: const TextStyle(
                                  color: Wp.accent,
                                  fontSize: 14,
                                  fontFamily: 'monospace',
                                  letterSpacing: 0.8,
                                ),
                              ),
                            ),
                            IconButton(
                              onPressed: _copy,
                              icon: Icon(
                                _copied ? Icons.check : Icons.copy,
                                size: 16,
                                color: _copied ? Wp.online : Wp.textDim,
                              ),
                            ),
                          ],
                        ),
                        Text(
                          'Share this ID so others can add you.',
                          style: TextStyle(
                            color: Wp.textFaint,
                            fontSize: 11,
                          ),
                        ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
