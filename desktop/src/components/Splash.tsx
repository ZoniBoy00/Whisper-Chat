import { useEffect } from "react";
import { emit } from "@tauri-apps/api/event";
import { Logo } from "./Logo";

/** How long the splash stays visible before signalling the Rust side. */
const SPLASH_HOLD_MS = 1800;
/** Event the Rust side listens for to close the splash and open the main window. */
const SPLASH_DONE_EVENT = "splash-done";

/**
 * Branded splash screen shown while the app boots. It is the first thing the
 * user sees, so it stays quiet: logo, wordmark and a soft progress bar, all on
 * the dark theme to avoid any white flash during the handoff.
 */
export function Splash() {
  useEffect(() => {
    const timer = window.setTimeout(() => {
      void emit(SPLASH_DONE_EVENT).catch(() => {
        // A failed emit must never break the handoff: the Rust side has its
        // own timeout fallback that opens the main window regardless.
      });
    }, SPLASH_HOLD_MS);
    return () => window.clearTimeout(timer);
  }, []);

  return (
    <div
      data-theme="dark"
      className="relative flex h-screen w-screen flex-col items-center justify-center gap-7 overflow-hidden bg-wp-deep"
    >
      {/* Soft brand halo behind the mark. */}
      <div
        aria-hidden="true"
        className="pointer-events-none absolute left-1/2 top-1/2 h-72 w-72 -translate-x-1/2 -translate-y-1/2 rounded-full bg-wp-accent/10 blur-3xl"
      />

      <div className="relative animate-pop-in">
        <div className="animate-splash-glow rounded-2xl">
          <Logo size={84} />
        </div>
      </div>

      <div className="relative flex flex-col items-center gap-2">
        <h1 className="font-display text-4xl font-semibold tracking-tight text-wp-text">
          Whisper
        </h1>
        <p className="text-[0.65rem] font-medium uppercase tracking-[0.22em] text-wp-faint">
          End-to-end encrypted
        </p>
      </div>

      <div
        aria-hidden="true"
        className="relative h-1 w-44 overflow-hidden rounded-full bg-wp-panel-3"
      >
        <div className="splash-progress-bar h-full w-2/5 rounded-full bg-gradient-to-r from-wp-accent-strong to-wp-accent" />
      </div>
    </div>
  );
}
