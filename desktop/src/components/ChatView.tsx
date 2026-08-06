import {
  Fragment,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  ChevronDown,
  ChevronUp,
  Copy,
  Info,
  Lock,
  MessageSquarePlus,
  MessagesSquare,
  Plus,
  Search,
  Trash2,
  Users,
  X,
} from "lucide-react";
import type {
  Conversation,
  Message,
  PresenceInfo,
  QuoteInfo,
} from "../types";
import {
  cx,
  dayKey,
  findMatches,
  formatDaySeparator,
  formatLastSeen,
  mediaUrl,
  shortPeerId,
} from "../lib/format";
import { copyText } from "../lib/clipboard";
import { useI18n } from "../i18n/I18nContext";
import { Avatar } from "./Avatar";
import { MessageBubble } from "./MessageBubble";
import { Composer } from "./Composer";
import { ContextMenu } from "./ContextMenu";

/** Scrolling is considered "at the bottom" within this distance in px. */
const BOTTOM_THRESHOLD = 120;

/** The emoji palette offered by the reaction picker (Signal-style presets). */
const REACTION_EMOJIS = ["👍", "❤️", "😂", "😮", "😢", "🙏", "🔥", "🎉"];

interface ChatViewProps {
  conversation: Conversation | null;
  /** Whether the active peer is currently typing (from the `typing` event). */
  typingWriters: string[];
  /** Display names for typing writers (peer id -> name), for group headers. */
  typingNames: Record<string, string>;
  /** Presence of the active peer; null when not yet known. */
  presence: PresenceInfo | null;
  /** Relay endpoint; used to resolve `/media/{hash}` avatar paths. */
  relayUrl: string;
  /** Our own peer ID; used to mark our own reactions. */
  myPeerId: string | null;
  onSend: (text: string, quote?: QuoteInfo | null) => void;
  onTypingChange: (isTyping: boolean) => void;
  /** Whether Enter sends a message in the composer (off: Enter = new line). */
  enterToSend: boolean;
  /** The active conversation's draft text (controlled, from the parent). */
  draft: string;
  /** Called with the new draft text on every composer edit. */
  onDraftChange: (text: string) => void;
  /** Opens the contact's profile dialog (WhatsApp/Signal style). */
  onOpenProfile: () => void;
  /** Opens the group info panel (WhatsApp/Signal style). Only set for groups. */
  onOpenGroupInfo: (() => void) | undefined;
  /** Deletes one message locally ("delete for me"). */
  onDeleteMessage: (messageId: string) => void;
  /** React or un-react to a message. `active` is the caller-computed state. */
  onReact: (messageId: string, emoji: string, active: boolean) => void;
}

/** The right-click state of a message: where to open the menu + the target. */
interface MessageMenuState {
  x: number;
  y: number;
  message: Message;
}

/** Horizontal emoji picker shown when "Add reaction" is chosen from the
 *  message context menu. Rendered at the click point (clamped to the
 *  viewport), dismissed by an outside press or Escape. */
