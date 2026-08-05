import { useEffect, useRef, useState } from "react";
import {
  Copy,
  Loader2,
  MessageCircle,
  MessageCirclePlus,
  MoreVertical,
  Search,
  SearchX,
  Settings,
  SquarePen,
  Trash2,
  UserPlus,
  UserRound,
  UserX,
  Users,
} from "lucide-react";
import type { Conversation, PresenceInfo, ProfileInfo } from "../types";
import { cx, formatTime, mediaUrl, shortPeerId } from "../lib/format";
import { conversationPreview } from "../lib/chatList";
import { searchUsers } from "../lib/relay";
import { copyText } from "../lib/clipboard";
import { Avatar } from "./Avatar";
import { CopyButton } from "./CopyButton";
import { ContextMenu } from "./ContextMenu";
import type { ContextMenuItem } from "./ContextMenu";

interface SidebarProps {
  peerId: string;
  /** Our own public display name; null when unset. */
  myDisplayName: string | null;
  conversations: Conversation[];
  /** Latest known presence per peer, fed by pushes and the 30s poll. */
  presence: Record<string, PresenceInfo>;
  activeId: string | null;
  connected: boolean;
  connecting: boolean;
  /** Whether the Rust side is retrying a dropped connection automatically. */
  reconnecting: boolean;
  /** Current auto-reconnect progress; null while not reconnecting. */
  reconnectInfo: { attempt: number; nextInMs: number } | null;
  connectionError: string | null;
  /** Relay endpoint; used to resolve `/media/{hash}` avatar paths. */
  relayUrl: string;
  onSelect: (id: string) => void;
  onAddContact: () => void;
  /** Open the "New group" dialog. */
  onNewGroup: () => void;
  /** Start a chat directly with a peer (from a directory search result). */
  onStartChat: (peerId: string) => Promise<void>;
  onOpenSettings: () => void;
  onReconnect: () => void;
  onReset: () => void;
  /** Open the profile dialog for a specific 1:1 contact (context menu). */
  onOpenProfile: (peerId: string) => void;
  /** Open the group info panel for a specific group (context menu). */
  onOpenGroupInfo: (groupId: string) => void;
  /** Remove a 1:1 contact locally (context menu). */
  onRemoveContact: (peerId: string) => void;
}

/** Right-click state of a conversation row: menu position + the target. */
interface RowMenuState {
  x: number;
  y: number;
  conversation: Conversation;
}

/** Debounce for the directory search so keystrokes don't spam the backend. */
const SEARCH_DEBOUNCE_MS = 250;
/** Directory search activates once the user has typed this many characters. */
const SEARCH_MIN_CHARS = 3;

