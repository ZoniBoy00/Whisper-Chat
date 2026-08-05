import { Bell, EyeOff, Play, Volume2 } from "lucide-react";
import { useI18n } from "../../i18n/I18nContext";
import { playTestSound } from "../../lib/sound";
import { ToggleRow } from "./controls";

interface NotificationsTabProps {
  active: boolean;
  notificationsEnabled: boolean;
  onNotificationsEnabledChange: (value: boolean) => void;
  notificationPreview: boolean;
  onNotificationPreviewChange: (value: boolean) => void;
  notificationSound: boolean;
  onNotificationSoundChange: (value: boolean) => void;
}

export function NotificationsTab({
  active,
  notificationsEnabled,
  onNotificationsEnabledChange,
  notificationPreview,
  onNotificationPreviewChange,
  notificationSound,
  onNotificationSoundChange,
}: NotificationsTabProps) {
  const { t } = useI18n();
  return (
    <div
      role="tabpanel"
      id="settings-panel-notifications"
      aria-labelledby="settings-tab-notifications"
      className="space-y-3"
      hidden={!active}
    >
      <ToggleRow
        id="setting-notifications-enabled"
        checked={notificationsEnabled}
        onChange={onNotificationsEnabledChange}
        icon={<Bell className="h-4 w-4" />}
        title={t("notifications.desktop_title")}
        description={t("notifications.desktop_desc")}
      />
      <ToggleRow
        id="setting-notification-preview"
        checked={notificationPreview}
        onChange={onNotificationPreviewChange}
        icon={<EyeOff className="h-4 w-4" />}
        title={t("notifications.preview_title")}
        description={t("notifications.preview_desc")}
      />
      <ToggleRow
        id="setting-notification-sound"
        checked={notificationSound}
        onChange={onNotificationSoundChange}
        icon={<Volume2 className="h-4 w-4" />}
        title={t("notifications.sound_title")}
        description={t("notifications.sound_desc")}
      />
      <div className="rounded-xl border border-wp-line/10 bg-wp-panel-3 p-4">
        <button
          type="button"
          onClick={() => playTestSound()}
          className="inline-flex items-center gap-2 rounded-xl bg-wp-panel-2 px-4 py-2.5 text-sm font-semibold text-wp-text transition hover:bg-wp-panel-3"
        >
          <Play className="h-3.5 w-3.5" aria-hidden="true" />
          {t("notifications.test_sound")}
        </button>
      </div>
    </div>
  );
}
