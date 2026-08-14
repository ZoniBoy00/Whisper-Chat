import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'src/i18n.dart';
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

/// Boot flow: splash -> (onboarding | main), with language scope on top.
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
  String _identityJson = '';
  core.WhisperClient? _client;
  String _lang = 'en';

  @override
  void initState() {
    super.initState();
    _boot();
  }

  Future<void> _boot() async {
    await RustLib.init();
    _lang = await L10n.load();
    final prefs = await SharedPreferences.getInstance();
    final stored = prefs.getString('identity_json');
    String peerId;
    if (stored != null) {
      final info = await core.identityFromJson(json: stored);
      peerId = info.peerId;
      _identityJson = stored;
      _needsOnboarding = false;
    } else {
      final info = await core.identityCreate();
      await prefs.setString('identity_json', info.json);
      peerId = info.peerId;
      _identityJson = info.json;
      _needsOnboarding = true;
    }
    _client = await core.WhisperClient.newInstance();
    _peerId = peerId;
    setState(() => _ready = true);
  }

  void _onLanguageChanged(String lang) {
    setState(() => _lang = lang);
  }

  void _onSplashDone() {
    setState(() => _showSplash = false);
  }

  @override
  Widget build(BuildContext context) {
    final child = _buildBody();
    return LanguageScope(
      l10n: L10n(_lang),
      onLanguageChanged: _onLanguageChanged,
      child: child,
    );
  }

  Widget _buildBody() {
    if (_showSplash) {
      return SplashScreen(onDone: _onSplashDone);
    }
    if (!_ready) {
      return SplashScreen(onDone: () {});
    }
    final client = _client!;
    if (_needsOnboarding) {
      return OnboardingScreen(
        client: client,
        identityJson: _identityJson,
        peerId: _peerId,
        onDone: () => setState(() => _needsOnboarding = false),
      );
    }
    return MainScreen(client: client, peerId: _peerId);
  }
}
