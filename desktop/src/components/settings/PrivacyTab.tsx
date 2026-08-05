import { CheckCircle2, EyeOff, MessageSquare, Trash2 } from "lucide-react";
import { useState } from "react";
import { useI18n } from "../../i18n/I18nContext";
import { cx } from "../../lib/format";
import { SectionHeading, ToggleRow } from "./controls";

interface PrivacyTabProps {
  active: boolean;
  presenceVisible: boolean;
  onPresenceVisibleChange: (value: boolean) => void;
  readReceipts: boolean;
  onReadReceiptsChange: (value: boolean) => void;
  typingIndicator: boolean;
  onTypingIndicatorChange: (value: boolean) => void;
  /** Wipes every message on this device (contacts and sessions are kept). */
  onClearHistory: () => Promise<void>;
}

export function PrivacyTab({
  active,
  presenceVisible,
  onPresenceVisibleChange,
  readReceipts,
  onReadReceiptsChange,
  typingIndicator,
  onTypingIndicatorChange,
  onClearHistory,
}: PrivacyTabProps) {
  const { t } = useI18n();
  const [confirmingClear, setConfirmingClear] = useState(false);

  const handleClear = () => {
    if (confirmingClear) {
      setConfirmingClear(false);
      void onClearHistory();
    } else {
      setConfirmingClear(true);
    }
  };

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

      {/* History */}
      <section aria-labelledby="settings-history-title" className="pt-2">
        <SectionHeading
          id="settings-history-title"
          icon={<Trash2 className="h-3.5 w-3.5" />}
          label={t("privacy.history")}
        />
        <div className="rounded-xl border border-wp-line/10 bg-wp-panel-3 p-4">
          <p className="text-xs leading-relaxed text-wp-dim">
            {t("privacy.clear_history_desc")}
          </p>
          <button
            type="button"
            onClick={handleClear}
            className={cx(
              "mt-3 inline-flex items-center gap-2 rounded-xl px-4 py-2.5 text-xs font-semibold transition",
              confirmingClear
                ? "bg-wp-danger/15 text-wp-danger"
                : "border border-wp-line/10 text-wp-danger hover:bg-wp-danger/10"
            )}
          >
            <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
            {confirmingClear
              ? t("privacy.clear_history_confirm")
              : t("privacy.clear_history_title")}
          </button>
        </div>
      </section>
    </div>
  );
}