function ReactionPicker({
  x,
  y,
  message,
  myPeerId,
  onReact,
  onClose,
}: {
  x: number;
  y: number;
  message: Message;
  myPeerId: string | null;
  onReact: (messageId: string, emoji: string, active: boolean) => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });

  // The picker is measured hidden at (x, y), then snapped above the click
  // point and clamped so it never overflows the window edges.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const clampedX = Math.max(8, Math.min(x, window.innerWidth - rect.width - 8));
    const clampedY = Math.max(
      8,
      Math.min(y - rect.height - 8, window.innerHeight - rect.height - 8)
    );
    setPos({ x: clampedX, y: clampedY });
  }, [x, y]);

  // Dismiss on outside pointer press or Escape.
  useEffect(() => {
    const handlePointerDown = (event: PointerEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) {
        onClose();
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      role="menu"
      aria-label={t("chat.react_to_message")}
      style={{ left: pos.x, top: pos.y, visibility: "visible" }}
      className="fixed z-50 flex animate-menu-in items-center gap-0.5 rounded-full border border-wp-line/10 bg-wp-panel-2 p-1.5 shadow-2xl shadow-black/50"
    >
      {REACTION_EMOJIS.map((emoji) => (
        <button
          key={emoji}
          type="button"
          role="menuitem"
          onClick={() => {
            const mine = (message.reactions ?? []).find(
              (r) => r.sender === myPeerId
            );
            onReact(message.id, emoji, !(mine?.emoji === emoji));
            onClose();
          }}
          className="rounded-full px-1 py-0.5 text-xl leading-none transition hover:bg-wp-panel-3 active:scale-90"
        >
          {emoji}
        </button>
      ))}
    </div>
  );
}

/** One match of the in-chat search: which message and where in its text. */
interface SearchMatch {
  messageId: string;
  start: number;
  end: number;
}

export function ChatView({
  conversation,
  typingWriters,
  typingNames,
  presence,
  relayUrl,
  myPeerId,
  onSend,
  onTypingChange,
  enterToSend,
  draft,
  onDraftChange,
  onOpenProfile,
  onOpenGroupInfo,
  onDeleteMessage,
  onReact,
}: ChatViewProps) {
  const { t, language } = useI18n();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [menu, setMenu] = useState<MessageMenuState | null>(null);
  /** The message we are replying to, arming the composer reply bar. */
  const [replyTo, setReplyTo] = useState<Message | null>(null);
  /** The reaction picker popover target (position + message). */
  const [emojiPicker, setEmojiPicker] = useState<MessageMenuState | null>(null);

  // ---- In-chat message search --------------------------------------------
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [currentMatch, setCurrentMatch] = useState(0);
  const searchInputRef = useRef<HTMLInputElement>(null);
  // Ref per message row so the active search result can be scrolled to.
  const messageRowRefs = useRef<Record<string, HTMLDivElement | null>>({});

  // ---- Scroll-to-bottom "New messages" jump ------------------------------
  const [unseenCount, setUnseenCount] = useState(0);
  const [showJumpToBottom, setShowJumpToBottom] = useState(false);

  // A context menu, reply and reaction picker belong to one conversation:
  // switching chats dismisses all of them.
  useEffect(() => {
    setMenu(null);
    setReplyTo(null);
    setEmojiPicker(null);
  }, [conversation?.peerId]);

  // Every newly-opened conversation starts at its newest message.
  const conversationId = conversation?.peerId ?? null;
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [conversationId]);

  // Only the newest message animates in. Tracking the previous message count
  // means history already on screen never replays its entrance (on first load
  // or after switching conversations), and status flips (no length change)
  // do not retrigger it either. New messages auto-scroll only when the reader
  // is already at the bottom (or they are our own); otherwise a Telegram-style
  // "New messages" pill appears so the scroll position is never yanked.
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
      setUnseenCount(0);
      setShowJumpToBottom(false);
      return;
    }
    const prev = lastCountRef.current;
    lastCountRef.current = list.length;
    if (list.length > prev) {
      const added = list.slice(prev);
      setNewMessageIds(new Set(added.map((m) => m.id)));
      const el = scrollRef.current;
      const atBottom = el
        ? el.scrollHeight - el.scrollTop - el.clientHeight < BOTTOM_THRESHOLD
        : true;
      if (added.some((m) => m.outgoing) || atBottom) {
        if (el) el.scrollTop = el.scrollHeight;
        setUnseenCount(0);
        setShowJumpToBottom(false);
      } else {
        setUnseenCount((count) => count + added.length);
        setShowJumpToBottom(true);
      }
    }
  }, [conversation, conversationId]);

  const handleScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    if (el.scrollHeight - el.scrollTop - el.clientHeight < BOTTOM_THRESHOLD) {
      setUnseenCount((count) => (count === 0 ? count : 0));
      setShowJumpToBottom((shown) => (shown ? false : shown));
    }
  };

  const jumpToBottom = () => {
    const el = scrollRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
    setUnseenCount(0);
    setShowJumpToBottom(false);
  };

  // ---- Search result computation & navigation ----------------------------
  const searchMatches = useMemo<SearchMatch[]>(() => {
    const q = searchQuery.trim();
    if (!q) return [];
    const matches: SearchMatch[] = [];
    conversation?.messages.forEach((message) => {
      for (const range of findMatches(message.text, q)) {
        matches.push({
          messageId: message.id,
          start: range.start,
          end: range.end,
        });
      }
    });
    return matches;
  }, [conversation, searchQuery]);

  // Clamp the cursor when the result set shrinks (e.g. a narrower query).
  useEffect(() => {
    if (searchMatches.length === 0) {
      setCurrentMatch(0);
    } else if (currentMatch >= searchMatches.length) {
      setCurrentMatch(searchMatches.length - 1);
    }
  }, [searchMatches, currentMatch]);

  // Bring the active match into view when navigating.
  useEffect(() => {
    if (searchMatches.length === 0 || currentMatch >= searchMatches.length) {
      return;
    }
    const el = messageRowRefs.current[searchMatches[currentMatch].messageId];
    if (el) el.scrollIntoView({ block: "center" });
  }, [currentMatch, searchMatches]);

  // Reset the whole search surface when the conversation changes.
  useEffect(() => {
    setSearchOpen(false);
    setSearchQuery("");
    setCurrentMatch(0);
  }, [conversationId]);

  useEffect(() => {
    if (searchOpen) searchInputRef.current?.focus();
  }, [searchOpen]);

  const closeSearch = () => {
    setSearchOpen(false);
    setSearchQuery("");
    setCurrentMatch(0);
  };

  const previousMatch = () => {
    if (searchMatches.length === 0) return;
    setCurrentMatch(
      (index) => (index - 1 + searchMatches.length) % searchMatches.length
    );
  };

  const nextMatch = () => {
    if (searchMatches.length === 0) return;
    setCurrentMatch((index) => (index + 1) % searchMatches.length);
  };

  const activeMatch = searchMatches.length
    ? searchMatches[currentMatch] ?? null
    : null;

  if (!conversation) {
    return (
      <main className="flex min-w-0 flex-1 flex-col items-center justify-center gap-4 bg-wp-bg px-8">
        <div className="flex h-20 w-20 items-center justify-center rounded-3xl bg-wp-panel text-wp-faint shadow-inner">
          <MessagesSquare className="h-10 w-10" strokeWidth={1.8} />
        </div>
        <div className="text-center">
          <h2 className="text-xl font-semibold tracking-tight text-wp-text">
            {t("chat.select_conversation")}
          </h2>
          <p className="mx-auto mt-2 max-w-sm text-sm leading-relaxed text-wp-dim">
            {t("chat.select_conversation_hint")}
          </p>
        </div>
      </main>
    );
  }

  const isGroup = conversation.isGroup === true;
  const displayName = isGroup
    ? conversation.name
    : conversation.displayName ?? shortPeerId(conversation.peerId, 16);
  const avatarSrc = conversation.avatarUrl
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
          aria-label={
            isGroup
              ? t("chat.view_group_info_aria", { name: displayName })
              : t("chat.view_profile_aria", { name: displayName })
          }
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
                {conversation.memberCount
                  ? `${t("common.members_count", { n: conversation.memberCount })} · `
                  : ""}
                {t("common.end_to_end_encrypted")}
              </p>
            ) : typingWriters.length > 0 ? (
              <p
                aria-live="polite"
                className="mt-0.5 inline-flex items-center gap-1.5 text-sm font-semibold text-wp-read"
              >
                {isGroup && typingWriters.length > 1
                  ? t("chat.typing_many", { n: typingWriters.length })
                  : isGroup
                    ? t("chat.typing_name", {
                        name:
                          typingNames[typingWriters[0]] ??
                          shortPeerId(typingWriters[0]),
                      })
                    : t("chat.typing")}
                <span
                  className="flex items-center gap-0.5"
                  aria-hidden="true"
                >
                  <span className="typing-dot" />
                  <span className="typing-dot" />
                  <span className="typing-dot" />
                </span>
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
                  {t("common.online")}
                </p>
              ) : (
                <p
                  aria-live="polite"
                  className="mt-0.5 inline-flex items-center gap-1.5 text-sm font-medium text-wp-dim"
                >
                  {presence.last_seen != null
                    ? `${t("chat.last_seen_prefix")}${formatLastSeen(presence.last_seen, t)}`
                    : t("common.last_seen_unavailable")}
                </p>
              )
            ) : (
              <p className="mt-0.5 inline-flex items-center gap-1.5 rounded-full border border-wp-line/10 bg-wp-panel-2 px-2.5 py-0.5 text-xs font-semibold tracking-wide text-wp-dim">
                <Lock className="h-3 w-3 text-wp-accent" aria-hidden="true" />
                {t("common.end_to_end_encrypted")}
              </p>
            )}
          </div>
        </button>
        {isGroup && onOpenGroupInfo ? (
          <button
            type="button"
            onClick={onOpenGroupInfo}
            title={t("common.group_info")}
            aria-label={t("common.group_info")}
            className="rounded-lg p-2 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-text"
          >
            <Info className="h-5 w-5" />
          </button>
        ) : null}
        <button
          type="button"
          onClick={() => setSearchOpen((open) => !open)}
          title={t("chat.search_open_aria")}
          aria-label={t("chat.search_open_aria")}
          aria-expanded={searchOpen}
          className="rounded-lg p-2 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-text"
        >
          <Search className="h-5 w-5" />
        </button>
      </header>

      {searchOpen ? (
        <div className="flex items-center gap-2 border-b border-wp-line/10 bg-wp-panel-2 px-5 py-2">
          <Search className="h-4 w-4 shrink-0 text-wp-faint" aria-hidden="true" />
          <input
            ref={searchInputRef}
            type="search"
            value={searchQuery}
            onChange={(event) => {
              setSearchQuery(event.target.value);
              setCurrentMatch(0);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") closeSearch();
            }}
            placeholder={t("chat.search_placeholder")}
            aria-label={t("chat.search_aria")}
            autoComplete="off"
            spellCheck={false}
            className="w-full bg-transparent text-sm text-wp-text placeholder-wp-faint outline-none"
          />
          <span
            aria-live="polite"
            className={cx(
              "shrink-0 text-xs tabular-nums",
              searchQuery.trim() !== "" && searchMatches.length === 0
                ? "text-wp-faint"
                : "text-wp-dim"
            )}
          >
            {searchQuery.trim() !== "" && searchMatches.length > 0
              ? `${currentMatch + 1}/${searchMatches.length}`
              : searchQuery.trim() !== ""
                ? t("chat.search_no_results")
                : ""}
          </span>
          <button
            type="button"
            onClick={previousMatch}
            disabled={searchMatches.length === 0}
            title={t("chat.search_prev_aria")}
            aria-label={t("chat.search_prev_aria")}
            className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text disabled:opacity-40"
          >
            <ChevronUp className="h-4 w-4" />
          </button>
          <button
            type="button"
            onClick={nextMatch}
            disabled={searchMatches.length === 0}
            title={t("chat.search_next_aria")}
            aria-label={t("chat.search_next_aria")}
            className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text disabled:opacity-40"
          >
            <ChevronDown className="h-4 w-4" />
          </button>
          <button
            type="button"
            onClick={closeSearch}
            title={t("chat.search_close_aria")}
            aria-label={t("chat.search_close_aria")}
            className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      ) : null}

      <div className="relative min-h-0 flex-1">
        <div
          ref={scrollRef}
          role="log"
          aria-label={t("chat.messages_with", { name: displayName })}
          onScroll={handleScroll}
          className="select-chat h-full overflow-y-auto px-6 py-6"
        >
          <div className="mx-auto flex max-w-3xl flex-col gap-2">
            {conversation.messages.map((message, messageIndex) => {
              const previous = conversation.messages[messageIndex - 1];
              const showDay =
                messageIndex === 0 ||
                dayKey(previous?.timestamp ?? 0) !== dayKey(message.timestamp);
              const activeRange =
                activeMatch?.messageId === message.id
                  ? { start: activeMatch.start, end: activeMatch.end }
                  : null;
              return (
                <Fragment key={message.id}>
                  {showDay ? (
                    <div className="flex justify-center pt-1">
                      <span className="animate-pop-in rounded-full bg-wp-panel-2 px-3 py-1 text-xs font-semibold text-wp-dim shadow-sm ring-1 ring-wp-line/10">
                        {formatDaySeparator(message.timestamp, t, language)}
                      </span>
                    </div>
                  ) : null}
                  <div
                    ref={(el) => {
                      messageRowRefs.current[message.id] = el;
                    }}
                  >
                    <MessageBubble
                      message={message}
                      myPeerId={myPeerId}
                      animate={newMessageIds.has(message.id)}
                      searchQuery={searchQuery.trim()}
                      searchActiveRange={activeRange}
                      onReact={(target, emoji) => {
                        const mine = (target.reactions ?? []).find(
                          (r) => r.sender === myPeerId
                        );
                        onReact(
                          target.id,
                          emoji,
                          !(mine?.emoji === emoji)
                        );
                      }}
                      onContextMenu={(event) => {
                        event.preventDefault();
                        setMenu({ x: event.clientX, y: event.clientY, message });
                      }}
                    />
                  </div>
                </Fragment>
              );
            })}
          </div>
        </div>

        {showJumpToBottom ? (
          <button
            type="button"
            onClick={jumpToBottom}
            className="animate-jump-in absolute bottom-4 right-6 z-10 inline-flex items-center gap-2 rounded-full border border-wp-line/10 bg-wp-panel-2 px-4 py-2 text-sm font-semibold text-wp-text shadow-lg shadow-black/40 transition hover:bg-wp-panel-3 active:scale-95"
          >
            <ChevronDown className="h-4 w-4 text-wp-accent" aria-hidden="true" />
            {t("chat.new_messages")}
            {unseenCount > 1 ? (
              <span className="inline-flex h-5 min-w-5 items-center justify-center rounded-full bg-wp-accent px-1.5 text-xs font-bold tabular-nums text-wp-accent-fg">
                {unseenCount}
              </span>
            ) : null}
          </button>
        ) : null}
      </div>

      {menu ? (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          label={t("chat.actions_for_message", { name: displayName })}
          onClose={() => setMenu(null)}
          items={[
            {
              id: "reply",
              label: t("chat.reply"),
              icon: <MessageSquarePlus className="h-4 w-4" />,
              onSelect: () => {
                setReplyTo(menu.message);
                setMenu(null);
              },
            },
            {
              id: "add-reaction",
              label: t("chat.add_reaction"),
              icon: <Plus className="h-4 w-4" />,
              onSelect: () => {
                // Replace the menu with the reaction picker at the same spot.
                setEmojiPicker({ x: menu.x, y: menu.y, message: menu.message });
                setMenu(null);
              },
            },
            {
              id: "copy-text",
              label: t("chat.copy_text"),
              icon: <Copy className="h-4 w-4" />,
              onSelect: () => void copyText(menu.message.text),
            },
            {
              id: "delete-for-me",
              label: t("chat.delete_for_me"),
              danger: true,
              icon: <Trash2 className="h-4 w-4" />,
              onSelect: () => onDeleteMessage(menu.message.id),
            },
          ]}
        />
      ) : null}

      {emojiPicker ? (
        <ReactionPicker
          x={emojiPicker.x}
          y={emojiPicker.y}
          message={emojiPicker.message}
          myPeerId={myPeerId}
          onReact={onReact}
          onClose={() => setEmojiPicker(null)}
        />
      ) : null}

      <Composer
        value={draft}
        onChange={onDraftChange}
        conversationId={conversationId}
        onSend={(text) => {
          if (replyTo) {
            const quote: QuoteInfo = {
              message_id: replyTo.id,
              text: replyTo.text,
              sender: replyTo.outgoing ? myPeerId ?? "" : conversation?.peerId ?? "",
              sender_name: replyTo.outgoing
                ? t("composer.yourself")
                : (conversation?.displayName ?? shortPeerId(conversation?.peerId ?? "")),
            };
            onSend(text, quote);
            setReplyTo(null);
          } else {
            onSend(text);
          }
        }}
        replyTo={replyTo}
        replyToName={
          replyTo
            ? replyTo.outgoing
              ? t("composer.yourself")
              : (conversation?.displayName ?? shortPeerId(conversation?.peerId ?? ""))
            : undefined
        }
        onCancelReply={() => setReplyTo(null)}
        onTypingChange={onTypingChange}
        enterToSend={enterToSend}
      />
    </main>
  );
}
