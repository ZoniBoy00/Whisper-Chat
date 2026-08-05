import { CheckCircle2, EyeOff, MessageSquare } from "lucide-react";
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
  return (
    <div
      role="tabpanel"
      id="settings-panel-privacy"
      aria-labelledby="settings-tab-privacy"
      className="space-y-3"
      hidden={!active}
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
  );
}
