import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/whisper.dart' as core;
import '../theme.dart';

/// First-run onboarding: create the identity and show the peer ID card,
/// mirroring the desktop Onboarding.tsx flow.
class OnboardingScreen extends StatefulWidget {
  final core.IdentityInfo identity;
  final VoidCallback onDone;
  const OnboardingScreen({
    super.key,
    required this.identity,
    required this.onDone,
  });

  @override
  State<OnboardingScreen> createState() => _OnboardingScreenState();
}

class _OnboardingScreenState extends State<OnboardingScreen> {
  bool _copied = false;

  Future<void> _copy() async {
    await Clipboard.setData(ClipboardData(text: widget.identity.peerId));
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
                    'Your identity is a cryptographic key pair — no phone '
                    'number, no account. Share your peer ID to start chatting.',
                    textAlign: TextAlign.center,
                    style: TextStyle(
                      color: Wp.textDim,
                      fontSize: 13,
                      height: 1.45,
                    ),
                  ),
                  const SizedBox(height: 26),
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
                        SelectableText(
                          widget.identity.peerId,
                          style: const TextStyle(
                            color: Wp.accent,
                            fontSize: 16,
                            fontFamily: 'monospace',
                            letterSpacing: 1.0,
                          ),
                        ),
                        const SizedBox(height: 10),
                        Text(
                          'Keys stay on this device. Back them up if you ever '
                          'want to restore this identity.',
                          style: TextStyle(
                            color: Wp.textFaint,
                            fontSize: 11,
                            height: 1.4,
                          ),
                        ),
                      ],
                    ),
                  ),
                  const SizedBox(height: 24),
                  FilledButton.icon(
                    onPressed: _copy,
                    style: FilledButton.styleFrom(
                      backgroundColor: Wp.panel3,
                      foregroundColor: Wp.text,
                      side: const BorderSide(color: Wp.line),
                      padding: const EdgeInsets.symmetric(vertical: 14),
                    ),
                    icon: Icon(
                      _copied ? Icons.check : Icons.copy,
                      size: 18,
                      color: _copied ? Wp.online : Wp.textDim,
                    ),
                    label: Text(
                      _copied ? 'Copied!' : 'Copy peer ID',
                      style: const TextStyle(fontWeight: FontWeight.w600),
                    ),
                  ),
                  const SizedBox(height: 10),
                  FilledButton(
                    onPressed: widget.onDone,
                    style: FilledButton.styleFrom(
                      backgroundColor: Wp.accent,
                      foregroundColor: Wp.accentFg,
                      padding: const EdgeInsets.symmetric(vertical: 14),
                      textStyle: const TextStyle(
                        fontSize: 15,
                        fontWeight: FontWeight.w700,
                      ),
                    ),
                    child: const Text('Start chatting'),
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
