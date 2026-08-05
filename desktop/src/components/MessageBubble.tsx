import { CheckCheck } from "lucide-react";
import type { Message } from "../types";
import { cx, formatTime } from "../lib/format";

interface MessageBubbleProps {
  message: Message;
}

export function MessageBubble({ message }: MessageBubbleProps) {
  const { outgoing } = message;

  return (
    <div
      className={cx("flex animate-msg-in", outgoing ? "justify-end" : "justify-start")}
    >
      <div
        className={cx(
          "max-w-[68%] rounded-2xl px-4 py-2 shadow-sm shadow-black/20",
          outgoing
            ? "rounded-br-md bg-gradient-to-br from-wp-bubble-out-2 to-wp-bubble-out"
            : "rounded-bl-md bg-wp-bubble-in"
        )}
      >
        <p className="whitespace-pre-wrap break-words text-sm leading-relaxed text-wp-text">
          {message.text}
        </p>
        <div
          className={cx(
            "mt-1 flex items-center justify-end gap-1",
            outgoing ? "text-wp-accent" : "text-wp-faint"
          )}
        >
          {outgoing ? <CheckCheck className="h-3 w-3" strokeWidth={2.4} /> : null}
          <span className="text-[10px] font-medium tabular-nums">
            {formatTime(message.timestamp)}
          </span>
        </div>
      </div>
    </div>
  );
}
