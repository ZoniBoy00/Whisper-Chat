import { Check, CheckCheck } from "lucide-react";
import type { Message } from "../types";
import { cx, formatTime } from "../lib/format";

interface MessageBubbleProps {
  message: Message;
  /** Animate the entrance — only newly appended messages set this. */
  animate?: boolean;
}

export function MessageBubble({ message, animate = false }: MessageBubbleProps) {
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
      >
        <p className="whitespace-pre-wrap break-words text-[15px] leading-relaxed text-wp-text">
          {message.text}
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
              aria-label={read ? "Read" : delivered ? "Delivered" : "Sent"}
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
