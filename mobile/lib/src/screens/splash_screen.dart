import 'dart:async';

import 'package:flutter/material.dart';

import '../theme.dart';

/// Discord-style splash: the logo fades/scales in while a two-segment loader
/// sweeps across, then the callback fires. Mirrors the desktop Splash.tsx.
class SplashScreen extends StatefulWidget {
  final VoidCallback onDone;
  const SplashScreen({super.key, required this.onDone});

  @override
  State<SplashScreen> createState() => _SplashScreenState();
}

class _SplashScreenState extends State<SplashScreen>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller;
  late final Animation<double> _fade;
  late final Animation<double> _scale;
  Timer? _timer;

  @override
  void initState() {
    super.initState();
    _controller = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 900),
    );
    _fade = CurvedAnimation(parent: _controller, curve: Curves.easeOut);
    _scale = Tween<double>(begin: 0.86, end: 1.0).animate(
      CurvedAnimation(parent: _controller, curve: Curves.easeOutBack),
    );
    _controller.forward();
    _timer = Timer(const Duration(milliseconds: 2200), widget.onDone);
  }

  @override
  void dispose() {
    _timer?.cancel();
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Wp.bg,
      body: Center(
        child: FadeTransition(
          opacity: _fade,
          child: ScaleTransition(
            scale: _scale,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                ClipRRect(
                  borderRadius: BorderRadius.circular(28),
                  child: Image.asset(
                    'assets/whisper-logo.png',
                    width: 120,
                    height: 120,
                  ),
                ),
                const SizedBox(height: 22),
                const Text(
                  'Whisper',
                  style: TextStyle(
                    color: Wp.text,
                    fontSize: 30,
                    fontWeight: FontWeight.w700,
                    letterSpacing: -0.5,
                  ),
                ),
                const SizedBox(height: 6),
                Text(
                  'Your conversations are whispers.',
                  style: TextStyle(color: Wp.textFaint, fontSize: 13),
                ),
                const SizedBox(height: 40),
                _LoaderBar(),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// Two-segment sweeping loader bar (mirrors the desktop splash loader).
class _LoaderBar extends StatefulWidget {
  @override
  State<_LoaderBar> createState() => _LoaderBarState();
}

class _LoaderBarState extends State<_LoaderBar>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller;

  @override
  void initState() {
    super.initState();
    _controller = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 1100),
    )..repeat();
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 140,
      height: 3,
      child: AnimatedBuilder(
        animation: _controller,
        builder: (context, _) {
          final t = _controller.value;
          final x1 = (t * 1.4 - 0.2).clamp(0.0, 1.0);
          final x2 = (t * 1.4).clamp(0.0, 1.0);
          return Stack(
            children: [
              Container(
                decoration: BoxDecoration(
                  color: Wp.panel3,
                  borderRadius: BorderRadius.circular(2),
                ),
              ),
              Align(
                alignment: Alignment(x1 * 2 - 1, 0),
                child: FractionallySizedBox(
                  widthFactor: (x2 - x1).clamp(0.1, 1.0),
                  child: Container(
                    decoration: BoxDecoration(
                      color: Wp.accent,
                      borderRadius: BorderRadius.circular(2),
                      boxShadow: [
                        BoxShadow(
                          color: Wp.accent.withValues(alpha: 0.5),
                          blurRadius: 6,
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ],
          );
        },
      ),
    );
  }
}
