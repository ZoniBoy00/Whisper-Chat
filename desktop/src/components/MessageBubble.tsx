import { useState, type MouseEvent, type ReactNode } from "react";
import { Check, CheckCheck, SmilePlus } from "lucide-react";
import type { Message } from "../types";
import { cx, findMatches, formatTime, shortPeerId } from "../lib/format";
import { useI18n } from "../i18n/I18nContext";

/** The emoji palette offered by the quick-reaction picker. */
const REACTION_EMOJIS = ["👍", "❤️", "😂", "😮", "😢", "🙏", "🔥", "🎉"];

interface MessageBubbleProps {
  message: Message;
  /** Our own peer ID; used to highlight our own reactions in the pills. */
  myPeerId?: string | null;
  /** Animate the entrance — only newly appended messages set this. */
  animate?: boolean;
  /** Right-click handler (opens the message context menu). */
  onContextMenu?: (event: MouseEvent<HTMLDivElement>) => void;
  /** Clicking an existing reaction pill toggles it (React / un-react). */
  onReact?: (message: Message, emoji: string) => void;
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

/** Group a message's reactions by emoji for the pill row (emoji → count).
 *  A pill counts as "mine" when one of my own reactions uses that emoji. */
function groupReactions(
  message: Message,
  myPeerId: string | null | undefined
): Array<{ emoji: string; count: number; mine: boolean }> {
  const grouped = new Map<string, { count: number; mine: boolean }>();
  for (const reaction of message.reactions ?? []) {
    const entry = grouped.get(reaction.emoji) ?? { count: 0, mine: false };
    entry.count += 1;
    if (reaction.sender === myPeerId) entry.mine = true;
    grouped.set(reaction.emoji, entry);
  }
  return [...grouped.entries()].map(([emoji, entry]) => ({
    emoji,
    ...entry,
  }));
}

export function MessageBubble({
  message,
  myPeerId,
  animate = false,
  onContextMenu,
  onReact,
  searchQuery = "",
  searchActiveRange = null,
}: MessageBubbleProps) {
  const { t } = useI18n();
  const { outgoing } = message;
  // The inline reaction picker opens on the quick-reaction button click.
  const [pickerOpen, setPickerOpen] = useState(false);
  // Read receipts: "sent" = single gray tick, "delivered" = double gray tick,
  // "read" = double blue tick.
  const read = outgoing && message.status === "read";
  const delivered = outgoing && message.status === "delivered";
  const doubleTick = read || delivered;
  const StatusIcon = doubleTick ? CheckCheck : Check;
  const reactions = groupReactions(message, myPeerId);

  return (
    <div
      className={cx(
        "group flex flex-col",
        animate && "animate-msg-in",
        outgoing ? "items-end" : "items-start"
      )}
    >
      <div
        className={cx(
          "relative max-w-[68%] rounded-2xl px-4 py-2.5 shadow-sm shadow-black/20",
          outgoing
            ? "rounded-br-md bg-gradient-to-br from-wp-bubble-out-2 to-wp-bubble-out"
            : "rounded-bl-md bg-wp-bubble-in"
        )}
        onContextMenu={onContextMenu}
      >
        {/* Quick-reaction button: always visible so reactions are discoverable. */}
        {onReact ? (
          <button
            type="button"
            onClick={() => setPickerOpen((open) => !open)}
            title={t("chat.add_reaction")}
            aria-label={t("chat.add_reaction")}
            aria-expanded={pickerOpen}
            className={cx(
              "absolute -top-3 z-10 inline-flex h-6 w-6 items-center justify-center rounded-full bg-wp-panel-2 text-wp-dim shadow-md shadow-black/30 ring-1 ring-wp-line/10 transition hover:text-wp-accent active:scale-90",
              outgoing ? "-left-3" : "-right-3"
            )}
          >
            <SmilePlus className="h-3.5 w-3.5" aria-hidden="true" />
          </button>
        ) : null}
        {/* Inline emoji picker (a child of the bubble, so hover stays true). */}
        {pickerOpen ? (
          <div
            className={cx(
              "absolute -top-11 z-20 flex items-center gap-0.5 rounded-full bg-wp-panel-2 p-1.5 shadow-xl shadow-black/40 ring-1 ring-wp-line/10",
              outgoing ? "-left-3" : "-right-3"
            )}
            role="menu"
            aria-label={t("chat.react_to_message")}
          >
            {REACTION_EMOJIS.map((emoji) => (
              <button
                key={emoji}
                type="button"
                role="menuitem"
                onClick={() => {
                  onReact?.(message, emoji);
                  setPickerOpen(false);
                }}
                className="rounded-full px-0.5 py-0.5 text-lg leading-none transition hover:bg-wp-panel-3 active:scale-90"
              >
                {emoji}
              </button>
            ))}
          </div>
        ) : null}
        {message.quote ? (
          <div className="mb-2 rounded-lg border-l-2 border-wp-accent/60 bg-black/15 px-3 py-1.5">
            <p className="truncate text-xs font-semibold text-wp-accent">
              {message.quote.sender_name ?? shortPeerId(message.quote.sender)}
            </p>
            <p className="wp-msg line-clamp-2 text-xs text-wp-dim">
              {message.quote.text}
            </p>
          </div>
        ) : null}
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
      {reactions.length > 0 ? (
        <div
          className={cx(
            "mt-0.5 flex max-w-[68%] flex-wrap gap-1",
            outgoing ? "justify-end" : "justify-start"
          )}
        >
          {reactions.map(({ emoji, count, mine }) => (
            <button
              key={emoji}
              type="button"
              onClick={() => onReact?.(message, emoji)}
              title={t("bubble.react")}
              className={cx(
                "inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-sm shadow-sm shadow-black/20 ring-1 transition active:scale-95",
                mine
                  ? "bg-wp-accent/20 ring-wp-accent/60"
                  : "bg-wp-panel-2 ring-wp-line/10 hover:bg-wp-panel-3"
              )}
            >
              <span aria-hidden="true">{emoji}</span>
              <span className="text-xs font-semibold tabular-nums text-wp-dim">
                {count}
              </span>
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
