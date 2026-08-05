import { useEffect, useRef } from "react";
import { Lock, MessagesSquare } from "lucide-react";
import type { Conversation, PresenceInfo } from "../types";
import { formatLastSeen, shortPeerId } from "../lib/format";
import { Avatar } from "./Avatar";
import { MessageBubble } from "./MessageBubble";
import { Composer } from "./Composer";

interface ChatViewProps {
  conversation: Conversation | null;
  /** Whether the active peer is currently typing (from the `typing` event). */
  isTyping: boolean;
  /** Presence of the active peer; null when not yet known. */
  presence: PresenceInfo | null;
  onSend: (text: string) => void;
  onTypingChange: (isTyping: boolean) => void;
}

export function ChatView({
  conversation,
  isTyping,
  presence,
  onSend,
  onTypingChange,
}: ChatViewProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const messageCount = conversation?.messages.length ?? 0;

  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messageCount]);

  if (!conversation) {
    return (
      <main className="flex min-w-0 flex-1 flex-col items-center justify-center gap-4 bg-wp-bg px-8">
        <div className="flex h-20 w-20 items-center justify-center rounded-3xl bg-wp-panel text-wp-faint shadow-inner">
          <MessagesSquare className="h-10 w-10" strokeWidth={1.8} />
        </div>
        <div className="text-center">
          <h2 className="text-xl font-semibold tracking-tight text-wp-text">
            Select a conversation
          </h2>
          <p className="mx-auto mt-2 max-w-sm text-sm leading-relaxed text-wp-dim">
            Pick a conversation from the sidebar to start whispering. Every
            message is end-to-end encrypted — not even Whisper can read it.
          </p>
        </div>
      </main>
    );
  }

  const displayName =
    conversation.displayName ?? shortPeerId(conversation.peerId, 16);

  return (
    <main className="flex min-w-0 flex-1 flex-col bg-wp-bg">
      <header className="flex items-center gap-3 border-b border-wp-line/10 bg-wp-panel px-5 py-3">
        <Avatar name={displayName} size={38} />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            <p
              className="truncate text-sm font-semibold text-wp-text"
              title={conversation.peerId}
            >
              {displayName}
            </p>
            {conversation.displayName ? (
              <p className="truncate font-mono text-[10px] text-wp-faint">
                {shortPeerId(conversation.peerId, 10)}
              </p>
            ) : null}
          </div>
          {isTyping ? (
            <p
              aria-live="polite"
              className="mt-0.5 inline-flex items-center gap-1.5 text-xs font-semibold text-wp-read"
            >
              <span
                className="flex items-center gap-0.5"
                aria-hidden="true"
              >
                <span className="typing-dot" />
                <span className="typing-dot" />
                <span className="typing-dot" />
              </span>
              typing…
            </p>
          ) : presence ? (
            presence.online ? (
              <p
                aria-live="polite"
                className="mt-0.5 inline-flex items-center gap-1.5 text-xs font-semibold text-wp-online"
              >
                <span
                  className="h-2 w-2 rounded-full bg-wp-online"
                  aria-hidden="true"
                />
                Online
              </p>
            ) : (
              <p
                aria-live="polite"
                className="mt-0.5 inline-flex items-center gap-1.5 text-xs font-medium text-wp-dim"
              >
                {presence.last_seen != null
                  ? `Last seen ${formatLastSeen(presence.last_seen)}`
                  : "Last seen unavailable"}
              </p>
            )
          ) : (
            <p className="mt-0.5 inline-flex items-center gap-1.5 rounded-full border border-wp-line/10 bg-wp-panel-2 px-2.5 py-0.5 text-[10px] font-semibold tracking-wide text-wp-dim">
              <Lock className="h-3 w-3 text-wp-accent" aria-hidden="true" />
              End-to-end encrypted
            </p>
          )}
        </div>
      </header>

      <div
        ref={scrollRef}
        role="log"
        aria-label={`Messages with ${displayName}`}
        className="flex-1 overflow-y-auto px-6 py-6"
      >
        <div className="mx-auto flex max-w-3xl flex-col gap-2">
          {conversation.messages.map((message) => (
            <MessageBubble key={message.id} message={message} />
          ))}
        </div>
      </div>

      <Composer onSend={onSend} onTypingChange={onTypingChange} />
    </main>
  );
}
