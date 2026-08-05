import { useEffect, useRef, useState } from "react";
import {
  AtSign,
  CheckCircle2,
  KeyRound,
  Loader2,
  Moon,
  Palette,
  Save,
  Sun,
  Trash2,
  Upload,
  User,
} from "lucide-react";
import type { Theme } from "../../types";
import { cx, mediaUrl } from "../../lib/format";
import { Avatar } from "../Avatar";
import { CopyButton } from "../CopyButton";
import { SectionHeading } from "./controls";

/** Reserved handles that can never be claimed. */
const RESERVED_USERNAMES = new Set([
  "admin",
  "whisper",
  "support",
  "mod",
  "system",
  "root",
]);

const MAX_AVATAR_BYTES = 2 * 1024 * 1024;

/** Live-validate a candidate username; returns an error string or null. */
function usernameError(value: string): string | null {
  if (!value) return null;
  if (!/^[a-z0-9_]+$/.test(value)) {
    return "Usernames use lowercase letters, digits and underscores only.";
  }
  if (value.length < 3 || value.length > 32) {
    return "Usernames must be 3–32 characters.";
  }
  if (RESERVED_USERNAMES.has(value)) {
    return "That username is reserved.";
  }
  return null;
}

interface GeneralTabProps {
  active: boolean;
  peerId: string;
  /** Our own public display name; null when unset. */
  myDisplayName: string | null;
  /** Our registered username; null when not yet registered. */
  myUsername: string | null;
  /** Our avatar path ("/media/{hash}"); null when unset. */
  myAvatarUrl: string | null;
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
  /** Relay endpoint; used to resolve `/media/{hash}` avatar paths. */
  relayUrl: string;
  /** Persist a new display name; empty clears it. */
  onSaveDisplayName: (name: string) => Promise<void>;
  /** Register a public username for our identity. */
  onRegisterUsername: (username: string) => Promise<void>;
  /** Upload a new avatar image (raw base64 without the data: prefix). */
  onSetAvatar: (avatarBase64: string) => Promise<void>;
  onReset: () => void;
  /** Report an in-flight save/register so the dialog can block closing until
   *  the operation settles. */
  onBusyChange: (busy: boolean) => void;
}

