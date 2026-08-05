import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import {
  Info,
  KeyRound,
  Loader2,
  Moon,
  Palette,
  Save,
  Server,
  Settings,
  Sun,
  Trash2,
  User,
  X,
} from "lucide-react";
import { cx } from "../lib/format";
import { Avatar } from "./Avatar";
import { CopyButton } from "./CopyButton";

type Theme = "dark" | "light";

interface SettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  peerId: string;
  /** Our own public display name; null when unset. */
  myDisplayName: string | null;
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
  relayUrl: string;
  onSaveRelayUrl: (url: string) => Promise<void>;
  /** Persist a new display name; empty clears it. */
  onSaveDisplayName: (name: string) => Promise<void>;
  onReset: () => void;
}

function SectionHeading({
  id,
  icon,
  label,
}: {
  id: string;
  icon: ReactNode;
  label: string;
}) {
  return (
    <h3
      id={id}
      className="mb-3 flex items-center gap-1.5 text-xs font-semibold uppercase tracking-widest text-wp-faint"
    >
      <span className="text-wp-accent" aria-hidden="true">
        {icon}
      </span>
      {label}
    </h3>
  );
}

export function SettingsDialog({
  open,
  onOpenChange,
  peerId,
  myDisplayName,
  theme,
  onThemeChange,
  relayUrl,
  onSaveRelayUrl,
  onSaveDisplayName,
  onReset,
}: SettingsDialogProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [relayInput, setRelayInput] = useState("");
  const [nameInput, setNameInput] = useState("");
  const [saving, setSaving] = useState(false);
  const [savingName, setSavingName] = useState(false);
  const [saved, setSaved] = useState(false);
  const [savedName, setSavedName] = useState(false);
  const [relayError, setRelayError] = useState<string | null>(null);
  const [nameError, setNameError] = useState<string | null>(null);
  const [confirmingReset, setConfirmingReset] = useState(false);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      // Seed the form from the latest settings each time the dialog opens.
      setRelayInput(relayUrl);
      setNameInput(myDisplayName ?? "");
      setSaved(false);
      setSavedName(false);
      setRelayError(null);
      setNameError(null);
      setConfirmingReset(false);
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const close = () => {
    if (saving) return;
    onOpenChange(false);
  };

  const handleSaveRelay = async () => {
    const url = relayInput.trim();
    if (!url) return;
    setSaving(true);
    setRelayError(null);
    try {
      await onSaveRelayUrl(url);
      setSaved(true);
      window.setTimeout(() => setSaved(false), 2000);
    } catch (err) {
      setRelayError(String(err).replace(/^Error:\s*/, ""));
    } finally {
      setSaving(false);
    }
  };

  const handleReset = () => {
    if (confirmingReset) {
      setConfirmingReset(false);
      onReset();
      onOpenChange(false);
    } else {
      setConfirmingReset(true);
    }
  };

  const handleSaveName = async () => {
    const name = nameInput.trim();
    if (name.length > 64) {
      setNameError("Display name must be 64 characters or fewer.");
      return;
    }
    setSavingName(true);
    setNameError(null);
    try {
      await onSaveDisplayName(name);
      setSavedName(true);
      window.setTimeout(() => setSavedName(false), 2000);
    } catch (err) {
      setNameError(String(err).replace(/^Error:\s*/, ""));
    } finally {
      setSavingName(false);
    }
  };

  return (
    <dialog
      ref={dialogRef}
      className="wp-dialog"
      aria-labelledby="settings-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
    >
      <div className="w-[min(92vw,26rem)] rounded-2xl bg-wp-panel-2">
        <div className="flex items-center justify-between gap-4 border-b border-wp-line/10 px-5 py-4">
          <div className="flex items-center gap-3">
            <Settings
              className="h-4 w-4 text-wp-accent"
              aria-hidden="true"
            />
            <h2
              id="settings-title"
              className="font-display text-lg font-semibold tracking-tight text-wp-text"
            >
              Settings
            </h2>
          </div>
          <button
            type="button"
            onClick={close}
            aria-label="Close settings"
            className="rounded-lg p-2 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
        </div>

        <div className="max-h-[70vh] space-y-6 overflow-y-auto px-5 py-5">
          {/* Profile */}
          <section aria-labelledby="settings-profile-title">
            <SectionHeading
              id="settings-profile-title"
              icon={<User className="h-3.5 w-3.5" />}
              label="Profile"
            />
            <div className="space-y-4 rounded-xl border border-wp-line/10 bg-wp-panel-3 p-4">
              <div className="flex items-center gap-4">
                <Avatar name={myDisplayName ?? undefined} size={48} />
                <div className="min-w-0 flex-1">
                  <p className="text-xs font-medium text-wp-dim">
                    Your Whisper ID
                  </p>
                  <p className="mt-1 select-all break-all font-mono text-sm text-wp-text">
                    {peerId}
                  </p>
                </div>
                <CopyButton value={peerId} label="Copy" />
              </div>
              <div>
                <label
                  htmlFor="settings-display-name"
                  className="text-xs font-medium text-wp-dim"
                >
                  Display name
                </label>
                <div className="mt-2 flex gap-2">
                  <input
                    id="settings-display-name"
                    type="text"
                    value={nameInput}
                    onChange={(e) => {
                      setNameInput(e.target.value);
                      setSavedName(false);
                    }}
                    placeholder="What should people call you?"
                    maxLength={64}
                    autoComplete="off"
                    spellCheck={false}
                    aria-invalid={nameError ? true : undefined}
                    aria-describedby={nameError ? "settings-name-error" : "settings-name-hint"}
                    className="min-w-0 flex-1 rounded-xl bg-wp-panel-2 px-3.5 py-2.5 text-sm text-wp-text placeholder-wp-faint outline-none transition focus:ring-1 focus:ring-wp-accent/60"
                  />
                  <button
                    type="button"
                    onClick={() => void handleSaveName()}
                    disabled={savingName || nameInput.trim() === (myDisplayName ?? "")}
                    className={cx(
                      "inline-flex shrink-0 items-center gap-2 rounded-xl bg-wp-accent px-4 py-2.5 text-xs font-semibold text-wp-accent-fg transition hover:bg-wp-accent-strong",
                      "disabled:cursor-not-allowed disabled:opacity-50"
                    )}
                  >
                    {savingName ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                    ) : (
                      <Save className="h-3.5 w-3.5" aria-hidden="true" />
                    )}
                    {savedName ? "Saved" : savingName ? "Saving…" : "Save"}
                  </button>
                </div>
                <p
                  id="settings-name-hint"
                  className="mt-2 text-[11px] leading-snug text-wp-faint"
                >
                  Public profile data — shown to people who start a chat with
                  you. 64 characters max.
                </p>
                {nameError ? (
                  <p
                    id="settings-name-error"
                    role="alert"
                    className="mt-2 text-[11px] leading-snug text-wp-danger"
                  >
                    {nameError}
                  </p>
                ) : null}
              </div>
            </div>
          </section>

          {/* Connections */}
          <section aria-labelledby="settings-connections-title">
            <SectionHeading
              id="settings-connections-title"
              icon={<Server className="h-3.5 w-3.5" />}
              label="Connections"
            />
            <div className="rounded-xl border border-wp-line/10 bg-wp-panel-3 p-4">
              <label
                htmlFor="settings-relay-url"
                className="text-xs font-medium text-wp-dim"
              >
                Relay address
              </label>
              <div className="mt-2 flex gap-2">
                <input
                  id="settings-relay-url"
                  type="url"
                  value={relayInput}
                  onChange={(e) => {
                    setRelayInput(e.target.value);
                    setSaved(false);
                  }}
                  placeholder="ws://127.0.0.1:8080/ws"
                  autoComplete="off"
                  spellCheck={false}
                  aria-invalid={relayError ? true : undefined}
                  aria-describedby={
                    relayError ? "settings-relay-error" : "settings-relay-hint"
                  }
                  className="min-w-0 flex-1 rounded-xl bg-wp-panel-2 px-3.5 py-2.5 font-mono text-sm text-wp-text placeholder-wp-faint outline-none transition focus:ring-1 focus:ring-wp-accent/60"
                />
                <button
                  type="button"
                  onClick={() => void handleSaveRelay()}
                  disabled={
                    saving ||
                    !relayInput.trim() ||
                    relayInput.trim() === relayUrl
                  }
                  className={cx(
                    "inline-flex shrink-0 items-center gap-2 rounded-xl bg-wp-accent px-4 py-2.5 text-xs font-semibold text-wp-accent-fg transition hover:bg-wp-accent-strong",
                    "disabled:cursor-not-allowed disabled:opacity-50"
                  )}
                >
                  {saving ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                  ) : (
                    <Save className="h-3.5 w-3.5" aria-hidden="true" />
                  )}
                  {saved ? "Saved" : saving ? "Saving…" : "Save"}
                </button>
              </div>
              <p
                id="settings-relay-hint"
                className="mt-2 text-[11px] leading-snug text-wp-faint"
              >
                Default: ws://127.0.0.1:8080/ws. Saving reconnects to the new
                relay.
              </p>
              {relayError ? (
                <p
                  id="settings-relay-error"
                  role="alert"
                  className="mt-2 text-[11px] leading-snug text-wp-danger"
                >
                  {relayError}
                </p>
              ) : null}
            </div>
          </section>

          {/* Appearance */}
          <section aria-labelledby="settings-appearance-title">
            <SectionHeading
              id="settings-appearance-title"
              icon={<Palette className="h-3.5 w-3.5" />}
              label="Appearance"
            />
            <div className="rounded-xl border border-wp-line/10 bg-wp-panel-3 p-4">
              <div className="flex items-center justify-between gap-4">
                <div>
                  <p className="text-xs font-medium text-wp-text">Theme</p>
                  <p className="mt-0.5 text-[11px] leading-snug text-wp-faint">
                    Dark is the default; your choice is remembered.
                  </p>
                </div>
                <div className="flex shrink-0 gap-1 rounded-xl bg-wp-panel-2 p-1">
                  <button
                    type="button"
                    aria-pressed={theme === "dark"}
                    onClick={() => onThemeChange("dark")}
                    className={cx(
                      "inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-semibold transition",
                      theme === "dark"
                        ? "bg-wp-accent text-wp-accent-fg"
                        : "text-wp-dim hover:text-wp-text"
                    )}
                  >
                    <Moon className="h-3.5 w-3.5" aria-hidden="true" />
                    Dark
                  </button>
                  <button
                    type="button"
                    aria-pressed={theme === "light"}
                    onClick={() => onThemeChange("light")}
                    className={cx(
                      "inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-semibold transition",
                      theme === "light"
                        ? "bg-wp-accent text-wp-accent-fg"
                        : "text-wp-dim hover:text-wp-text"
                    )}
                  >
                    <Sun className="h-3.5 w-3.5" aria-hidden="true" />
                    Light
                  </button>
                </div>
              </div>
            </div>
          </section>

          {/* Identity */}
          <section aria-labelledby="settings-identity-title">
            <SectionHeading
              id="settings-identity-title"
              icon={<KeyRound className="h-3.5 w-3.5" />}
              label="Identity"
            />
            <div className="rounded-xl border border-wp-line/10 bg-wp-panel-3 p-4">
              <p className="text-xs leading-relaxed text-wp-dim">
                Keys never leave this device. Resetting creates a fresh identity
                with a brand-new peer ID.
              </p>
              <button
                type="button"
                onClick={handleReset}
                className={cx(
                  "mt-3 inline-flex items-center gap-2 rounded-xl px-4 py-2.5 text-xs font-semibold transition",
                  confirmingReset
                    ? "bg-wp-danger/15 text-wp-danger"
                    : "border border-wp-line/10 text-wp-danger hover:bg-wp-danger/10"
                )}
              >
                <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
                {confirmingReset ? "Click again to confirm" : "Reset identity"}
              </button>
            </div>
          </section>

          {/* About */}
          <section aria-labelledby="settings-about-title">
            <SectionHeading
              id="settings-about-title"
              icon={<Info className="h-3.5 w-3.5" />}
              label="About"
            />
            <div className="rounded-xl border border-wp-line/10 bg-wp-panel-3 p-4 text-center">
              <p className="font-display text-base font-semibold text-wp-text">
                Whisper
              </p>
              <p className="mt-0.5 text-xs italic text-wp-dim">
                your conversations are whispers
              </p>
              <div className="mx-auto mt-3 h-px w-12 bg-wp-line/10" />
              <p className="mt-3 text-[11px] text-wp-faint">
                Version 0.1.0 · MIT
              </p>
              <p className="mt-1 text-[11px] text-wp-faint">
                End-to-end encrypted · Zero-knowledge relay
              </p>
            </div>
          </section>
        </div>
      </div>
    </dialog>
  );
}
