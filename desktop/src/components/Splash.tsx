import { useEffect } from "react";
import { emit } from "@tauri-apps/api/event";
import { Logo } from "./Logo";

/** How long the splash stays visible before signalling the Rust side. */
const SPLASH_HOLD_MS = 1800;
/** Event the Rust side listens for to close the splash and open the main window. */
const SPLASH_DONE_EVENT = "splash-done";

/**
 * Branded splash screen shown while the app boots. The window is frameless and
 * transparent, so the splash is a floating rounded card on the desktop — no
 * title bar, no controls, just the brand (Discord-style). The entrance is a
 * short staggered reveal: logo, wordmark, then a comet-like two-segment loader.
 */
export function Splash() {
  useEffect(() => {
    // The splash window is transparent; make the document background see-through
    // so the desktop shows around the rounded card (the main window keeps the
    // opaque dark body). Cleaned up on unmount in case this view ever re-mounts.
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";

    const timer = window.setTimeout(() => {
      void emit(SPLASH_DONE_EVENT).catch(() => {
        // A failed emit must never break the handoff: the Rust side has its
        // own timeout fallback that opens the main window regardless.
      });
    }, SPLASH_HOLD_MS);
    return () => {
      window.clearTimeout(timer);
      document.documentElement.style.background = "";
      document.body.style.background = "";
    };
  }, []);

  return (
    <div
      data-theme="dark"
      className="flex h-screen w-screen flex-col items-center justify-center p-3"
    >
      <div className="relative flex h-full w-full flex-col items-center justify-center overflow-hidden rounded-3xl border border-wp-line/5 bg-wp-deep shadow-2xl shadow-black/60">
        {/* Ambient brand gradients inside the card. */}
        <div
          aria-hidden="true"
          className="pointer-events-none absolute -top-28 left-1/2 h-80 w-80 -translate-x-1/2 rounded-full bg-wp-accent/15 blur-3xl"
        />
        <div
          aria-hidden="true"
          className="pointer-events-none absolute -bottom-32 -right-20 h-72 w-72 rounded-full bg-[#2dd4bf]/10 blur-3xl"
        />

        {/* Logo with its breathing glow halo. */}
        <div className="relative animate-logo-in">
          <div className="animate-splash-glow rounded-3xl">
            <Logo size={104} />
          </div>
        </div>

        {/* Wordmark rising in. */}
        <h1 className="relative mt-8 animate-wordmark-in font-display text-5xl font-semibold tracking-tight text-wp-text">
          Whisper
        </h1>
        <p
          className="relative mt-3 animate-fade-in-soft text-xs font-medium uppercase tracking-[0.3em] text-wp-faint"
          style={{ animationDelay: "0.25s" }}
        >
          End-to-end encrypted
        </p>

        {/* Two-segment loader: a bright head dragging a long, fading tail. */}
        <div
          aria-hidden="true"
          className="relative mt-12 h-1.5 w-56 animate-fade-in-soft overflow-hidden rounded-full bg-wp-panel-3"
          style={{ animationDelay: "0.4s" }}
        >
          <span className="splash-sweep-bar absolute inset-y-0 left-0 w-24 rounded-full bg-gradient-to-r from-transparent via-wp-accent/50 to-wp-accent" />
          <span
            className="splash-sweep-bar absolute inset-y-0 left-0 w-9 rounded-full bg-gradient-to-r from-wp-accent via-wp-accent-strong to-wp-accent-strong shadow-[0_0_14px_rgb(var(--wp-accent)/0.8)]"
            style={{ animationDelay: "-0.45s" }}
          />
        </div>
      </div>
    </div>
  );
}
