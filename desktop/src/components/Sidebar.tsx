import { useState } from "react";
import {
  Loader2,
  MessageCirclePlus,
  MoreVertical,
  Search,
  SearchX,
  Settings,
  SquarePen,
  Trash2,
  UserPlus,
} from "lucide-react";
import type { Conversation } from "../types";
import { cx, formatTime, shortPeerId } from "../lib/format";
import { Avatar } from "./Avatar";
import { CopyButton } from "./CopyButton";

interface SidebarProps {
  peerId: string;
  conversations: Conversation[];
  activeId: string | null;
  connected: boolean;
  connecting: boolean;
  connectionError: string | null;
  onSelect: (id: string) => void;
  onAddContact: () => void;
  onOpenSettings: () => void;
  onReconnect: () => void;
  onReset: () => void;
}

export function Sidebar({
  peerId,
  conversations,
  activeId,
  connected,
  connecting,
  connectionError,
  onSelect,
  onAddContact,
  onOpenSettings,
  onReconnect,
  onReset,
}: SidebarProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [query, setQuery] = useState("");

  const trimmedQuery = query.trim().toLowerCase();
  const filteredConversations = conversations.filter((conversation) =>
    conversation.peerId.toLowerCase().includes(trimmedQuery)
  );

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

  return (
    <aside className="flex w-[350px] shrink-0 flex-col border-r border-wp-line/10 bg-wp-panel">
      {/* Profile */}
      <header className="flex items-center gap-3 px-4 pb-3 pt-4">
        <Avatar size={40} />
        <div className="min-w-0 flex-1">
          <p className="font-display text-sm font-semibold tracking-tight text-wp-text">
            Your Whisper ID
          </p>
          <p className="truncate font-mono text-xs text-wp-dim">{peerId}</p>
        </div>
        <CopyButton value={peerId} />
        <button
          type="button"
          onClick={onOpenSettings}
          title="Settings"
          aria-label="Settings"
          className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-text"
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
            className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-text"
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
          Search conversations
        </label>
        <div className="flex items-center gap-2 rounded-xl bg-wp-panel-2 px-3 py-2">
          <Search className="h-4 w-4 shrink-0 text-wp-faint" />
          <input
            id="search-conversations"
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search by Whisper ID"
            autoComplete="off"
            spellCheck={false}
            className="w-full bg-transparent text-sm text-wp-text placeholder-wp-faint outline-none"
          />
        </div>
      </div>

      {/* Section header */}
      <div className="flex items-center justify-between px-4 pb-2">
        <p className="text-xs font-semibold uppercase tracking-widest text-wp-faint">
          Conversations
        </p>
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

      {/* Conversation list */}
      <div className="flex-1 overflow-y-auto px-2 pb-2">
        {conversations.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
            <div className="rounded-full bg-wp-panel-2 p-3 text-wp-faint">
              <MessageCirclePlus className="h-5 w-5" />
            </div>
            <div>
              <p className="text-sm font-medium text-wp-dim">
                No conversations yet
              </p>
              <p className="mt-1 text-xs leading-relaxed text-wp-faint">
                Start a chat with a friend by their Whisper ID.
              </p>
            </div>
            <button
              type="button"
              onClick={onAddContact}
              className="mt-1 inline-flex items-center gap-2 rounded-xl bg-wp-accent px-4 py-2 text-xs font-semibold text-wp-accent-fg transition hover:bg-wp-accent-strong"
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
              <p className="mt-1 text-xs leading-relaxed text-wp-faint">
                No Whisper IDs match your search.
              </p>
            </div>
          </div>
        ) : (
          filteredConversations.map((conversation) => {
            const last = conversation.messages[conversation.messages.length - 1];
            const active = conversation.id === activeId;
            return (
              <button
                key={conversation.id}
                type="button"
                onClick={() => onSelect(conversation.id)}
                aria-current={active ? "true" : undefined}
                className={cx(
                  "flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition",
                  active ? "bg-wp-panel-3" : "hover:bg-wp-panel-2"
                )}
              >
                <Avatar name={shortPeerId(conversation.peerId)} size={42} />
                <div className="min-w-0 flex-1">
                  <div className="flex items-baseline justify-between gap-2">
                    <p className="truncate font-mono text-sm font-medium text-wp-text">
                      {shortPeerId(conversation.peerId, 16)}
                    </p>
                    {last ? (
                      <span className="shrink-0 text-[10px] tabular-nums text-wp-faint">
                        {formatTime(last.timestamp)}
                      </span>
                    ) : null}
                  </div>
                  <p className="truncate text-xs text-wp-dim">
                    {last
                      ? `${last.outgoing ? "You: " : ""}${last.text}`
                      : shortPeerId(conversation.peerId)}
                  </p>
                </div>
              </button>
            );
          })
        )}
      </div>

      {/* Connection status */}
      <footer className="border-t border-wp-line/10 px-4 py-3">
        {connected ? (
          <div className="flex items-center gap-2.5">
            <span className="relative flex h-2.5 w-2.5">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-wp-accent opacity-50" />
              <span className="relative inline-flex h-2.5 w-2.5 rounded-full bg-wp-accent" />
            </span>
            <span className="text-xs font-semibold tracking-wide text-wp-dim">
              Connected
            </span>
            <span className="text-[10px] text-wp-faint">
              · end-to-end encrypted
            </span>
          </div>
        ) : connecting ? (
          <div className="flex items-center gap-2.5 text-wp-dim">
            <Loader2 className="h-4 w-4 animate-spin text-wp-faint" />
            <span className="text-xs font-semibold tracking-wide">
              Connecting…
            </span>
          </div>
        ) : (
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2.5 text-wp-dim">
              <span className="h-2.5 w-2.5 rounded-full bg-wp-danger" />
              <span className="text-xs font-semibold tracking-wide">
                Disconnected
              </span>
            </div>
            <button
              type="button"
              onClick={onReconnect}
              className="rounded-lg bg-wp-panel-2 px-2.5 py-1.5 text-xs font-semibold text-wp-text transition hover:bg-wp-panel-3"
            >
              Reconnect
            </button>
          </div>
        )}
        {connectionError ? (
          <p
            role="alert"
            className="mt-2 text-[11px] leading-snug text-wp-danger"
          >
            {connectionError}
          </p>
        ) : null}
      </footer>
    </aside>
  );
}
