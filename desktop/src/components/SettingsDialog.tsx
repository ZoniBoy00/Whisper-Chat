import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import {
  AtSign,
  Bell,
  CheckCircle2,
  EyeOff,
  Info,
  KeyRound,
  Loader2,
  MessageSquare,
  Moon,
  Palette,
  Save,
  Server,
  Settings,
  ShieldCheck,
  Sun,
  Trash2,
  Upload,
  User,
  X,
} from "lucide-react";
import { cx, mediaUrl } from "../lib/format";
import { Avatar } from "./Avatar";
import { CopyButton } from "./CopyButton";

type Theme = "dark" | "light";

type TabId = "general" | "privacy" | "notifications" | "about";

const TABS: { id: TabId; label: string; icon: typeof Settings }[] = [
  { id: "general", label: "General", icon: Settings },
  { id: "privacy", label: "Privacy", icon: ShieldCheck },
  { id: "notifications", label: "Notifications", icon: Bell },
  { id: "about", label: "About", icon: Info },
];

interface SettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  peerId: string;
  /** Our own public display name; null when unset. */
  myDisplayName: string | null;
  /** Our registered username; null when not yet registered. */
  myUsername: string | null;
  /** Our avatar path ("/media/{hash}"); null when unset. */
  myAvatarUrl: string | null;
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
  relayUrl: string;
  onSaveRelayUrl: (url: string) => Promise<void>;
  /** Persist a new display name; empty clears it. */
  onSaveDisplayName: (name: string) => Promise<void>;
  /** Register a public username for our identity. */
  onRegisterUsername: (username: string) => Promise<void>;
  /** Upload a new avatar image (raw base64 without the data: prefix). */
  onSetAvatar: (avatarBase64: string) => Promise<void>;
  onReset: () => void;
  /** Privacy toggles. */
  presenceVisible: boolean;
  onPresenceVisibleChange: (value: boolean) => void;
  readReceipts: boolean;
  onReadReceiptsChange: (value: boolean) => void;
  typingIndicator: boolean;
  onTypingIndicatorChange: (value: boolean) => void;
  /** Notification toggles. */
  notificationsEnabled: boolean;
  onNotificationsEnabledChange: (value: boolean) => void;
  notificationPreview: boolean;
  onNotificationPreviewChange: (value: boolean) => void;
}

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

/** A labelled switch row with an accessible `role="switch"` control. */
function ToggleRow({
  id,
  checked,
  onChange,
  title,
  description,
  icon,
  disabled,
}: {
  id: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  title: string;
  description: string;
  icon: ReactNode;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-start justify-between gap-4 rounded-xl border border-wp-line/10 bg-wp-panel-3 p-4">
      <div className="flex items-start gap-3">
        <span className="mt-0.5 text-wp-accent" aria-hidden="true">
          {icon}
        </span>
        <div>
          <p className="text-sm font-medium text-wp-text">{title}</p>
          <p className="mt-0.5 text-xs leading-snug text-wp-faint">
            {description}
          </p>
        </div>
      </div>
      <button
        type="button"
        role="switch"
        id={id}
        aria-checked={checked}
        aria-label={title}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={cx(
          "relative h-6 w-11 shrink-0 rounded-full transition-colors",
          checked ? "bg-wp-accent" : "bg-wp-panel-2 ring-1 ring-inset ring-wp-line/20",
          "disabled:cursor-not-allowed disabled:opacity-50"
        )}
      >
        <span
          aria-hidden="true"
          className={cx(
            "absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform",
            checked ? "translate-x-5" : "translate-x-0"
          )}
        />
      </button>
    </div>
  );
}

