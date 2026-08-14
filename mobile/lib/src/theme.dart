import 'package:flutter/material.dart';

/// Whisper brand palette — mirrors the desktop client's CSS variables
/// (`--wp-*` in desktop/src/styles.css) so both platforms share one look.
class Wp {
  static const bg = Color(0xFF111417); // --wp-bg
  static const bgDeep = Color(0xFF0B0E10); // --wp-bg-deep
  static const panel = Color(0xFF1A1F24); // --wp-panel
  static const panel2 = Color(0xFF1F252B); // --wp-panel-2
  static const panel3 = Color(0xFF20262B); // --wp-panel-3
  static const text = Color(0xFFE9EDF2); // --wp-text
  static const textDim = Color(0xFF9AA3AD); // --wp-text-dim
  static const textFaint = Color(0xFF848E98); // --wp-text-faint
  static const accent = Color(0xFF14B8A6); // --wp-accent
  static const accentStrong = Color(0xFF0D9488); // --wp-accent-strong
  static const accentFg = Color(0xFF0B0E10); // --wp-accent-fg
  static const bubbleOut = Color(0xFF0F574F); // --wp-bubble-out
  static const bubbleOut2 = Color(0xFF12685E); // --wp-bubble-out-2
  static const bubbleIn = Color(0xFF202529); // --wp-bubble-in
  static const danger = Color(0xFFF87171); // --wp-danger
  static const online = Color(0xFF25D366); // --wp-online / success

  /// Border color used everywhere: white at low alpha.
  static const line = Color(0x1AFFFFFF);
}

/// The app-wide dark theme built from the Whisper palette.
ThemeData whisperTheme() {
  final base = ThemeData.dark(useMaterial3: true);
  return base.copyWith(
    scaffoldBackgroundColor: Wp.bg,
    colorScheme: const ColorScheme.dark(
      primary: Wp.accent,
      onPrimary: Wp.accentFg,
      secondary: Wp.accentStrong,
      surface: Wp.panel,
      onSurface: Wp.text,
      error: Wp.danger,
    ),
    appBarTheme: const AppBarTheme(
      backgroundColor: Wp.bgDeep,
      foregroundColor: Wp.text,
      elevation: 0,
      centerTitle: false,
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: Wp.panel2,
      contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(14),
        borderSide: const BorderSide(color: Wp.line),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(14),
        borderSide: const BorderSide(color: Wp.line),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(14),
        borderSide: const BorderSide(color: Wp.accent),
      ),
      hintStyle: const TextStyle(color: Wp.textFaint, fontSize: 14),
    ),
    textTheme: base.textTheme.apply(
      bodyColor: Wp.text,
      displayColor: Wp.text,
    ),
  );
}
