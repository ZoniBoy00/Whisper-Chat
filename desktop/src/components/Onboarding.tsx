import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { KeyRound, Loader2, Sparkles } from "lucide-react";
import { Logo } from "./Logo";
import { PeerIdCard } from "./PeerIdCard";

interface OnboardingProps {
  onCreated: (peerId: string) => void;
}

interface IdentityResult {
  peer_id: string;
  exists: boolean;
}

export function Onboarding({ onCreated }: OnboardingProps) {
  const [creating, setCreating] = useState(false);
  const [createdId, setCreatedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const createIdentity = async () => {
    setCreating(true);
    setError(null);
    try {
      const result = await invoke<IdentityResult>("generate_identity");
      setCreatedId(result.peer_id);
      onCreated(result.peer_id);
    } catch (err) {
      setError(String(err));
    } finally {
      setCreating(false);
    }
  };

  return (
    <div className="relative flex h-screen items-center justify-center overflow-hidden bg-[#0a0e14]">
      <div className="pointer-events-none absolute -top-40 left-1/2 h-96 w-96 -translate-x-1/2 rounded-full bg-violet-600/20 blur-3xl" />
      <div className="pointer-events-none absolute -bottom-40 right-10 h-80 w-80 rounded-full bg-cyan-500/10 blur-3xl" />

      <div className="relative flex w-full max-w-md flex-col items-center px-6 text-center">
        <Logo size={72} />

        <h1 className="mt-6 text-3xl font-semibold tracking-tight text-slate-100">
          Welcome to Whisper
        </h1>
        <p className="mt-2 text-sm leading-relaxed text-slate-400">
          Private, end-to-end encrypted chat. Your identity is generated locally
          and never leaves this device.
        </p>

        {createdId ? (
          <div className="mt-8 w-full space-y-4">
            <PeerIdCard peerId={createdId} />
            <p className="text-xs text-slate-500">
              Share this ID with a friend so they can add you. Keep it safe — it
              is your address on Whisper.
            </p>
            <button
              type="button"
              onClick={() => onCreated(createdId)}
              className="mt-2 inline-flex items-center gap-2 rounded-xl bg-violet-600 px-6 py-3 text-sm font-semibold text-white shadow-lg shadow-violet-600/30 transition hover:bg-violet-500"
            >
              <Sparkles className="h-4 w-4" />
              Enter Whisper
            </button>
          </div>
        ) : (
          <button
            type="button"
            onClick={() => void createIdentity()}
            disabled={creating}
            className="mt-10 inline-flex items-center gap-2 rounded-xl bg-violet-600 px-6 py-3 text-sm font-semibold text-white shadow-lg shadow-violet-600/30 transition hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {creating ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <KeyRound className="h-4 w-4" />
            )}
            {creating ? "Generating..." : "Create identity"}
          </button>
        )}

        {error ? (
          <p className="mt-4 max-w-full truncate text-xs text-rose-400">{error}</p>
        ) : null}
      </div>
    </div>
  );
}
