import type { MouseEvent, ReactNode } from "react";
import { Check, CheckCheck } from "lucide-react";
import type { Message } from "../types";
import { cx, findMatches, formatTime } from "../lib/format";
import { useI18n } from "../i18n/I18nContext";

interface MessageBubbleProps {
  message: Message;
  /** Animate the entrance — only newly appended messages set this. */
  animate?: boolean;
  /** Right-click handler (opens the message context menu). */
  onContextMenu?: (event: MouseEvent<HTMLDivElement>) => void;
  /** Active in-chat search query; empty disables highlighting. */
  searchQuery?: string;
  /** The exact match range (inside this message's text) that is the active
   *  search result, so it renders with the stronger highlight. */
  searchActiveRange?: { start: number; end: number } | null;
}

/** Split `text` into segments and wrap every search match in a <mark>. */
function HighlightedText({
  text,
  query,
  activeRange,
}: {
  text: string;
  query: string;
  activeRange: { start: number; end: number } | null;
}) {
  const matches = findMatches(text, query);
  if (matches.length === 0) return <>{text}</>;
  const parts: ReactNode[] = [];
  let cursor = 0;
  for (const match of matches) {
    if (match.start > cursor) parts.push(text.slice(cursor, match.start));
    const active =
      activeRange !== null &&
      match.start === activeRange.start &&
      match.end === activeRange.end;
    parts.push(
      <mark
        key={`${match.start}-${match.end}`}
        className={active ? "chat-mark chat-mark-active" : "chat-mark"}
      >
        {text.slice(match.start, match.end)}
      </mark>
    );
    cursor = match.end;
  }
  if (cursor < text.length) parts.push(text.slice(cursor));
  return <>{parts}</>;
}

export function MessageBubble({
  message,
  animate = false,
  onContextMenu,
  searchQuery = "",
  searchActiveRange = null,
}: MessageBubbleProps) {
  const { t } = useI18n();
  const { outgoing } = message;
  // Read receipts: "sent" = single gray tick, "delivered" = double gray tick,
  // "read" = double blue tick.
  const read = outgoing && message.status === "read";
  const delivered = outgoing && message.status === "delivered";
  const doubleTick = read || delivered;
  const StatusIcon = doubleTick ? CheckCheck : Check;

  return (
    <div
      className={cx(
        "flex",
        animate && "animate-msg-in",
        outgoing ? "justify-end" : "justify-start"
      )}
    >
      <div
        className={cx(
          "max-w-[68%] rounded-2xl px-4 py-2.5 shadow-sm shadow-black/20",
          outgoing
            ? "rounded-br-md bg-gradient-to-br from-wp-bubble-out-2 to-wp-bubble-out"
            : "rounded-bl-md bg-wp-bubble-in"
        )}
        onContextMenu={onContextMenu}
      >
        <p className="wp-msg whitespace-pre-wrap break-words leading-relaxed text-wp-text">
          <HighlightedText
            text={message.text}
            query={searchQuery}
            activeRange={searchActiveRange}
          />
        </p>
        <div
          className={cx(
            "mt-1 flex items-center justify-end gap-1",
            outgoing ? (read ? "text-wp-read" : "text-wp-faint") : "text-wp-faint"
          )}
        >
          {outgoing ? (
            <StatusIcon
              className="h-3.5 w-3.5"
              strokeWidth={2.4}
              role="img"
              aria-label={read ? t("bubble.read") : delivered ? t("bubble.delivered") : t("bubble.sent")}
            />
          ) : null}
          <span className="text-xs font-medium tabular-nums">
            {formatTime(message.timestamp)}
          </span>
        </div>
      </div>
    </div>
  );
}
