import { useState } from "react";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  FileKey,
  KeyRound,
  Languages,
  Loader2,
  Lock,
  ServerOff,
  ShieldCheck,
} from "lucide-react";
import { Logo } from "./Logo";
import { useI18n } from "../i18n/I18nContext";
import { useToast } from "../hooks/useToast";
import { importIdentity } from "../lib/relay";
import { cx } from "../lib/format";

interface OnboardingProps {
  onCreated: (peerId: string) => void;
}

interface IdentityResult {
  peer_id: string;
  exists: boolean;
}

/** Language option labels are proper nouns shown in their own language. */
const LANGUAGE_OPTIONS: { value: "en" | "fi"; label: string }[] = [
  { value: "en", label: "English" },
  { value: "fi", label: "Suomi" },
];

function TrustPoint({ icon, label }: { icon: ReactNode; label: string }) {
  return (
    <div className="flex flex-col items-center gap-2 rounded-2xl border border-wp-line/10 bg-wp-panel px-3 py-4">
      <span className="text-wp-accent">{icon}</span>
      <span className="text-xs font-medium text-wp-dim">{label}</span>
    </div>
  );
}

export function Onboarding({ onCreated }: OnboardingProps) {
  const { t, language, setLanguage } = useI18n();
  const toast = useToast().toast;
  const [name, setName] = useState("");
  const [creating, setCreating] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const createIdentity = async () => {
    setCreating(true);
    setError(null);
    try {
      const result = await invoke<IdentityResult>("generate_identity", {
        displayName: name.trim() || null,
      });
      onCreated(result.peer_id);
    } catch (err) {
      setError(String(err));
    } finally {
      setCreating(false);
    }
  };

  /** Restore a previously backed-up identity file: open the native picker,
   *  import over the (missing) local identity, then reload so the app boots
   *  into the restored profile. */
  const handleRestore = async () => {
    setRestoring(true);
    setError(null);
    try {
      await importIdentity();
      toast(t("toast.identity_imported"), "success");
      toast(t("toast.identity_import_restart"), "info");
      window.setTimeout(() => window.location.reload(), 1500);
    } catch (err) {
      const message = String(err).replace(/^Error:\s*/, "");
      // A user-cancelled picker is not an error worth shouting about.
      if (!message.toLowerCase().includes("cancel")) {
        toast(message, "error");
      }
      setError(message);
    } finally {
      setRestoring(false);
    }
  };

  return (
    <div className="relative flex h-screen items-center justify-center overflow-hidden bg-wp-bg">
      {/* Whisper-themed halo background */}
      <div className="pointer-events-none absolute -top-40 left-1/2 h-[30rem] w-[30rem] -translate-x-1/2 rounded-full bg-wp-accent/15 blur-3xl" />
      <div className="pointer-events-none absolute -bottom-44 right-14 h-96 w-96 rounded-full bg-[#2dd4bf]/10 blur-3xl" />
      <div className="pointer-events-none absolute -left-28 bottom-12 h-72 w-72 rounded-full bg-wp-accent/10 blur-3xl" />

      {/* Language selector, top-right corner */}
      <div className="absolute right-4 top-4 z-20 flex gap-1 rounded-xl bg-wp-panel-2/80 p-1 backdrop-blur">
        {LANGUAGE_OPTIONS.map((option) => (
          <button
            key={option.value}
            type="button"
            aria-pressed={language === option.value}
            onClick={() => setLanguage(option.value)}
            className={cx(
              "inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-semibold transition",
              language === option.value
                ? "bg-wp-accent text-wp-accent-fg"
                : "text-wp-dim hover:text-wp-text"
            )}
          >
            <Languages className="h-3.5 w-3.5" aria-hidden="true" />
            {option.label}
          </button>
        ))}
      </div>

      {/* Scrollable content: short windows must never clip the restore hint */}
      <div className="relative z-10 h-full w-full overflow-y-auto">
        <div className="flex min-h-full flex-col items-center justify-center px-6 py-8">
          <div className="flex w-full max-w-lg animate-fade-in-up flex-col items-center text-center">
            <Logo size={84} />

            <h1 className="mt-7 font-display text-4xl font-semibold tracking-tight text-wp-text">
              {t("onboarding.welcome_title")}
            </h1>
            <p className="mt-3 max-w-md text-base leading-relaxed text-wp-dim">
              {t("onboarding.welcome_subtitle")}
            </p>

            <div className="mt-9 grid w-full grid-cols-3 gap-3">
              <TrustPoint
                icon={<ShieldCheck className="h-5 w-5" />}
                label={t("onboarding.trust_e2e")}
              />
              <TrustPoint
                icon={<ServerOff className="h-5 w-5" />}
                label={t("onboarding.trust_zero_knowledge")}
              />
              <TrustPoint
                icon={<Lock className="h-5 w-5" />}
                label={t("onboarding.trust_keys")}
              />
            </div>

            <div className="mt-9 flex w-full flex-col gap-3">
              <label
                htmlFor="onboarding-display-name"
                className="block text-left"
              >
                <span className="text-sm font-semibold text-wp-text">
                  {t("onboarding.name_label")}
                </span>
                <span className="mt-1 block text-xs leading-relaxed text-wp-dim">
                  {t("onboarding.name_hint")}
                </span>
              </label>
              <input
                id="onboarding-display-name"
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={t("onboarding.name_placeholder")}
                maxLength={64}
                autoComplete="off"
                spellCheck={false}
                className="w-full rounded-xl bg-wp-panel px-4 py-3 text-sm text-wp-text placeholder-wp-faint outline-none transition focus:ring-1 focus:ring-wp-accent/60"
              />

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
                {creating ? t("onboarding.creating_identity") : t("onboarding.create_identity")}
              </button>

              <button
                type="button"
                onClick={() => void handleRestore()}
                disabled={restoring}
                className="inline-flex items-center justify-center gap-2 rounded-xl border border-wp-line/10 bg-wp-panel px-6 py-3.5 text-sm font-semibold text-wp-text transition hover:bg-wp-panel-2 disabled:cursor-not-allowed disabled:opacity-60"
              >
                {restoring ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <FileKey className="h-4 w-4" />
                )}
                {t("onboarding.restore_identity")}
              </button>

              <p className="mx-auto max-w-sm text-xs leading-relaxed text-wp-faint">
                {t("onboarding.restore_identity_hint")}
              </p>
            </div>

            {error ? (
              <p className="mt-4 max-w-full truncate text-xs text-wp-danger">
                {error}
              </p>
            ) : null}
          </div>
        </div>
      </div>
    </div>
  );
}
