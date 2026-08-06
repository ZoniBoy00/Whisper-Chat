import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type MouseEvent,
  type ReactNode,
} from "react";
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
  // The inline reaction picker opens on the quick-reaction button click. It is
  // positioned fixed (viewport coordinates from the bubble's rect) so the chat
  // list's overflow can never clip it.
  const bubbleRef = useRef<HTMLDivElement>(null);
  const pickerRef = useRef<HTMLDivElement>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pickerPos, setPickerPos] = useState<{ x: number; y: number } | null>(null);
  const [pickerClamped, setPickerClamped] = useState<{ x: number; y: number } | null>(null);

  const togglePicker = () => {
    const next = !pickerOpen;
    setPickerOpen(next);
    if (next && bubbleRef.current) {
      const rect = bubbleRef.current.getBoundingClientRect();
      setPickerPos({
        x: rect.left + rect.width / 2,
        y: rect.top,
      });
      setPickerClamped(null);
    }
  };

  // Measure the picker once and clamp it to the viewport so it never gets cut
  // off by the window edge or the chat list's overflow.
  useLayoutEffect(() => {
    if (!pickerOpen || !pickerPos) return;
    const el = pickerRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    setPickerClamped({
      x: Math.max(8, Math.min(pickerPos.x - rect.width / 2, window.innerWidth - rect.width - 8)),
      y: Math.max(8, Math.min(pickerPos.y - rect.height - 8, window.innerHeight - rect.height - 8)),
    });
  }, [pickerOpen, pickerPos]);

  // Dismiss the picker on any press outside it (or Escape).
  useEffect(() => {
    if (!pickerOpen) return;
    const handlePointerDown = (event: PointerEvent) => {
      if (pickerRef.current && !pickerRef.current.contains(event.target as Node)) {
        setPickerOpen(false);
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setPickerOpen(false);
    };
    document.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [pickerOpen]);
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
        ref={bubbleRef}
        className={cx(
          "max-w-[68%] rounded-2xl px-4 py-2.5 shadow-sm shadow-black/20",
          outgoing
            ? "rounded-br-md bg-gradient-to-br from-wp-bubble-out-2 to-wp-bubble-out"
            : "rounded-bl-md bg-wp-bubble-in"
        )}
        onContextMenu={onContextMenu}
      >
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
      {/* Reaction row: quick-react button + pills, UNDER the bubble so nothing
          gets clipped by the chat list or the window edge. */}
      {onReact || reactions.length > 0 ? (
        <div
          className={cx(
            "relative z-20 mt-1 flex max-w-[68%] flex-wrap items-center gap-1",
            outgoing ? "justify-end" : "justify-start"
          )}
        >
          {pickerOpen && pickerPos ? (
            <div
              ref={pickerRef}
              style={{
                left: (pickerClamped ?? pickerPos).x,
                top: (pickerClamped ?? pickerPos).y,
                visibility: pickerClamped ? "visible" : "hidden",
              }}
              className="fixed z-50 flex items-center gap-0.5 rounded-full bg-wp-panel-2 p-1.5 shadow-2xl shadow-black/50 ring-1 ring-wp-line/10"
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
          {onReact ? (
            <button
              type="button"
              onClick={togglePicker}
              title={t("chat.add_reaction")}
              aria-label={t("chat.add_reaction")}
              aria-expanded={pickerOpen}
              className={cx(
                "inline-flex h-6 w-6 items-center justify-center rounded-full bg-wp-panel-2 text-wp-dim shadow-sm shadow-black/20 ring-1 ring-wp-line/10 transition hover:text-wp-accent active:scale-90",
                pickerOpen && "text-wp-accent ring-wp-accent/40"
              )}
            >
              <SmilePlus className="h-3.5 w-3.5" aria-hidden="true" />
            </button>
          ) : null}
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
