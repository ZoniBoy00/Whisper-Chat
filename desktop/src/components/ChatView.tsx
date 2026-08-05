import { useEffect, useRef, useState } from "react";
import { Info, Lock, MessagesSquare, Users } from "lucide-react";
import type { Conversation, PresenceInfo } from "../types";
import { formatLastSeen, mediaUrl, shortPeerId } from "../lib/format";
import { Avatar } from "./Avatar";
import { MessageBubble } from "./MessageBubble";
import { Composer } from "./Composer";

interface ChatViewProps {
  conversation: Conversation | null;
  /** Whether the active peer is currently typing (from the `typing` event). */
  isTyping: boolean;
  /** Presence of the active peer; null when not yet known. */
  presence: PresenceInfo | null;
  /** Relay endpoint; used to resolve `/media/{hash}` avatar paths. */
  relayUrl: string;
  onSend: (text: string) => void;
  onTypingChange: (isTyping: boolean) => void;
  /** Opens the contact's profile dialog (WhatsApp/Signal style). */
  onOpenProfile: () => void;
  /** Opens the group info panel (WhatsApp/Signal style). Only set for groups. */
  onOpenGroupInfo: (() => void) | undefined;
}

export function ChatView({
  conversation,
  isTyping,
  presence,
  relayUrl,
  onSend,
  onTypingChange,
  onOpenProfile,
  onOpenGroupInfo,
}: ChatViewProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const messageCount = conversation?.messages.length ?? 0;

  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messageCount]);

  // Only the newest message animates in. Tracking the previous message count
  // means history already on screen never replays its entrance (on first load
  // or after switching conversations), and status flips (no length change)
  // do not retrigger it either.
  const conversationId = conversation?.peerId ?? null;
  const lastConvIdRef = useRef<string | null>(null);
  const lastCountRef = useRef(0);
  const [newMessageIds, setNewMessageIds] = useState<ReadonlySet<string>>(
    new Set()
  );

  useEffect(() => {
    const list = conversation?.messages ?? [];
    if (lastConvIdRef.current !== conversationId) {
      lastConvIdRef.current = conversationId;
      lastCountRef.current = 0;
      setNewMessageIds(new Set());
      return;
    }
    const prev = lastCountRef.current;
    lastCountRef.current = list.length;
    if (list.length > prev) {
      setNewMessageIds(new Set(list.slice(prev).map((m) => m.id)));
    }
  }, [conversation, conversationId]);

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

  const isGroup = conversation.isGroup === true;
  const displayName = isGroup
    ? conversation.name
    : conversation.displayName ?? shortPeerId(conversation.peerId, 16);
  const avatarSrc = isGroup
    ? null
    : conversation.avatarUrl
      ? mediaUrl(relayUrl, conversation.avatarUrl)
      : null;
  const headerClick = isGroup ? onOpenGroupInfo : onOpenProfile;

  return (
    <main className="flex min-w-0 flex-1 flex-col bg-wp-bg">
      <header className="flex items-center gap-3 border-b border-wp-line/10 bg-wp-panel px-5 py-3">
        {/* Clicking the peer opens the profile dialog; for groups it opens the
            group info panel. */}
        <button
          type="button"
          onClick={headerClick}
          aria-label={isGroup ? `View ${displayName} group info` : `View ${displayName}'s profile`}
          className="flex min-w-0 flex-1 items-center gap-3 text-left"
        >
          <Avatar
            name={displayName}
            size={40}
            src={avatarSrc}
            variant={isGroup ? "group" : "peer"}
          />
          <div className="min-w-0 flex-1">
            <div className="flex items-baseline gap-2">
              <p
                className="truncate text-base font-semibold text-wp-text"
                title={isGroup ? undefined : conversation.peerId}
              >
                {displayName}
              </p>
              {!isGroup && conversation.username ? (
                <p className="truncate font-mono text-xs text-wp-faint">
                  @{conversation.username}
                </p>
              ) : null}
            </div>
            {isGroup ? (
              <p className="mt-0.5 inline-flex items-center gap-1.5 text-sm font-medium text-wp-dim">
                <Users className="h-3.5 w-3.5 text-wp-faint" aria-hidden="true" />
                {conversation.memberCount ?? 0} members · end-to-end encrypted
              </p>
            ) : isTyping ? (
              <p
                aria-live="polite"
                className="mt-0.5 inline-flex items-center gap-1.5 text-sm font-semibold text-wp-read"
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
                  className="mt-0.5 inline-flex items-center gap-1.5 text-sm font-semibold text-wp-online"
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
                  className="mt-0.5 inline-flex items-center gap-1.5 text-sm font-medium text-wp-dim"
                >
                  {presence.last_seen != null
                    ? `Last seen ${formatLastSeen(presence.last_seen)}`
                    : "Last seen unavailable"}
                </p>
              )
            ) : (
              <p className="mt-0.5 inline-flex items-center gap-1.5 rounded-full border border-wp-line/10 bg-wp-panel-2 px-2.5 py-0.5 text-xs font-semibold tracking-wide text-wp-dim">
                <Lock className="h-3 w-3 text-wp-accent" aria-hidden="true" />
                End-to-end encrypted
              </p>
            )}
          </div>
        </button>
        {isGroup && onOpenGroupInfo ? (
          <button
            type="button"
            onClick={onOpenGroupInfo}
            title="Group info"
            aria-label="Group info"
            className="rounded-lg p-2 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-text"
          >
            <Info className="h-5 w-5" />
          </button>
        ) : null}
      </header>

      <div
        ref={scrollRef}
        role="log"
        aria-label={`Messages with ${displayName}`}
        className="flex-1 overflow-y-auto px-6 py-6"
      >
        <div className="mx-auto flex max-w-3xl flex-col gap-2">
          {conversation.messages.map((message) => (
            <MessageBubble
              key={message.id}
              message={message}
              animate={newMessageIds.has(message.id)}
            />
          ))}
        </div>
      </div>

      <Composer onSend={onSend} onTypingChange={onTypingChange} />
    </main>
  );
}
