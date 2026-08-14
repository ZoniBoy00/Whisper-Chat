import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:whisper_mobile/src/theme.dart';

void main() {
  group('Whisper theme', () {
    test('brand palette matches the desktop CSS variables', () {
      // --wp-bg: #111417
      expect(Wp.bg, const Color(0xFF111417));
      // --wp-panel: #1a1f24
      expect(Wp.panel, const Color(0xFF1A1F24));
      // --wp-accent: #14b8a6
      expect(Wp.accent, const Color(0xFF14B8A6));
      // --wp-text: #e9edf2
      expect(Wp.text, const Color(0xFFE9EDF2));
    });

    test('whisperTheme is a dark theme', () {
      final theme = whisperTheme();
      expect(theme.brightness, Brightness.dark);
      expect(theme.colorScheme.primary, Wp.accent);
    });
  });
}
