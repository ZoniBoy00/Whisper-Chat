import { useState } from "react";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  FileKey,
  KeyRound,
  Loader2,
  Lock,
  ServerOff,
  ShieldCheck,
} from "lucide-react";
import { Logo } from "./Logo";

interface OnboardingProps {
  onCreated: (peerId: string) => void;
}

interface IdentityResult {
  peer_id: string;
  exists: boolean;
}

function TrustPoint({ icon, label }: { icon: ReactNode; label: string }) {
  return (
    <div className="flex flex-col items-center gap-2 rounded-2xl border border-wp-line/10 bg-wp-panel px-3 py-4">
      <span className="text-wp-accent">{icon}</span>
      <span className="text-xs font-medium text-wp-dim">{label}</span>
    </div>
  );
}

export function Onboarding({ onCreated }: OnboardingProps) {
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [restoreHint, setRestoreHint] = useState(false);

  const createIdentity = async () => {
    setCreating(true);
    setError(null);
    try {
      const result = await invoke<IdentityResult>("generate_identity");
      onCreated(result.peer_id);
    } catch (err) {
      setError(String(err));
    } finally {
      setCreating(false);
    }
  };

  return (
    <div className="relative flex h-screen items-center justify-center overflow-hidden bg-wp-bg">
      {/* Whisper-themed halo background */}
      <div className="pointer-events-none absolute -top-40 left-1/2 h-[30rem] w-[30rem] -translate-x-1/2 rounded-full bg-wp-accent/15 blur-3xl" />
      <div className="pointer-events-none absolute -bottom-44 right-14 h-96 w-96 rounded-full bg-[#2dd4bf]/10 blur-3xl" />
      <div className="pointer-events-none absolute -left-28 bottom-12 h-72 w-72 rounded-full bg-wp-accent/10 blur-3xl" />

      <div className="relative z-10 flex w-full max-w-lg animate-fade-in-up flex-col items-center px-6 text-center">
        <Logo size={84} />

        <h1 className="mt-7 font-display text-4xl font-semibold tracking-tight text-wp-text">
          Welcome to Whisper
        </h1>
        <p className="mt-3 max-w-md text-base leading-relaxed text-wp-dim">
          Your conversations are whispers — only you and the recipient can hear
          them.
        </p>

        <div className="mt-9 grid w-full grid-cols-3 gap-3">
          <TrustPoint
            icon={<ShieldCheck className="h-5 w-5" />}
            label="E2E encrypted"
          />
          <TrustPoint
            icon={<ServerOff className="h-5 w-5" />}
            label="Zero-knowledge"
          />
          <TrustPoint
            icon={<Lock className="h-5 w-5" />}
            label="Keys on device"
          />
        </div>

        <div className="mt-9 flex w-full flex-col gap-3">
          <button
            type="button"
            onClick={() => void createIdentity()}
            disabled={creating}
            className="inline-flex items-center justify-center gap-2 rounded-xl bg-wp-accent px-6 py-3.5 text-sm font-semibold text-wp-accent-fg shadow-lg shadow-wp-accent/25 transition hover:bg-wp-accent-strong disabled:cursor-not-allowed disabled:opacity-60"
          >
            {creating ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <KeyRound className="h-4 w-4" />
            )}
            {creating ? "Creating your identity…" : "Create Identity"}
          </button>

          <button
            type="button"
            onClick={() => setRestoreHint((hint) => !hint)}
            className="inline-flex items-center justify-center gap-2 rounded-xl border border-wp-line/10 bg-wp-panel px-6 py-3.5 text-sm font-semibold text-wp-text transition hover:bg-wp-panel-2"
          >
            <FileKey className="h-4 w-4" />
            Restore Identity
          </button>

          {restoreHint ? (
            <p className="mx-auto max-w-xs animate-pop-in text-xs leading-relaxed text-wp-faint">
              Restoring from an identity file is coming soon. For now your
              identity file is stored locally in the app data folder and never
              leaves this device.
            </p>
          ) : null}
        </div>

        {error ? (
          <p className="mt-4 max-w-full truncate text-xs text-wp-danger">
            {error}
          </p>
        ) : null}
      </div>
    </div>
  );
}