export function GeneralTab({
  active,
  peerId,
  myDisplayName,
  myUsername,
  myAvatarUrl,
  theme,
  onThemeChange,
  relayUrl,
  onSaveDisplayName,
  onRegisterUsername,
  onSetAvatar,
  onReset,
  onBusyChange,
}: GeneralTabProps) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [nameInput, setNameInput] = useState(myDisplayName ?? "");
  const [usernameInput, setUsernameInput] = useState(myUsername ?? "");
  const [editingUsername, setEditingUsername] = useState(false);
  const [registeredFlash, setRegisteredFlash] = useState(false);
  const [avatarPreview, setAvatarPreview] = useState<string | null>(null);
  const [avatarBase64, setAvatarBase64] = useState<string | null>(null);
  const [savingName, setSavingName] = useState(false);
  const [registering, setRegistering] = useState(false);
  const [savingAvatar, setSavingAvatar] = useState(false);
  const [savedName, setSavedName] = useState(false);
  const [savedAvatar, setSavedAvatar] = useState(false);
  const [nameError, setNameError] = useState<string | null>(null);
  const [usernameErrorText, setUsernameErrorText] = useState<string | null>(null);
  const [avatarError, setAvatarError] = useState<string | null>(null);
  const [confirmingReset, setConfirmingReset] = useState(false);

  // Keep the dialog's close-guard informed of any in-flight operation. The
  // dialog remounts this tab on every open, so the form is always seeded
  // from the latest props.
  useEffect(() => {
    onBusyChange(savingName || registering || savingAvatar);
  }, [savingName, registering, savingAvatar, onBusyChange]);

  const handleReset = () => {
    if (confirmingReset) {
      setConfirmingReset(false);
      onReset();
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

  const handleRegisterUsername = async () => {
    const value = usernameInput.trim().toLowerCase();
    const err = usernameError(value);
    if (err) {
      setUsernameErrorText(err);
      return;
    }
    setRegistering(true);
    setUsernameErrorText(null);
    try {
      await onRegisterUsername(value);
      setUsernameInput(value);
      setEditingUsername(false);
      setRegisteredFlash(true);
      window.setTimeout(() => setRegisteredFlash(false), 2500);
    } catch (err) {
      setUsernameErrorText(String(err).replace(/^Error:\s*/, ""));
    } finally {
      setRegistering(false);
    }
  };

  const handleAvatarFile = (file: File | undefined) => {
    if (!file) return;
    setAvatarError(null);
    if (!/^image\/(png|jpe?g|webp)$/i.test(file.type)) {
      setAvatarError("Choose a PNG, JPEG or WebP image.");
      return;
    }
    if (file.size > MAX_AVATAR_BYTES) {
      setAvatarError("Avatar must be 2 MB or smaller.");
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      const result = String(reader.result ?? "");
      const comma = result.indexOf(",");
      // Strip the "data:image/...;base64," prefix; the backend expects raw
      // base64 (see relay.setAvatar).
      setAvatarBase64(comma >= 0 ? result.slice(comma + 1) : result);
      setAvatarPreview(result);
    };
    reader.readAsDataURL(file);
  };

  const handleSaveAvatar = async () => {
    if (!avatarBase64) return;
    setSavingAvatar(true);
    setAvatarError(null);
    try {
      await onSetAvatar(avatarBase64);
      setSavedAvatar(true);
      window.setTimeout(() => setSavedAvatar(false), 2500);
    } catch (err) {
      setAvatarError(String(err).replace(/^Error:\s*/, ""));
    } finally {
      setSavingAvatar(false);
    }
  };

  const avatarSrc = avatarPreview
    ? avatarPreview
    : myAvatarUrl
      ? mediaUrl(relayUrl, myAvatarUrl)
      : null;
  const usernameValid = usernameInput.trim() && usernameError(usernameInput.trim()) === null;

  return (
    <div
      role="tabpanel"
      id="settings-panel-general"
      aria-labelledby="settings-tab-general"
      className="space-y-6"
      hidden={!active}
    >
      {/* Profile */}
      <section aria-labelledby="settings-profile-title">
        <SectionHeading
          id="settings-profile-title"
          icon={<User className="h-3.5 w-3.5" />}
          label="Profile"
        />
        <div className="space-y-4 rounded-xl border border-wp-line/10 bg-wp-panel-3 p-4">
          <div className="flex items-center gap-4">
            <Avatar
              name={myDisplayName ?? undefined}
              size={56}
              src={avatarSrc}
            />
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

          {/* Username */}
          <div>
            <label
              htmlFor="settings-username"
              className="text-xs font-medium text-wp-dim"
            >
              Username
            </label>
            {myUsername && !editingUsername ? (
              <div className="mt-2 flex items-center gap-2">
                <p className="truncate font-mono text-sm text-wp-text">
                  @{myUsername}
                </p>
                {registeredFlash ? (
                  <p
                    className="inline-flex shrink-0 items-center gap-1 text-xs font-semibold text-wp-online"
                    role="status"
                  >
                    <CheckCircle2 className="h-3.5 w-3.5" aria-hidden="true" />
                    Registered
                  </p>
                ) : null}
                <button
                  type="button"
                  onClick={() => setEditingUsername(true)}
                  className="ml-auto shrink-0 rounded-lg border border-wp-line/10 bg-wp-panel-2 px-3 py-1.5 text-xs font-semibold text-wp-text transition hover:bg-wp-panel-3"
                >
                  Change
                </button>
              </div>
            ) : (
              <>
                <p className="mt-1 text-xs leading-relaxed text-wp-faint">
                  {myUsername
                    ? "Pick a new public handle."
                    : "Choose your username — people can find you by it."}
                </p>
                <div className="mt-2 flex gap-2">
                  <input
                    id="settings-username"
                    type="text"
                    value={usernameInput}
                    onChange={(e) => {
                      // Enforce lowercase and validate live as the user
                      // types so feedback arrives before they hit Register.
                      const value = e.target.value.toLowerCase();
                      setUsernameInput(value);
                      setUsernameErrorText(usernameError(value));
                      setRegisteredFlash(false);
                    }}
                    placeholder="e.g. alice_42"
                    maxLength={32}
                    autoComplete="off"
                    spellCheck={false}
                    aria-invalid={usernameErrorText ? true : undefined}
                    aria-describedby={
                      usernameErrorText
                        ? "settings-username-error"
                        : "settings-username-hint"
                    }
                    className="min-w-0 flex-1 rounded-xl bg-wp-panel-2 px-3.5 py-2.5 font-mono text-sm text-wp-text placeholder-wp-faint outline-none transition focus:ring-1 focus:ring-wp-accent/60"
                  />
                  <button
                    type="button"
                    onClick={() => void handleRegisterUsername()}
                    disabled={
                      registering ||
                      !usernameValid ||
                      usernameInput === (myUsername ?? "")
                    }
                    className={cx(
                      "inline-flex shrink-0 items-center gap-2 rounded-xl bg-wp-accent px-4 py-2.5 text-sm font-semibold text-wp-accent-fg transition hover:bg-wp-accent-strong",
                      "disabled:cursor-not-allowed disabled:opacity-50"
                    )}
                  >
                    {registering ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                    ) : (
                      <AtSign className="h-3.5 w-3.5" aria-hidden="true" />
                    )}
                    {registering ? "Registering…" : "Register"}
                  </button>
                </div>
                <p
                  id="settings-username-hint"
                  className="mt-2 text-xs leading-snug text-wp-faint"
                >
                  3–32 characters, lowercase letters, digits and
                  underscores. Reserved: admin, whisper, support, mod,
                  system, root.
                </p>
                {usernameErrorText ? (
                  <p
                    id="settings-username-error"
                    role="alert"
                    className="mt-2 text-xs leading-snug text-wp-danger"
                  >
                    {usernameErrorText}
                  </p>
                ) : null}
              </>
            )}
          </div>

          {/* Avatar */}
          <div>
            <p className="text-xs font-medium text-wp-dim">Avatar</p>
            <p className="mt-1 text-xs leading-snug text-wp-faint">
              Shown next to your messages. PNG, JPEG or WebP, up to 2 MB.
            </p>
            <div className="mt-2 flex items-center gap-2">
              <button
                type="button"
                onClick={() => fileInputRef.current?.click()}
                className="inline-flex shrink-0 items-center gap-2 rounded-xl bg-wp-panel-2 px-4 py-2.5 text-sm font-semibold text-wp-text transition hover:bg-wp-panel-3"
              >
                <Upload className="h-3.5 w-3.5" aria-hidden="true" />
                {avatarPreview ? "Choose another" : "Upload avatar"}
              </button>
              <input
                ref={fileInputRef}
                type="file"
                accept="image/png,image/jpeg,image/webp"
                className="sr-only"
                aria-hidden="true"
                tabIndex={-1}
                onChange={(e) => {
                  handleAvatarFile(e.target.files?.[0]);
                  e.target.value = "";
                }}
              />
              {avatarBase64 ? (
                <button
                  type="button"
                  onClick={() => void handleSaveAvatar()}
                  disabled={savingAvatar}
                  className={cx(
                    "inline-flex shrink-0 items-center gap-2 rounded-xl bg-wp-accent px-4 py-2.5 text-sm font-semibold text-wp-accent-fg transition hover:bg-wp-accent-strong",
                    "disabled:cursor-not-allowed disabled:opacity-50"
                  )}
                >
                  {savingAvatar ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                  ) : (
                    <Save className="h-3.5 w-3.5" aria-hidden="true" />
                  )}
                  {savedAvatar ? "Saved" : savingAvatar ? "Saving…" : "Save"}
                </button>
              ) : null}
            </div>
            {avatarError ? (
              <p
                role="alert"
                className="mt-2 text-xs leading-snug text-wp-danger"
              >
                {avatarError}
              </p>
            ) : null}
          </div>

          {/* Display name */}
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
                  "inline-flex shrink-0 items-center gap-2 rounded-xl bg-wp-accent px-4 py-2.5 text-sm font-semibold text-wp-accent-fg transition hover:bg-wp-accent-strong",
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
              className="mt-2 text-xs leading-snug text-wp-faint"
            >
              Public profile data — shown to people who start a chat with
              you. 64 characters max.
            </p>
            {nameError ? (
              <p
                id="settings-name-error"
                role="alert"
                className="mt-2 text-xs leading-snug text-wp-danger"
              >
                {nameError}
              </p>
            ) : null}
          </div>
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
              <p className="mt-0.5 text-xs leading-snug text-wp-faint">
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
    </div>
  );
}
