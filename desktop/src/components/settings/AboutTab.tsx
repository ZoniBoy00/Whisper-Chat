import { KeyRound } from "lucide-react";

export function AboutTab({ active }: { active: boolean }) {
  return (
    <div
      role="tabpanel"
      id="settings-panel-about"
      aria-labelledby="settings-tab-about"
      className="space-y-6"
      hidden={!active}
    >
      <div className="rounded-xl border border-wp-line/10 bg-wp-panel-3 p-6 text-center">
        <p className="font-display text-xl font-semibold text-wp-text">
          Whisper
        </p>
        <p className="mt-0.5 text-xs italic text-wp-dim">
          your conversations are whispers
        </p>
        <div className="mx-auto mt-4 h-px w-12 bg-wp-line/10" />
        <p className="mt-4 text-xs text-wp-faint">
          Version 0.1.0 · MIT
        </p>
        <p className="mt-1 text-xs text-wp-faint">
          End-to-end encrypted · Zero-knowledge relay
        </p>
        <p className="mt-3 inline-flex items-center gap-1.5 rounded-full border border-wp-accent/30 bg-wp-accent/10 px-3 py-1 text-xs font-semibold text-wp-accent">
          <KeyRound className="h-3.5 w-3.5" aria-hidden="true" />
          Keys never leave this device
        </p>
      </div>
    </div>
  );
}
