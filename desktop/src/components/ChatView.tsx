import { useEffect, useRef } from "react";
import { Lock, MessagesSquare } from "lucide-react";
import type { Conversation } from "../types";
import { shortPeerId } from "../lib/format";
import { Avatar } from "./Avatar";
import { MessageBubble } from "./MessageBubble";
import { Composer } from "./Composer";

interface ChatViewProps {
  conversation: Conversation | null;
  onSend: (text: string) => void;
}

export function ChatView({ conversation, onSend }: ChatViewProps) {
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

  return (
    <main className="flex min-w-0 flex-1 flex-col bg-wp-bg">
      <header className="flex items-center gap-3 border-b border-wp-line/10 bg-wp-panel px-5 py-3">
        <Avatar name={shortPeerId(conversation.peerId)} size={38} />
        <div className="min-w-0 flex-1">
          <p className="truncate font-mono text-sm font-semibold text-wp-text">
            {shortPeerId(conversation.peerId, 16)}
          </p>
          <p className="mt-0.5 inline-flex items-center gap-1.5 rounded-full border border-wp-line/10 bg-wp-panel-2 px-2.5 py-0.5 text-[10px] font-semibold tracking-wide text-wp-dim">
            <Lock className="h-3 w-3 text-wp-accent" aria-hidden="true" />
            End-to-end encrypted
          </p>
        </div>
      </header>

      <div
        ref={scrollRef}
        role="log"
        aria-label={`Messages with ${shortPeerId(conversation.peerId, 16)}`}
        className="flex-1 overflow-y-auto px-6 py-6"
      >
        <div className="mx-auto flex max-w-3xl flex-col gap-2">
          {conversation.messages.map((message) => (
            <MessageBubble key={message.id} message={message} />
          ))}
        </div>
      </div>

      <Composer onSend={onSend} />
    </main>
  );
}