export function Sidebar({
  peerId,
  myDisplayName,
  conversations,
  presence,
  activeId,
  connected,
  connecting,
  reconnecting,
  reconnectInfo,
  connectionError,
  relayUrl,
  onSelect,
  onAddContact,
  onNewGroup,
  onStartChat,
  onOpenSettings,
  onReconnect,
  onReset,
  onOpenProfile,
  onOpenGroupInfo,
  onRemoveContact,
}: SidebarProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [rowMenu, setRowMenu] = useState<RowMenuState | null>(null);
  const [query, setQuery] = useState("");
  const [serverResults, setServerResults] = useState<ProfileInfo[] | null>(null);
  const [serverSearching, setServerSearching] = useState(false);
  const [serverSearchFailed, setServerSearchFailed] = useState(false);
  const [searchAddError, setSearchAddError] = useState<string | null>(null);
  // Ids of conversations that have just appeared, so only the new row animates
  // in. The initial mount is skipped so an existing list does not replay.
  const [enteringIds, setEnteringIds] = useState<ReadonlySet<string>>(new Set());
  const seenPeerIdsRef = useRef<Set<string> | null>(null);

  useEffect(() => {
    const seen = seenPeerIdsRef.current;
    if (!seen) {
      seenPeerIdsRef.current = new Set(conversations.map((c) => c.peerId));
      setEnteringIds(new Set());
      return;
    }
    const fresh = new Set<string>();
    for (const conversation of conversations) {
      if (!seen.has(conversation.peerId)) fresh.add(conversation.peerId);
    }
    if (fresh.size > 0) {
      setEnteringIds(fresh);
      for (const id of fresh) seen.add(id);
    }
  }, [conversations]);

  const trimmedQuery = query.trim().toLowerCase();

  // Directory search (username/ID lookup) with debounce. If the backend
  // command is not wired up yet, `search_users` rejects and we silently fall
  // back to the local conversation filter below.
  useEffect(() => {
    const trimmed = query.trim().toLowerCase();
    if (trimmed.length < SEARCH_MIN_CHARS) {
      setServerResults(null);
      setServerSearchFailed(false);
      setServerSearching(false);
      return;
    }
    setServerSearching(true);
    setSearchAddError(null);
    const handle = window.setTimeout(() => {
      searchUsers(trimmed, 10)
        .then((results) => {
          if (query.trim().toLowerCase() === trimmed) {
            setServerResults(results);
            setServerSearchFailed(false);
          }
        })
        .catch(() => {
          if (query.trim().toLowerCase() === trimmed) {
            setServerResults(null);
            setServerSearchFailed(true);
          }
        })
        .finally(() => {
          if (query.trim().toLowerCase() === trimmed) {
            setServerSearching(false);
          }
        });
    }, SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  const filteredConversations = conversations.filter((conversation) =>
    conversation.peerId.toLowerCase().includes(trimmedQuery) ||
    conversation.name.toLowerCase().includes(trimmedQuery) ||
    (conversation.displayName ?? "").toLowerCase().includes(trimmedQuery) ||
    (conversation.username ?? "").toLowerCase().includes(trimmedQuery)
  );

  /** The server-backed search is authoritative while it is active. */
  const serverSearchActive =
    trimmedQuery.length >= SEARCH_MIN_CHARS && !serverSearchFailed;

  const toggleMenu = () => {
    setMenuOpen((open) => !open);
    setConfirming(false);
  };

  const handleReset = () => {
    if (confirming) {
      setMenuOpen(false);
      setConfirming(false);
      onReset();
    } else {
      setConfirming(true);
    }
  };

  const handlePickResult = async (result: ProfileInfo) => {
    setSearchAddError(null);
    try {
      await onStartChat(result.peer_id);
      setQuery("");
      setServerResults(null);
    } catch (err) {
      setSearchAddError(String(err).replace(/^Error:\s*/, ""));
    }
  };

  return (
    <aside className="flex w-[350px] shrink-0 flex-col border-r border-wp-line/10 bg-wp-panel">
      {/* Profile */}
      <header className="flex items-center gap-3 px-4 pb-3 pt-4">
        <Avatar name={myDisplayName ?? undefined} size={40} />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold tracking-tight text-wp-text">
            {myDisplayName ?? "Your Whisper ID"}
          </p>
          <p className="truncate font-mono text-xs text-wp-dim">{peerId}</p>
        </div>
        <CopyButton value={peerId} />
        <button
          type="button"
          onClick={onOpenSettings}
          title="Settings"
          aria-label="Settings"
          className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-text active:scale-90"
        >
          <Settings className="h-4 w-4" />
        </button>
        <div className="relative">
          <button
            type="button"
            onClick={toggleMenu}
            title="Identity options"
            aria-label="Identity options"
            aria-haspopup="true"
            aria-expanded={menuOpen}
            className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-text active:scale-90"
          >
            <MoreVertical className="h-4 w-4" />
          </button>
          {menuOpen ? (
            <div className="absolute right-0 top-10 z-20 w-64 animate-pop-in rounded-xl border border-wp-line/10 bg-wp-panel-2 p-1.5 shadow-xl shadow-black/40">
              <p className="px-3 py-2 text-xs leading-relaxed text-wp-faint">
                Identity is stored locally in the app data folder. It never
                leaves this device.
              </p>
              <div className="my-1 h-px bg-wp-line/10" />
              <button
                type="button"
                onClick={handleReset}
                className={cx(
                  "flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-xs font-medium transition",
                  confirming
                    ? "bg-wp-danger/15 text-wp-danger"
                    : "text-wp-dim hover:bg-wp-panel-3 hover:text-wp-danger"
                )}
              >
                <Trash2 className="h-3.5 w-3.5" />
                {confirming ? "Click again to confirm" : "Reset identity"}
              </button>
            </div>
          ) : null}
        </div>
      </header>

      {/* Search */}
      <div className="px-4 pb-3">
        <label className="sr-only" htmlFor="search-conversations">
          Search by name, @username or Whisper ID
        </label>
        <div className="flex items-center gap-2 rounded-xl bg-wp-panel-2 px-3 py-2">
          {serverSearching ? (
            <Loader2 className="h-4 w-4 shrink-0 animate-spin text-wp-faint" />
          ) : (
            <Search className="h-4 w-4 shrink-0 text-wp-faint" />
          )}
          <input
            id="search-conversations"
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search by name, @username or ID"
            autoComplete="off"
            spellCheck={false}
            className="w-full bg-transparent text-sm text-wp-text placeholder-wp-faint outline-none"
          />
        </div>
        {searchAddError ? (
          <p
            role="alert"
            className="mt-2 text-xs leading-snug text-wp-danger"
          >
            {searchAddError}
          </p>
        ) : null}
      </div>

      {/* Section header */}
      <div className="flex items-center justify-between px-4 pb-2">
        <p className="text-xs font-semibold uppercase tracking-widest text-wp-faint">
          Conversations
        </p>
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={onNewGroup}
            title="New group"
            aria-label="New group"
            className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-text"
          >
            <Users className="h-4 w-4" />
          </button>
          <button
            type="button"
            onClick={onAddContact}
            title="Start a new chat"
            aria-label="Start a new chat"
            className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-text"
          >
            <SquarePen className="h-4 w-4" />
          </button>
        </div>
      </div>

      {/* Conversation list / directory results */}
      <div className="flex-1 overflow-y-auto px-2 pb-2">
        {serverSearchActive ? (
          serverResults === null ? (
            <div className="flex h-full items-center justify-center">
              <Loader2 className="h-5 w-5 animate-spin text-wp-faint" />
            </div>
          ) : serverResults.length === 0 ? (
            <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
              <div className="rounded-full bg-wp-panel-2 p-3 text-wp-faint">
                <SearchX className="h-5 w-5" />
              </div>
              <div>
                <p className="text-sm font-medium text-wp-dim">
                  No users found
                </p>
                <p className="mt-1 text-sm leading-relaxed text-wp-faint">
                  No registered usernames or IDs match your search.
                </p>
              </div>
            </div>
          ) : (
            <div>
              <p className="flex items-center gap-1.5 px-3 pb-2 pt-1 text-xs font-semibold uppercase tracking-widest text-wp-faint">
                <Users className="h-3.5 w-3.5" aria-hidden="true" />
                Search results
              </p>
              {serverResults.map((result) => (
                <button
                  key={result.peer_id}
                  type="button"
                  onClick={() => void handlePickResult(result)}
                  className="flex w-full items-center gap-3 rounded-xl px-3 py-3 text-left transition hover:bg-wp-panel-2"
                >
                  <Avatar
                    name={result.display_name ?? result.username ?? undefined}
                    size={44}
                    src={result.avatar_url ? mediaUrl(relayUrl, result.avatar_url) : null}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-baseline gap-2">
                      <p className="truncate text-sm font-medium text-wp-text">
                        {result.display_name ?? "Whisper user"}
                      </p>
                      {result.username ? (
                        <p className="truncate font-mono text-xs text-wp-faint">
                          @{result.username}
                        </p>
                      ) : null}
                    </div>
                    <p className="truncate font-mono text-xs text-wp-dim">
                      {shortPeerId(result.peer_id, 16)}
                    </p>
                  </div>
                  <UserPlus className="h-4 w-4 shrink-0 text-wp-accent" aria-hidden="true" />
                </button>
              ))}
            </div>
          )
        ) : conversations.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
            <div className="rounded-full bg-wp-panel-2 p-3 text-wp-faint">
              <MessageCirclePlus className="h-5 w-5" />
            </div>
            <div>
              <p className="text-sm font-medium text-wp-dim">
                No conversations yet
              </p>
              <p className="mt-1 text-sm leading-relaxed text-wp-faint">
                Start a chat with a friend by their Whisper ID.
              </p>
            </div>
            <button
              type="button"
              onClick={onAddContact}
              className="mt-1 inline-flex items-center gap-2 rounded-xl bg-wp-accent px-4 py-2 text-sm font-semibold text-wp-accent-fg transition hover:bg-wp-accent-strong active:scale-95"
            >
              <UserPlus className="h-3.5 w-3.5" />
              New Chat
            </button>
          </div>
        ) : filteredConversations.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
            <div className="rounded-full bg-wp-panel-2 p-3 text-wp-faint">
              <SearchX className="h-5 w-5" />
            </div>
            <div>
              <p className="text-sm font-medium text-wp-dim">
                No conversations found
              </p>
              <p className="mt-1 text-sm leading-relaxed text-wp-faint">
                No names or Whisper IDs match your search.
              </p>
            </div>
          </div>
        ) : (
          filteredConversations.map((conversation) => {
            const last = conversation.messages[conversation.messages.length - 1];
            const active = conversation.id === activeId;
            const isGroup = conversation.isGroup === true;
            const displayName = isGroup
              ? conversation.name
              : conversation.displayName ?? shortPeerId(conversation.peerId, 16);
            const online = presence[conversation.peerId]?.online === true;
            const avatarSrc = conversation.avatarUrl
              ? mediaUrl(relayUrl, conversation.avatarUrl)
              : null;
            return (
              <button
                key={conversation.id}
                type="button"
                onClick={() => onSelect(conversation.id)}
                onContextMenu={(event) => {
                  event.preventDefault();
                  setRowMenu({ x: event.clientX, y: event.clientY, conversation });
                }}
                aria-current={active ? "true" : undefined}
                className={cx(
                  "flex w-full items-center gap-3 rounded-xl px-3 py-3 text-left transition-colors duration-200 ease-out",
                  enteringIds.has(conversation.id) && "animate-conv-in",
                  active ? "bg-wp-panel-3" : "hover:bg-wp-panel-2"
                )}
              >
                <div className="relative shrink-0">
                  <Avatar
                    name={displayName}
                    size={44}
                    src={isGroup ? null : avatarSrc}
                    variant={isGroup ? "group" : "peer"}
                  />
                  {online && !isGroup ? (
                    <span
                      aria-hidden="true"
                      className="absolute bottom-0 right-0 h-3 w-3 animate-presence-pulse rounded-full border-2 border-wp-panel bg-wp-online"
                    />
                  ) : null}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center justify-between gap-2">
                    <div className="flex min-w-0 items-baseline gap-1.5">
                      <p className="truncate text-sm font-medium text-wp-text">
                        {displayName}
                      </p>
                      {conversation.username ? (
                        <p className="truncate font-mono text-xs text-wp-faint">
                          @{conversation.username}
                        </p>
                      ) : null}
                    </div>
                    {last ? (
                      <span className="shrink-0 text-xs tabular-nums text-wp-faint">
                        {formatTime(last.timestamp)}
                      </span>
                    ) : null}
                  </div>
                  <p className="truncate text-sm text-wp-dim">
                    {conversationPreview(conversation)}
                  </p>
                </div>
              </button>
            );
          })
        )}
      </div>

      {rowMenu ? (
        <ContextMenu
          x={rowMenu.x}
          y={rowMenu.y}
          label={`Actions for ${rowMenu.conversation.name}`}
          onClose={() => setRowMenu(null)}
          items={(() => {
            const conversation = rowMenu.conversation;
            const isGroup = conversation.isGroup === true;
            const items: ContextMenuItem[] = [
              {
                id: "view-profile",
                label: isGroup ? "View Group Info" : "View Profile",
                icon: isGroup ? (
                  <Users className="h-4 w-4" />
                ) : (
                  <UserRound className="h-4 w-4" />
                ),
                onSelect: () => {
                  if (isGroup) onOpenGroupInfo(conversation.peerId);
                  else onOpenProfile(conversation.peerId);
                },
              },
              {
                id: "send-message",
                label: "Send Message",
                icon: <MessageCircle className="h-4 w-4" />,
                onSelect: () => onSelect(conversation.id),
              },
              {
                id: "copy-peer-id",
                label: "Copy Peer ID",
                icon: <Copy className="h-4 w-4" />,
                onSelect: () => void copyText(conversation.peerId),
              },
            ];
            if (!isGroup) {
              items.push({
                id: "remove-contact",
                label: "Remove Contact",
                danger: true,
                icon: <UserX className="h-4 w-4" />,
                onSelect: () => onRemoveContact(conversation.peerId),
              });
            }
            return items;
          })()}
        />
      ) : null}

      {/* Connection status */}
      <footer className="border-t border-wp-line/10 px-4 py-3">
        {connected ? (
          <div className="flex items-center gap-2.5">
            <span className="relative flex h-2.5 w-2.5">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-wp-accent opacity-50" />
              <span className="relative inline-flex h-2.5 w-2.5 rounded-full bg-wp-accent" />
            </span>
            <span className="text-sm font-semibold tracking-wide text-wp-dim">
              Connected
            </span>
            <span className="text-xs text-wp-faint">
              · end-to-end encrypted
            </span>
          </div>
        ) : reconnecting ? (
          <div
            role="status"
            aria-live="polite"
            className="flex items-center gap-2.5 text-wp-dim"
          >
            <Loader2 className="h-4 w-4 animate-spin text-wp-accent" />
            <div className="min-w-0">
              <p className="text-sm font-semibold tracking-wide">
                Reconnecting…
              </p>
              {reconnectInfo ? (
                <p className="text-xs text-wp-faint">
                  Attempt {reconnectInfo.attempt} · retrying in{" "}
                  {Math.max(1, Math.round(reconnectInfo.nextInMs / 1000))}s
                </p>
              ) : null}
            </div>
          </div>
        ) : connecting ? (
          <div className="flex items-center gap-2.5 text-wp-dim">
            <Loader2 className="h-4 w-4 animate-spin text-wp-faint" />
            <span className="text-sm font-semibold tracking-wide">
              Connecting…
            </span>
          </div>
        ) : (
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2.5 text-wp-dim">
              <span className="h-2.5 w-2.5 rounded-full bg-wp-danger" />
              <span className="text-sm font-semibold tracking-wide">
                Disconnected
              </span>
            </div>
            <button
              type="button"
              onClick={onReconnect}
              className="rounded-lg bg-wp-panel-2 px-2.5 py-1.5 text-xs font-semibold text-wp-text transition hover:bg-wp-panel-3 active:scale-95"
            >
              Reconnect
            </button>
          </div>
        )}
        {connectionError ? (
          <p
            role="alert"
            className="mt-2 text-xs leading-snug text-wp-danger"
          >
            {connectionError}
          </p>
        ) : null}
      </footer>
    </aside>
  );
}
