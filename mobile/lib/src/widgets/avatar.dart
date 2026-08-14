import 'package:flutter/material.dart';

import '../theme.dart';

/// Letter avatar — mirrors the desktop Avatar.tsx:
/// - person: circular, panel gradient, dim letter, thin ring
/// - group: circular, teal accent gradient, dark letter
class WpAvatar extends StatelessWidget {
  final String name;
  final double size;
  final bool group;
  const WpAvatar({
    super.key,
    required this.name,
    this.size = 36,
    this.group = false,
  });

  String get _initial {
    final n = name.trim();
    if (n.isEmpty) return '?';
    return n[0].toUpperCase();
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        gradient: group
            ? const LinearGradient(
                begin: Alignment.topLeft,
                end: Alignment.bottomRight,
                colors: [Color(0xD914B8A6), Wp.accentStrong],
              )
            : const LinearGradient(
                begin: Alignment.topLeft,
                end: Alignment.bottomRight,
                colors: [Wp.panel3, Wp.panel2],
              ),
        border: Border.all(color: Wp.line, width: 1),
      ),
      child: Center(
        child: Text(
          _initial,
          style: TextStyle(
            color: group ? Wp.accentFg : Wp.textDim,
            fontSize: size * 0.42,
            fontWeight: FontWeight.w600,
          ),
        ),
      ),
    );
  }
}
