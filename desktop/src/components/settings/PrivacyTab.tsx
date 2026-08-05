import { CheckCircle2, EyeOff, MessageSquare } from "lucide-react";
import { useI18n } from "../../i18n/I18nContext";
import { ToggleRow } from "./controls";

interface PrivacyTabProps {
  active: boolean;
  presenceVisible: boolean;
  onPresenceVisibleChange: (value: boolean) => void;
  readReceipts: boolean;
  onReadReceiptsChange: (value: boolean) => void;
  typingIndicator: boolean;
  onTypingIndicatorChange: (value: boolean) => void;
}

export function PrivacyTab({
  active,
  presenceVisible,
  onPresenceVisibleChange,
  readReceipts,
  onReadReceiptsChange,
  typingIndicator,
  onTypingIndicatorChange,
}: PrivacyTabProps) {
  const { t } = useI18n();
  return (
    <div
      role="tabpanel"
      id="settings-panel-privacy"
      aria-labelledby="settings-tab-privacy"
      className="space-y-3"
      hidden={!active}
    >
      <p className="text-xs leading-relaxed text-wp-faint">
        {t("privacy.intro")}
      </p>
      <ToggleRow
        id="setting-presence-visible"
        checked={presenceVisible}
        onChange={onPresenceVisibleChange}
        icon={<EyeOff className="h-4 w-4" />}
        title={t("privacy.presence_title")}
        description={t("privacy.presence_desc")}
      />
      <ToggleRow
        id="setting-read-receipts"
        checked={readReceipts}
        onChange={onReadReceiptsChange}
        icon={<CheckCircle2 className="h-4 w-4" />}
        title={t("privacy.receipts_title")}
        description={t("privacy.receipts_desc")}
      />
      <ToggleRow
        id="setting-typing-indicator"
        checked={typingIndicator}
        onChange={onTypingIndicatorChange}
        icon={<MessageSquare className="h-4 w-4" />}
        title={t("privacy.typing_title")}
        description={t("privacy.typing_desc")}
      />
    </div>
  );
}
