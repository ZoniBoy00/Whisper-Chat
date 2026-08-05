import { useCallback, useEffect, useRef, useState } from "react";
import { Settings, X } from "lucide-react";
import type { Theme } from "../types";
import { useI18n } from "../i18n/I18nContext";
import { AboutTab } from "./settings/AboutTab";
import { GeneralTab } from "./settings/GeneralTab";
import { LogsTab } from "./settings/LogsTab";
import { NotificationsTab } from "./settings/NotificationsTab";
import { PrivacyTab } from "./settings/PrivacyTab";
import { SettingsTabs } from "./settings/SettingsTabs";

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
  /** Relay endpoint; used to resolve `/media/{hash}` avatar paths. */
  relayUrl: string;
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
  /** Plays a short chime for incoming messages. */
  notificationSound: boolean;
  onNotificationSoundChange: (value: boolean) => void;
}

/** The dialog frame around the settings tabs: native `<dialog>` handling
 *  (open/close, Esc), the header with the close button, and the busy guard
 *  that keeps the dialog open while a save/register is in flight. The tab
 *  panels themselves live in `settings/`. */
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
  notificationSound,
  onNotificationSoundChange,
}: SettingsDialogProps) {
  const { t } = useI18n();
  const dialogRef = useRef<HTMLDialogElement>(null);
  // Incremented on every open; remounts the tab subtree so each tab re-seeds
  // its form from the latest settings (and resets validation/save flashes).
  const [sessionKey, setSessionKey] = useState(0);
  // Set while any tab has a save/register in flight; blocks closing.
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      setBusy(false);
      setSessionKey((key) => key + 1);
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const close = () => {
    if (busy) return;
    onOpenChange(false);
  };

  const handleBusyChange = useCallback((value: boolean) => {
    setBusy(value);
  }, []);

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
              {t("common.settings")}
            </h2>
          </div>
          <button
            type="button"
            onClick={close}
            aria-label={t("common.close_settings")}
            className="rounded-lg p-2 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
        </div>

        <SettingsTabs key={sessionKey}>
          {(activeTab) => (
            <>
              <GeneralTab
                active={activeTab === "general"}
                peerId={peerId}
                myDisplayName={myDisplayName}
                myUsername={myUsername}
                myAvatarUrl={myAvatarUrl}
                theme={theme}
                onThemeChange={onThemeChange}
                relayUrl={relayUrl}
                onSaveDisplayName={onSaveDisplayName}
                onRegisterUsername={onRegisterUsername}
                onSetAvatar={onSetAvatar}
                onReset={onReset}
                onBusyChange={handleBusyChange}
              />
              <PrivacyTab
                active={activeTab === "privacy"}
                presenceVisible={presenceVisible}
                onPresenceVisibleChange={onPresenceVisibleChange}
                readReceipts={readReceipts}
                onReadReceiptsChange={onReadReceiptsChange}
                typingIndicator={typingIndicator}
                onTypingIndicatorChange={onTypingIndicatorChange}
              />
              <NotificationsTab
                active={activeTab === "notifications"}
                notificationsEnabled={notificationsEnabled}
                onNotificationsEnabledChange={onNotificationsEnabledChange}
                notificationPreview={notificationPreview}
                onNotificationPreviewChange={onNotificationPreviewChange}
                notificationSound={notificationSound}
                onNotificationSoundChange={onNotificationSoundChange}
              />
              <LogsTab active={activeTab === "logs"} />
              <AboutTab active={activeTab === "about"} />
            </>
          )}
        </SettingsTabs>
      </div>
    </dialog>
  );
}