export function SettingsDialog({
  open,
  onOpenChange,
  peerId,
  myDisplayName,
  myUsername,
  myAvatarUrl,
  theme,
  onThemeChange,
  relayUrl,
  onSaveRelayUrl,
  onSaveDisplayName,
  onRegisterUsername,
  onSetAvatar,
  onReset,
  presenceVisible,
  onPresenceVisibleChange,
  readReceipts,
  onReadReceiptsChange,
  typingIndicator,
  onTypingIndicatorChange,
  notificationsEnabled,
  onNotificationsEnabledChange,
  notificationPreview,
  onNotificationPreviewChange,
}: SettingsDialogProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [activeTab, setActiveTab] = useState<TabId>("general");
  const [relayInput, setRelayInput] = useState("");
  const [nameInput, setNameInput] = useState("");
  const [usernameInput, setUsernameInput] = useState("");
  const [editingUsername, setEditingUsername] = useState(false);
  const [registeredFlash, setRegisteredFlash] = useState(false);
  const [avatarPreview, setAvatarPreview] = useState<string | null>(null);
  const [avatarBase64, setAvatarBase64] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [savingName, setSavingName] = useState(false);
  const [registering, setRegistering] = useState(false);
  const [savingAvatar, setSavingAvatar] = useState(false);
  const [saved, setSaved] = useState(false);
  const [savedName, setSavedName] = useState(false);
  const [savedAvatar, setSavedAvatar] = useState(false);
  const [relayError, setRelayError] = useState<string | null>(null);
  const [nameError, setNameError] = useState<string | null>(null);
  const [usernameErrorText, setUsernameErrorText] = useState<string | null>(null);
  const [avatarError, setAvatarError] = useState<string | null>(null);
  const [confirmingReset, setConfirmingReset] = useState(false);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      // Seed the form from the latest settings each time the dialog opens.
      setActiveTab("general");
      setRelayInput(relayUrl);
      setNameInput(myDisplayName ?? "");
      setUsernameInput(myUsername ?? "");
      setEditingUsername(false);
      setRegisteredFlash(false);
      setAvatarPreview(null);
      setAvatarBase64(null);
      setSaved(false);
      setSavedName(false);
      setSavedAvatar(false);
      setRelayError(null);
      setNameError(null);
      setUsernameErrorText(null);
      setAvatarError(null);
      setConfirmingReset(false);
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const close = () => {
    if (saving || savingName || registering || savingAvatar) return;
    onOpenChange(false);
  };

  /** Arrow-key navigation for the tab strip (a11y: WAI-ARIA tabs pattern). */
  const handleTabKey = (event: React.KeyboardEvent) => {
    const index = TABS.findIndex((tab) => tab.id === activeTab);
    let next = index;
    if (event.key === "ArrowRight") next = (index + 1) % TABS.length;
    else if (event.key === "ArrowLeft") next = (index - 1 + TABS.length) % TABS.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = TABS.length - 1;
    else return;
    event.preventDefault();
    const tab = TABS[next];
    setActiveTab(tab.id);
    document.getElementById(`settings-tab-${tab.id}`)?.focus();
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
    <dialog
      ref={dialogRef}
      className="wp-dialog"
      aria-labelledby="settings-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
    >
      <div className="w-[min(92vw,28rem)] rounded-2xl bg-wp-panel-2">
        <div className="flex items-center justify-between gap-4 border-b border-wp-line/10 px-5 py-4">
          <div className="flex items-center gap-3">
            <Settings className="h-4 w-4 text-wp-accent" aria-hidden="true" />
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

        {/* Tab strip (WAI-ARIA tabs pattern). */}
        <div
          role="tablist"
          aria-label="Settings sections"
          className="flex gap-1 border-b border-wp-line/10 px-4 pt-3"
        >
          {TABS.map((tab) => {
            const selected = activeTab === tab.id;
            return (
              <button
                key={tab.id}
                type="button"
                role="tab"
                id={`settings-tab-${tab.id}`}
                aria-selected={selected}
                aria-controls={`settings-panel-${tab.id}`}
                tabIndex={selected ? 0 : -1}
                onClick={() => setActiveTab(tab.id)}
                onKeyDown={handleTabKey}
                className={cx(
                  "inline-flex items-center gap-1.5 rounded-t-lg px-3 py-2 text-xs font-semibold transition",
                  selected
                    ? "border-b-2 border-wp-accent text-wp-text"
                    : "border-b-2 border-transparent text-wp-dim hover:text-wp-text"
                )}
              >
                <tab.icon className="h-3.5 w-3.5" aria-hidden="true" />
                {tab.label}
              </button>
            );
          })}
        </div>

        <div className="max-h-[70vh] space-y-6 overflow-y-auto px-5 py-5">
          {/* ---------------------------------------------------------- General */}
          <div
            role="tabpanel"
            id="settings-panel-general"
            aria-labelledby="settings-tab-general"
            className="space-y-6"
            hidden={activeTab !== "general"}
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
                      "inline-flex shrink-0 items-center gap-2 rounded-xl bg-wp-accent px-4 py-2.5 text-sm font-semibold text-wp-accent-fg transition hover:bg-wp-accent-strong",
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
                  className="mt-2 text-xs leading-snug text-wp-faint"
                >
                  Default: ws://127.0.0.1:8080/ws. Saving reconnects to the new
                  relay.
                </p>
                {relayError ? (
                  <p
                    id="settings-relay-error"
                    role="alert"
                    className="mt-2 text-xs leading-snug text-wp-danger"
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

          {/* ---------------------------------------------------------- Privacy */}
          <div
            role="tabpanel"
            id="settings-panel-privacy"
            aria-labelledby="settings-tab-privacy"
            className="space-y-3"
            hidden={activeTab !== "privacy"}
          >
            <p className="text-xs leading-relaxed text-wp-faint">
              Control what others can see about you — everything here is
              end-to-end protected by the relay.
            </p>
            <ToggleRow
              id="setting-presence-visible"
              checked={presenceVisible}
              onChange={onPresenceVisibleChange}
              icon={<EyeOff className="h-4 w-4" />}
              title="Show online status & last seen"
              description="When off, others always see you as offline with no last-seen — even while you're here."
            />
            <ToggleRow
              id="setting-read-receipts"
              checked={readReceipts}
              onChange={onReadReceiptsChange}
              icon={<CheckCircle2 className="h-4 w-4" />}
              title="Read receipts"
              description="When off, we don't send receipts when you read messages. Receipts others send you are still shown — you can't stop others from seeing you've read them."
            />
            <ToggleRow
              id="setting-typing-indicator"
              checked={typingIndicator}
              onChange={onTypingIndicatorChange}
              icon={<MessageSquare className="h-4 w-4" />}
              title="Typing indicator"
              description="When off, the peer never sees that you're typing."
            />
          </div>

          {/* ------------------------------------------------------ Notifications */}
          <div
            role="tabpanel"
            id="settings-panel-notifications"
            aria-labelledby="settings-tab-notifications"
            className="space-y-3"
            hidden={activeTab !== "notifications"}
          >
            <ToggleRow
              id="setting-notifications-enabled"
              checked={notificationsEnabled}
              onChange={onNotificationsEnabledChange}
              icon={<Bell className="h-4 w-4" />}
              title="Show desktop notifications"
              description="Shows an HTML5 notification for new messages while the window isn't focused. If the system notification permission was denied, the toggle stays on but nothing is shown."
            />
            <ToggleRow
              id="setting-notification-preview"
              checked={notificationPreview}
              onChange={onNotificationPreviewChange}
              icon={<EyeOff className="h-4 w-4" />}
              title="Preview message text in notifications"
              description="When off, notifications only say \u201cNew message from @name\u201d without the message content."
            />
          </div>

          {/* ------------------------------------------------------------ About */}
          <div
            role="tabpanel"
            id="settings-panel-about"
            aria-labelledby="settings-tab-about"
            className="space-y-6"
            hidden={activeTab !== "about"}
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
        </div>
      </div>
    </dialog>
  );
}
