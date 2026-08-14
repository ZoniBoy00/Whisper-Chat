import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'src/rust/api/whisper.dart' as core;
import 'src/rust/frb_generated.dart';
import 'src/screens/main_screen.dart';
import 'src/screens/onboarding_screen.dart';
import 'src/screens/splash_screen.dart';
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
      home: const RootScreen(),
    );
  }
}

/// Boot flow: splash -> (onboarding | main), mirroring the desktop app.
class RootScreen extends StatefulWidget {
  const RootScreen({super.key});

  @override
  State<RootScreen> createState() => _RootScreenState();
}

class _RootScreenState extends State<RootScreen> {
  bool _showSplash = true;
  bool _ready = false;
  bool _needsOnboarding = false;
  String _peerId = '';
  core.WhisperClient? _client;

  @override
  void initState() {
    super.initState();
    _boot();
  }

  Future<void> _boot() async {
    await RustLib.init();
    final prefs = await SharedPreferences.getInstance();
    final stored = prefs.getString('identity_json');
    String peerId;
    if (stored != null) {
      final info = await core.identityFromJson(json: stored);
      peerId = info.peerId;
      _needsOnboarding = false;
    } else {
      final info = await core.identityCreate();
      await prefs.setString('identity_json', info.json);
      peerId = info.peerId;
      _needsOnboarding = true;
    }
    _client = await core.WhisperClient.newInstance();
    _peerId = peerId;
    setState(() => _ready = true);
  }

  void _onSplashDone() {
    setState(() => _showSplash = false);
  }

  @override
  Widget build(BuildContext context) {
    if (_showSplash) {
      return SplashScreen(onDone: _onSplashDone);
    }
    if (!_ready) {
      // Still initializing (RustLib / identity) — keep the splash visible.
      return SplashScreen(onDone: () {});
    }
    final client = _client!;
    if (_needsOnboarding) {
      return OnboardingScreen(
        identity: core.IdentityInfo(peerId: _peerId, json: ''),
        onDone: () => setState(() => _needsOnboarding = false),
      );
    }
    return MainScreen(client: client, peerId: _peerId);
  }
}
