import { Bell, EyeOff } from "lucide-react";
import { ToggleRow } from "./controls";

interface NotificationsTabProps {
  active: boolean;
  notificationsEnabled: boolean;
  onNotificationsEnabledChange: (value: boolean) => void;
  notificationPreview: boolean;
  onNotificationPreviewChange: (value: boolean) => void;
}

export function NotificationsTab({
  active,
  notificationsEnabled,
  onNotificationsEnabledChange,
  notificationPreview,
  onNotificationPreviewChange,
}: NotificationsTabProps) {
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
  );
}
