import { useEffect, useRef, useState } from "react";
import {
  AtSign,
  CheckCircle2,
  Download,
  KeyRound,
  Languages,
  Link2,
  Loader2,
  Minimize2,
  Moon,
  Palette,
  Rocket,
  Save,
  Send,
  Sun,
  Trash2,
  Type,
  Upload,
  User,
} from "lucide-react";
import type { Theme } from "../../types";
import { cx, mediaUrl } from "../../lib/format";
import { copyText } from "../../lib/clipboard";
import { getInviteLink } from "../../lib/relay";
import { useI18n } from "../../i18n/I18nContext";
import { useToast } from "../../hooks/useToast";
import type { TFunction } from "../../i18n/types";
import { Avatar } from "../Avatar";
import { CopyButton } from "../CopyButton";
import { SectionHeading, ToggleRow } from "./controls";

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

/** Language option labels are proper nouns shown in their own language. */
const LANGUAGE_OPTIONS: { value: "en" | "fi"; label: string }[] = [
  { value: "en", label: "English" },
  { value: "fi", label: "Suomi" },
];

/** Live-validate a candidate username; returns an error string or null. */
function usernameError(value: string, t: TFunction): string | null {
  if (!value) return null;
  if (!/^[a-z0-9_]+$/.test(value)) {
    return t("general.username_chars_error");
  }
  if (value.length < 3 || value.length > 32) {
    return t("general.username_length_error");
  }
  if (RESERVED_USERNAMES.has(value)) {
    return t("general.username_reserved_error");
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
  /** Whether the app registers itself to launch at system startup. */
  autostart: boolean;
  onAutostartChange: (value: boolean) => void;
  /** Whether closing the window hides to the system tray instead of quitting. */
  minimizeToTray: boolean;
  onMinimizeToTrayChange: (value: boolean) => void;
  /** Whether Enter sends a message (off: Enter = new line, Ctrl+Enter sends). */
  enterToSend: boolean;
  onEnterToSendChange: (value: boolean) => void;
  /** Message bubble font scale: "small" | "normal" | "large". */
  messageFontScale: string;
  onMessageFontScaleChange: (value: string) => void;
  /** Back up / restore the identity file through a native file dialog. */
  onExportIdentity: () => Promise<void>;
  onImportIdentity: () => Promise<void>;
}

/** Message font scale options; labels are translated through `t`. */
const FONT_SCALE_OPTIONS: { value: string; labelKey: "general.font_small" | "general.font_normal" | "general.font_large" }[] = [
  { value: "small", labelKey: "general.font_small" },
  { value: "normal", labelKey: "general.font_normal" },
  { value: "large", labelKey: "general.font_large" },
];

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
  autostart,
  onAutostartChange,
  minimizeToTray,
  onMinimizeToTrayChange,
  enterToSend,
  onEnterToSendChange,
  messageFontScale,
  onMessageFontScaleChange,
  onExportIdentity,
  onImportIdentity,
}: GeneralTabProps) {
  const { t, language, setLanguage } = useI18n();
  const { toast } = useToast();
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
  const [inviteCopied, setInviteCopied] = useState(false);

  /** Copy our whisper:// invite link to the clipboard (best-effort). */
  const handleShareInvite = async () => {
    try {
      const link = await getInviteLink();
      const ok = await copyText(link);
      if (ok) {
        setInviteCopied(true);
        setTimeout(() => setInviteCopied(false), 1600);
      }
    } catch {
      // Best-effort: the peer ID copy button next to it always works.
    }
  };

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
      setNameError(t("general.display_name_too_long"));
      return;
    }
    setSavingName(true);
    setNameError(null);
    try {
      await onSaveDisplayName(name);
      setSavedName(true);
      window.setTimeout(() => setSavedName(false), 2000);
      toast(t("toast.display_name_saved"), "success");
    } catch (err) {
      const message = String(err).replace(/^Error:\s*/, "");
      setNameError(message);
      toast(message, "error");
    } finally {
      setSavingName(false);
    }
  };

  const handleRegisterUsername = async () => {
    const value = usernameInput.trim().toLowerCase();
    const err = usernameError(value, t);
    if (err) {
      setUsernameErrorText(err);
      toast(err, "error");
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
      const message = String(err).replace(/^Error:\s*/, "");
      setUsernameErrorText(message);
      toast(message, "error");
    } finally {
      setRegistering(false);
    }
  };

  const handleAvatarFile = (file: File | undefined) => {
    if (!file) return;
    setAvatarError(null);
    if (!/^image\/(png|jpe?g|webp)$/i.test(file.type)) {
      const message = t("general.avatar_type_error");
      setAvatarError(message);
      toast(message, "error");
      return;
    }
    if (file.size > MAX_AVATAR_BYTES) {
      const message = t("general.avatar_size_error");
      setAvatarError(message);
      toast(message, "error");
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
      const message = String(err).replace(/^Error:\s*/, "");
      setAvatarError(message);
      toast(message, "error");
    } finally {
      setSavingAvatar(false);
    }
  };

  const avatarSrc = avatarPreview
    ? avatarPreview
    : myAvatarUrl
      ? mediaUrl(relayUrl, myAvatarUrl)
      : null;
  const usernameValid = usernameInput.trim() && usernameError(usernameInput.trim(), t) === null;

  /** Apply a theme choice and confirm it with a toast. */
  const handleThemeChange = (next: Theme) => {
    onThemeChange(next);
    toast(t("toast.settings_saved"), "info");
  };

  /** Apply a message-font-scale choice and confirm it with a toast. */
  const handleFontScaleChange = (next: string) => {
    onMessageFontScaleChange(next);
    toast(t("toast.settings_saved"), "info");
  };

  /** Switch the UI language and confirm it with a toast. */
  const handleLanguageChange = (next: "en" | "fi") => {
    setLanguage(next);
    toast(t("toast.settings_saved"), "info");
  };

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
          label={t("general.profile")}
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
                {t("common.your_whisper_id")}
              </p>
              <p className="mt-1 select-all break-all font-mono text-sm text-wp-text">
                {peerId}
              </p>
            </div>
            <CopyButton value={peerId} label={t("common.copy")} />
            <button
              type="button"
              onClick={() => void handleShareInvite()}
              className={cx(
                "inline-flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-medium transition active:scale-95",
                inviteCopied
                  ? "border-wp-online/30 bg-wp-online/10 text-wp-online"
                  : "border-wp-line/10 bg-wp-panel-2 text-wp-dim hover:bg-wp-panel-3 hover:text-wp-text"
              )}
            >
              {inviteCopied ? (
                <CheckCircle2 className="h-3.5 w-3.5" aria-hidden="true" />
              ) : (
                <Link2 className="h-3.5 w-3.5" aria-hidden="true" />
              )}
              {inviteCopied ? t("common.copied") : t("common.share_invite")}
            </button>
          </div>

          {/* Username */}
          <div>
            <label
              htmlFor="settings-username"
              className="text-xs font-medium text-wp-dim"
            >
              {t("general.username")}
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
                    {t("general.registered")}
                  </p>
                ) : null}
                <button
                  type="button"
                  onClick={() => setEditingUsername(true)}
                  className="ml-auto shrink-0 rounded-lg border border-wp-line/10 bg-wp-panel-2 px-3 py-1.5 text-xs font-semibold text-wp-text transition hover:bg-wp-panel-3"
                >
                  {t("general.change")}
                </button>
              </div>
            ) : (
              <>
                <p className="mt-1 text-xs leading-relaxed text-wp-faint">
                  {myUsername
                    ? t("general.pick_new_handle")
                    : t("general.choose_username")}
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
                      setUsernameErrorText(usernameError(value, t));
                      setRegisteredFlash(false);
                    }}
                    placeholder={t("general.username_placeholder")}
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
                    {registering ? t("general.registering") : t("general.register")}
                  </button>
                </div>
                <p
                  id="settings-username-hint"
                  className="mt-2 text-xs leading-snug text-wp-faint"
                >
                  {t("general.username_hint")}
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
            <p className="text-xs font-medium text-wp-dim">{t("general.avatar")}</p>
            <p className="mt-1 text-xs leading-snug text-wp-faint">
              {t("general.avatar_hint")}
            </p>
            <div className="mt-2 flex items-center gap-2">
              <button
                type="button"
                onClick={() => fileInputRef.current?.click()}
                className="inline-flex shrink-0 items-center gap-2 rounded-xl bg-wp-panel-2 px-4 py-2.5 text-sm font-semibold text-wp-text transition hover:bg-wp-panel-3"
              >
                <Upload className="h-3.5 w-3.5" aria-hidden="true" />
                {avatarPreview ? t("general.choose_another") : t("general.upload_avatar")}
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
                  {savedAvatar ? t("general.saved") : savingAvatar ? t("general.saving") : t("general.save")}
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
              {t("general.display_name")}
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
                placeholder={t("general.what_should_people_call_you")}
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
                {savedName ? t("general.saved") : savingName ? t("general.saving") : t("general.save")}
              </button>
            </div>
            <p
              id="settings-name-hint"
              className="mt-2 text-xs leading-snug text-wp-faint"
            >
              {t("general.display_name_hint")}
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
          label={t("general.appearance")}
        />
        <div className="rounded-xl border border-wp-line/10 bg-wp-panel-3 p-4">
          <div className="flex items-center justify-between gap-4">
            <div>
              <p className="text-xs font-medium text-wp-text">{t("general.theme")}</p>
              <p className="mt-0.5 text-xs leading-snug text-wp-faint">
                {t("general.theme_hint")}
              </p>
            </div>
            <div className="flex shrink-0 gap-1 rounded-xl bg-wp-panel-2 p-1">
              <button
                type="button"
                aria-pressed={theme === "dark"}
                onClick={() => handleThemeChange("dark")}
                className={cx(
                  "inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-semibold transition",
                  theme === "dark"
                    ? "bg-wp-accent text-wp-accent-fg"
                    : "text-wp-dim hover:text-wp-text"
                )}
              >
                <Moon className="h-3.5 w-3.5" aria-hidden="true" />
                {t("general.dark")}
              </button>
              <button
                type="button"
                aria-pressed={theme === "light"}
                onClick={() => handleThemeChange("light")}
                className={cx(
                  "inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-semibold transition",
                  theme === "light"
                    ? "bg-wp-accent text-wp-accent-fg"
                    : "text-wp-dim hover:text-wp-text"
                )}
              >
                <Sun className="h-3.5 w-3.5" aria-hidden="true" />
                {t("general.light")}
              </button>
            </div>
          </div>

          <div className="my-4 h-px bg-wp-line/10" />

          <div className="flex items-center justify-between gap-4">
            <div>
              <p className="text-xs font-medium text-wp-text">{t("general.language")}</p>
              <p className="mt-0.5 text-xs leading-snug text-wp-faint">
                {t("general.language_hint")}
              </p>
            </div>
            <div className="flex shrink-0 gap-1 rounded-xl bg-wp-panel-2 p-1">
              {LANGUAGE_OPTIONS.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  aria-pressed={language === option.value}
                  onClick={() => handleLanguageChange(option.value)}
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
          </div>
        </div>
      </section>

      {/* Startup */}
      <section aria-labelledby="settings-startup-title">
        <SectionHeading
          id="settings-startup-title"
          icon={<Rocket className="h-3.5 w-3.5" />}
          label={t("general.startup")}
        />
        <div className="space-y-3">
          <ToggleRow
            id="setting-autostart"
            checked={autostart}
            onChange={onAutostartChange}
            icon={<Rocket className="h-4 w-4" />}
            title={t("general.autostart_title")}
            description={t("general.autostart_desc")}
          />
          <ToggleRow
            id="setting-minimize-to-tray"
            checked={minimizeToTray}
            onChange={onMinimizeToTrayChange}
            icon={<Minimize2 className="h-4 w-4" />}
            title={t("general.minimize_to_tray_title")}
            description={t("general.minimize_to_tray_desc")}
          />
        </div>
      </section>

      {/* Messaging */}
      <section aria-labelledby="settings-messaging-title">
        <SectionHeading
          id="settings-messaging-title"
          icon={<Send className="h-3.5 w-3.5" />}
          label={t("general.messaging")}
        />
        <div className="rounded-xl border border-wp-line/10 bg-wp-panel-3 p-4">
          <ToggleRow
            id="setting-enter-to-send"
            checked={enterToSend}
            onChange={onEnterToSendChange}
            icon={<Send className="h-4 w-4" />}
            title={t("general.enter_to_send_title")}
            description={t("general.enter_to_send_desc")}
          />

          <div className="my-4 h-px bg-wp-line/10" />

          <div className="flex items-center justify-between gap-4">
            <div>
              <p className="text-xs font-medium text-wp-text">
                {t("general.message_font_title")}
              </p>
              <p className="mt-0.5 text-xs leading-snug text-wp-faint">
                {t("general.message_font_desc")}
              </p>
            </div>
            <div className="flex shrink-0 gap-1 rounded-xl bg-wp-panel-2 p-1">
              {FONT_SCALE_OPTIONS.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  aria-pressed={messageFontScale === option.value}
                  onClick={() => handleFontScaleChange(option.value)}
                  className={cx(
                    "inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-semibold transition",
                    messageFontScale === option.value
                      ? "bg-wp-accent text-wp-accent-fg"
                      : "text-wp-dim hover:text-wp-text"
                  )}
                >
                  <Type className="h-3.5 w-3.5" aria-hidden="true" />
                  {t(option.labelKey)}
                </button>
              ))}
            </div>
          </div>
        </div>
      </section>

      {/* Identity */}
      <section aria-labelledby="settings-identity-title">
        <SectionHeading
          id="settings-identity-title"
          icon={<KeyRound className="h-3.5 w-3.5" />}
          label={t("general.identity")}
        />
        <div className="rounded-xl border border-wp-line/10 bg-wp-panel-3 p-4">
          <p className="text-xs leading-relaxed text-wp-dim">
            {t("general.identity_reset_hint")}
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
            {confirmingReset ? t("common.confirm_again") : t("common.reset_identity")}
          </button>

          <div className="my-4 h-px bg-wp-line/10" />

          <p className="text-xs leading-relaxed text-wp-dim">
            {t("general.identity_backup_hint")}
          </p>
          <p className="mt-1.5 text-xs leading-relaxed text-wp-faint">
            {t("general.restore_identity_warn")}
          </p>
          <div className="mt-3 flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => void onExportIdentity()}
              className="inline-flex items-center gap-2 rounded-xl bg-wp-panel-2 px-4 py-2.5 text-xs font-semibold text-wp-text transition hover:bg-wp-panel-3"
            >
              <Download className="h-3.5 w-3.5" aria-hidden="true" />
              {t("general.backup_identity")}
            </button>
            <button
              type="button"
              onClick={() => void onImportIdentity()}
              className="inline-flex items-center gap-2 rounded-xl bg-wp-panel-2 px-4 py-2.5 text-xs font-semibold text-wp-text transition hover:bg-wp-panel-3"
            >
              <Upload className="h-3.5 w-3.5" aria-hidden="true" />
              {t("general.restore_identity")}
            </button>
          </div>
        </div>
      </section>
    </div>
  );
}
