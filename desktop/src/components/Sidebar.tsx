import { useState } from "react";
import {
  MessageCirclePlus,
  MoreVertical,
  Search,
  SquarePen,
  Trash2,
} from "lucide-react";
import type { Conversation } from "../types";
import { cx, formatTime, shortPeerId } from "../lib/format";
import { Avatar } from "./Avatar";
import { CopyButton } from "./CopyButton";

interface SidebarProps {
  peerId: string;
  conversations: Conversation[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onReset: () => void;
}

export function Sidebar({
  peerId,
  conversations,
  activeId,
  onSelect,
  onReset,
}: SidebarProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);

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
          <p className="text-sm font-semibold text-wp-text">Your Whisper ID</p>
          <p className="truncate font-mono text-xs text-wp-dim">{peerId}</p>
        </div>
        <CopyButton value={peerId} />
        <div className="relative">
          <button
            type="button"
            onClick={toggleMenu}
            title="Identity options"
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
        <div className="flex items-center gap-2 rounded-xl bg-wp-panel-2 px-3 py-2">
          <Search className="h-4 w-4 shrink-0 text-wp-faint" />
          <input
            type="text"
            placeholder="Search conversations"
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
          title="New chat"
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
              className="mt-1 rounded-xl bg-wp-accent px-4 py-2 text-xs font-semibold text-wp-deep transition hover:bg-wp-accent-strong"
            >
              New Chat
            </button>
          </div>
        ) : (
          conversations.map((conversation) => {
            const last = conversation.messages[conversation.messages.length - 1];
            const active = conversation.id === activeId;
            return (
              <button
                key={conversation.id}
                type="button"
                onClick={() => onSelect(conversation.id)}
                className={cx(
                  "flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition",
                  active ? "bg-wp-panel-3" : "hover:bg-wp-panel-2"
                )}
              >
                <Avatar name={conversation.name} size={42} />
                <div className="min-w-0 flex-1">
                  <div className="flex items-baseline justify-between gap-2">
                    <p className="truncate text-sm font-medium text-wp-text">
                      {conversation.name}
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
    </aside>
  );
}
