import { useEffect, useRef, useState } from "react";
import {
  Check,
  Copy,
  Hourglass,
  Link2,
  Loader2,
  LogOut,
  MessageCircle,
  MessageCirclePlus,
  MoreVertical,
  Pin,
  Search,
  SearchX,
  Settings,
  SquarePen,
  Trash2,
  UserPlus,
  UserRound,
  UserX,
  Users,
  X,
} from "lucide-react";
import type {
  Conversation,
  FriendRequestIncoming,
  GroupInviteInfo,
  PresenceInfo,
  ProfileInfo,
} from "../types";
import { cx, formatTime, mediaUrl, shortPeerId } from "../lib/format";
import { conversationPreview } from "../lib/chatList";
import { relayErrorCode, searchUsers, getInviteLink } from "../lib/relay";
import { copyText } from "../lib/clipboard";
import { useI18n } from "../i18n/I18nContext";
import { Avatar } from "./Avatar";
import { ContactsView } from "./ContactsView";
import { CopyButton } from "./CopyButton";
import { ContextMenu } from "./ContextMenu";
import type { ContextMenuItem } from "./ContextMenu";

interface SidebarProps {
  peerId: string;
  /** Our own public display name; null when unset. */
  myDisplayName: string | null;
  /** Our own avatar path ("/media/{hash}"); null when unset. */
  myAvatarUrl: string | null;
  conversations: Conversation[];
  /** Incoming friend requests (requester + display name), in arrival order. */
  friendRequestsIncoming: FriendRequestIncoming[];
  /** Outgoing pending friend requests: peer IDs we asked, unanswered. */
  friendRequestsOutgoing: string[];
  /** Pending group invites (accept/decline in the UI). */
  groupInvites: GroupInviteInfo[];
  /** Accept a pending group invite. */
  onAcceptGroupInvite: (groupId: string) => Promise<void>;
  /** Decline a pending group invite. */
  onDeclineGroupInvite: (groupId: string) => Promise<void>;
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
  /** Accept an incoming friend request. */
  onAcceptRequest: (peerId: string) => Promise<void>;
  /** Decline an incoming friend request. */
  onDeclineRequest: (peerId: string) => Promise<void>;
  onOpenSettings: () => void;
  onReconnect: () => void;
  onReset: () => void;
  /** Open the profile dialog for a specific 1:1 contact (context menu). */
  onOpenProfile: (peerId: string) => void;
  /** Open the group info panel for a specific group (context menu). */
  onOpenGroupInfo: (groupId: string) => void;
  /** Remove a 1:1 contact locally (context menu). */
  onRemoveContact: (peerId: string) => void;
  /** Leave a group (context menu, all members). */
  onLeaveGroup: (groupId: string) => void | Promise<void>;
  /** IDs of conversations pinned to the top (client-side). */
  pinnedIds: string[];
  /** Toggle whether a conversation is pinned. */
  onTogglePin: (peerId: string) => void;
  /** Unread incoming-message counts per peer. */
  unread: Record<string, number>;
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
  myAvatarUrl,
  conversations,
  friendRequestsIncoming,
  friendRequestsOutgoing,
  groupInvites,
  onAcceptGroupInvite,
  onDeclineGroupInvite,
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
  onAcceptRequest,
  onDeclineRequest,
  onOpenSettings,
  onReconnect,
  onReset,
  onOpenProfile,
  onOpenGroupInfo,
  onRemoveContact,
  onLeaveGroup,
  pinnedIds,
  onTogglePin,
  unread,
}: SidebarProps) {
  const { t } = useI18n();
  const [menuOpen, setMenuOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [inviteCopied, setInviteCopied] = useState(false);

  // The identity menu closes on any press outside it (or the ⋯ button).
  const menuWrapRef = useRef<HTMLDivElement>(null);
  const toggleRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!menuOpen) return;
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (
        menuWrapRef.current?.contains(target) ||
        toggleRef.current?.contains(target)
      ) {
        return;
      }
      setMenuOpen(false);
    };
    document.addEventListener("pointerdown", handlePointerDown);
    return () => document.removeEventListener("pointerdown", handlePointerDown);
  }, [menuOpen]);

  /** Copy our whisper:// invite link to the clipboard (best-effort). */
  const handleShareInvite = async () => {
    try {
      const link = await getInviteLink();
      const ok = await copyText(link);
      if (ok) {
        setInviteCopied(true);
        setMenuOpen(false);
        setTimeout(() => setInviteCopied(false), 1600);
      }
    } catch {
      // Best-effort: the peer ID copy button next to it always works.
    }
  };
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
  // Chats (recency-sorted conversation list) vs Contacts (all friends with
  // live Online / Last seen status and a remove button).
  const [view, setView] = useState<"chats" | "contacts">("chats");

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
      switch (relayErrorCode(err)) {
        case "already_contacts":
          setSearchAddError(t("contacts.already_contacts"));
          break;
        case "already_pending":
          setSearchAddError(t("contacts.already_pending"));
          break;
        case "cannot_add_self":
          setSearchAddError(t("contacts.cannot_add_self"));
          break;
        case "not_found":
          setSearchAddError(t("contacts.not_found"));
          break;
        case "rate_limited":
          setSearchAddError(t("contacts.rate_limited"));
          break;
        default:
          setSearchAddError(String(err).replace(/^Error:\s*/, ""));
      }
    }
  };

  return (
    <aside className="flex w-[350px] shrink-0 flex-col border-r border-wp-line/10 bg-wp-panel">
      {/* Profile */}
      <header className="flex items-center gap-3 px-4 pb-3 pt-4">
        <Avatar
          name={myDisplayName ?? undefined}
          size={40}
          src={myAvatarUrl ? mediaUrl(relayUrl, myAvatarUrl) : null}
        />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold tracking-tight text-wp-text">
            {myDisplayName ?? t("common.your_whisper_id")}
          </p>
          <p className="truncate font-mono text-xs text-wp-dim">{peerId}</p>
        </div>
        <CopyButton value={peerId} />
        <button
          type="button"
          onClick={onOpenSettings}
          title={t("common.settings")}
          aria-label={t("common.settings")}
          className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-text active:scale-90"
        >
          <Settings className="h-4 w-4" />
        </button>
        <div className="relative">
          <button
            ref={toggleRef}
            type="button"
            onClick={toggleMenu}
            title={t("sidebar.identity_options")}
            aria-label={t("sidebar.identity_options")}
            aria-haspopup="true"
            aria-expanded={menuOpen}
            className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-text active:scale-90"
          >
            <MoreVertical className="h-4 w-4" />
          </button>
          {menuOpen ? (
            <div
              ref={menuWrapRef}
              className="absolute right-0 top-10 z-20 w-64 animate-pop-in rounded-xl border border-wp-line/10 bg-wp-panel-2 p-1.5 shadow-xl shadow-black/40"
            >
              <p className="px-3 py-2 text-xs leading-relaxed text-wp-faint">
                {t("sidebar.identity_local_note")}
              </p>
              <div className="my-1 h-px bg-wp-line/10" />
              <button
                type="button"
                onClick={() => void handleShareInvite()}
                className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-xs font-medium text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
              >
                {inviteCopied ? (
                  <Check className="h-3.5 w-3.5 text-wp-online" />
                ) : (
                  <Link2 className="h-3.5 w-3.5" />
                )}
                {inviteCopied ? t("common.copied") : t("common.share_invite")}
              </button>
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
                {confirming ? t("common.confirm_again") : t("common.reset_identity")}
              </button>
            </div>
          ) : null}
        </div>
      </header>

      {/* Search */}
      <div className="px-4 pb-3">
        <label className="sr-only" htmlFor="search-conversations">
          {t("sidebar.search_label")}
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
            placeholder={t("sidebar.search_placeholder")}
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

      {/* Chats / Contacts view switch */}
      <div className="flex gap-1 px-4 pb-2">
        <button
          type="button"
          onClick={() => setView("chats")}
          aria-pressed={view === "chats"}
          className={cx(
            "flex-1 rounded-lg px-3 py-1.5 text-sm font-semibold transition",
            view === "chats"
              ? "bg-wp-accent text-wp-accent-fg shadow-sm"
              : "text-wp-dim hover:bg-wp-panel-3 hover:text-wp-text"
          )}
        >
          {t("sidebar.tab_chats")}
        </button>
        <button
          type="button"
          onClick={() => setView("contacts")}
          aria-pressed={view === "contacts"}
          className={cx(
            "flex-1 rounded-lg px-3 py-1.5 text-sm font-semibold transition",
            view === "contacts"
              ? "bg-wp-accent text-wp-accent-fg shadow-sm"
              : "text-wp-dim hover:bg-wp-panel-3 hover:text-wp-text"
          )}
        >
          {t("sidebar.tab_contacts")}
        </button>
      </div>

      {/* Section header */}
      <div className="flex items-center justify-between px-4 pb-2">
        <p className="text-xs font-semibold uppercase tracking-widest text-wp-faint">
          {t("sidebar.conversations")}
        </p>
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={onNewGroup}
            title={t("common.new_group")}
            aria-label={t("common.new_group")}
            className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-text"
          >
            <Users className="h-4 w-4" />
          </button>
          <button
            type="button"
            onClick={onAddContact}
            title={t("sidebar.start_new_chat")}
            aria-label={t("sidebar.start_new_chat")}
            className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-text"
          >
            <SquarePen className="h-4 w-4" />
          </button>
        </div>
      </div>

      {/* Pending friend requests: incoming (accept/decline) + outgoing
          (waiting on the other side). A pending peer is not chatable — they
          only appear here until the request is accepted. */}
      {friendRequestsIncoming.length > 0 || friendRequestsOutgoing.length > 0 ? (
        <div className="border-b border-wp-line/10 px-2 pb-2">
          <p className="px-3 pb-1.5 text-xs font-semibold uppercase tracking-widest text-wp-accent">
            {t("contacts.title")}
          </p>
          <div className="flex flex-col gap-0.5" aria-label={t("contacts.title")}>
            {friendRequestsIncoming.map((request) => {
              const name = request.display_name ?? shortPeerId(request.peer_id, 16);
              return (
                <div
                  key={request.peer_id}
                  className="flex items-center gap-2.5 rounded-xl px-3 py-2 transition hover:bg-wp-panel-2"
                >
                  <Avatar name={request.display_name ?? undefined} size={36} />
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium text-wp-text">
                      {name}
                    </p>
                    <p className="truncate font-mono text-xs text-wp-faint">
                      {shortPeerId(request.peer_id, 16)}
                    </p>
                  </div>
                  <div className="flex shrink-0 items-center gap-0.5">
                    <button
                      type="button"
                      onClick={() => void onAcceptRequest(request.peer_id)}
                      title={t("contacts.accept")}
                      aria-label={t("contacts.accept_aria", { name })}
                      className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-online"
                    >
                      <Check className="h-4 w-4" />
                    </button>
                    <button
                      type="button"
                      onClick={() => void onDeclineRequest(request.peer_id)}
                      title={t("contacts.decline")}
                      aria-label={t("contacts.decline_aria", { name })}
                      className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-danger"
                    >
                      <X className="h-4 w-4" />
                    </button>
                  </div>
                </div>
              );
            })}
            {friendRequestsOutgoing.map((peerId) => {
              const name = shortPeerId(peerId, 16);
              return (
                <div
                  key={peerId}
                  className="flex items-center gap-2.5 rounded-xl px-3 py-2"
                >
                  <Avatar name={undefined} size={36} />
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium text-wp-dim">
                      {name}
                    </p>
                    <p className="truncate font-mono text-xs text-wp-faint">
                      {shortPeerId(peerId, 16)}
                    </p>
                  </div>
                  <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-wp-panel-3 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-wp-dim">
                    <Hourglass className="h-3 w-3" aria-hidden="true" />
                    {t("contacts.pending")}
                  </span>
                </div>
              );
            })}
          </div>
        </div>
      ) : null}

      {/* Pending group invites: accept/decline right here. */}
      {groupInvites.length > 0 ? (
        <div className="border-b border-wp-line/10 px-2 pb-2">
          <p className="px-3 pb-1.5 text-xs font-semibold uppercase tracking-widest text-wp-accent">
            {t("invites.title")}
          </p>
          <div className="flex flex-col gap-0.5" aria-label={t("invites.title")}>
            {groupInvites.map((invite) => (
              <div
                key={invite.group_id}
                className="flex items-center gap-2.5 rounded-xl px-3 py-2 transition hover:bg-wp-panel-2"
              >
                <Avatar name={invite.group_name} size={36} variant="group" />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium text-wp-text">
                    {invite.group_name}
                  </p>
                  <p className="truncate text-xs text-wp-faint">
                    {t("invites.from", { peer: shortPeerId(invite.inviter_peer_id, 16) })}
                  </p>
                </div>
                <div className="flex shrink-0 items-center gap-0.5">
                  <button
                    type="button"
                    onClick={() => void onAcceptGroupInvite(invite.group_id)}
                    title={t("invites.accept")}
                    aria-label={t("invites.accept_aria", { group: invite.group_name })}
                    className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-online"
                  >
                    <Check className="h-4 w-4" />
                  </button>
                  <button
                    type="button"
                    onClick={() => void onDeclineGroupInvite(invite.group_id)}
                    title={t("invites.decline")}
                    aria-label={t("invites.decline_aria", { group: invite.group_name })}
                    className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-danger"
                  >
                    <X className="h-4 w-4" />
                  </button>
                </div>
              </div>
            ))}
          </div>
        </div>
      ) : null}

      {/* Contacts view: all friends with live Online/Last seen + remove */}
      {view === "contacts" ? (
        <div className="flex-1 overflow-y-auto px-2 pb-2">
          <ContactsView
            contacts={conversations.filter((c) => c.isGroup !== true)}
            presence={presence}
            relayUrl={relayUrl}
            activeId={activeId}
            onSelect={onSelect}
            onRemoveContact={onRemoveContact}
          />
        </div>
      ) : (
      /* Conversation list / directory results */
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
                  {t("sidebar.no_users_found")}
                </p>
                <p className="mt-1 text-sm leading-relaxed text-wp-faint">
                  {t("sidebar.no_users_found_hint")}
                </p>
              </div>
            </div>
          ) : (
            <div>
              <p className="flex items-center gap-1.5 px-3 pb-2 pt-1 text-xs font-semibold uppercase tracking-widest text-wp-faint">
                <Users className="h-3.5 w-3.5" aria-hidden="true" />
                {t("sidebar.search_results")}
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
                        {result.display_name ?? t("sidebar.whisper_user")}
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
                {t("sidebar.no_conversations")}
              </p>
              <p className="mt-1 text-sm leading-relaxed text-wp-faint">
                {t("sidebar.no_conversations_hint")}
              </p>
            </div>
            <button
              type="button"
              onClick={onAddContact}
              className="mt-1 inline-flex items-center gap-2 rounded-xl bg-wp-accent px-4 py-2 text-sm font-semibold text-wp-accent-fg transition hover:bg-wp-accent-strong active:scale-95"
            >
              <UserPlus className="h-3.5 w-3.5" />
              {t("sidebar.new_chat")}
            </button>
          </div>
        ) : filteredConversations.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
            <div className="rounded-full bg-wp-panel-2 p-3 text-wp-faint">
              <SearchX className="h-5 w-5" />
            </div>
            <div>
              <p className="text-sm font-medium text-wp-dim">
                {t("sidebar.no_conversations_found")}
              </p>
              <p className="mt-1 text-sm leading-relaxed text-wp-faint">
                {t("sidebar.no_conversations_found_hint")}
              </p>
            </div>
          </div>
        ) : (
          filteredConversations.map((conversation) => {
            const last = conversation.messages[conversation.messages.length - 1];
            const active = conversation.id === activeId;
            const isGroup = conversation.isGroup === true;
            const isPinned = pinnedIds.includes(conversation.peerId);
            const unreadCount = unread[conversation.peerId] ?? 0;
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
                    src={avatarSrc}
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
                    <div className="flex shrink-0 flex-col items-end gap-1">
                      <div className="flex items-center gap-1.5">
                        {isPinned ? (
                          <Pin
                            className="h-3 w-3 text-wp-accent"
                            aria-label={t("sidebar.pinned")}
                          />
                        ) : null}
                        {last ? (
                          <span className="text-xs tabular-nums text-wp-faint">
                            {formatTime(last.timestamp)}
                          </span>
                        ) : null}
                      </div>
                      {unreadCount > 0 ? (
                        <span
                          aria-label={t("sidebar.unread_messages", {
                            n: unreadCount,
                          })}
                          className="animate-pop-in inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-full bg-wp-accent px-1 text-[10px] font-bold leading-none text-wp-accent-fg tabular-nums"
                        >
                          {unreadCount > 99 ? "99+" : unreadCount}
                        </span>
                      ) : null}
                    </div>
                  </div>
                  <p
                    className={cx(
                      "truncate text-sm",
                      unreadCount > 0 ? "font-medium text-wp-text" : "text-wp-dim"
                    )}
                  >
                    {conversationPreview(conversation, t)}
                  </p>
                </div>
              </button>
            );
          })
        )}
      </div>
      )}

      {rowMenu ? (
        <ContextMenu
          x={rowMenu.x}
          y={rowMenu.y}
          label={t("sidebar.actions_for", { name: rowMenu.conversation.name })}
          onClose={() => setRowMenu(null)}
          items={(() => {
            const conversation = rowMenu.conversation;
            const isGroup = conversation.isGroup === true;
            const isPinned = pinnedIds.includes(conversation.peerId);
            const items: ContextMenuItem[] = [
              {
                id: "view-profile",
                label: isGroup ? t("sidebar.view_group_info") : t("sidebar.view_profile"),
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
                label: t("common.send_message"),
                icon: <MessageCircle className="h-4 w-4" />,
                onSelect: () => onSelect(conversation.id),
              },
              {
                id: "copy-peer-id",
                label: t("sidebar.copy_peer_id"),
                icon: <Copy className="h-4 w-4" />,
                onSelect: () => void copyText(conversation.peerId),
              },
              {
                id: isPinned ? "unpin-chat" : "pin-chat",
                label: isPinned
                  ? t("sidebar.unpin_chat")
                  : t("sidebar.pin_chat"),
                icon: <Pin className="h-4 w-4" />,
                onSelect: () => onTogglePin(conversation.peerId),
              },
            ];
            if (!isGroup) {
              items.push({
                id: "remove-contact",
                label: t("common.remove_contact"),
                danger: true,
                icon: <UserX className="h-4 w-4" />,
                onSelect: () => onRemoveContact(conversation.peerId),
              });
            } else {
              items.push({
                id: "leave-group",
                label: t("groupInfo.leave_group"),
                danger: true,
                icon: <LogOut className="h-4 w-4" />,
                onSelect: () => void onLeaveGroup(conversation.peerId),
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
              {t("sidebar.connected")}
            </span>
            <span className="text-xs text-wp-faint">
              {t("sidebar.e2ee_suffix")}
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
                {t("sidebar.reconnecting")}
              </p>
              {reconnectInfo ? (
                <p className="text-xs text-wp-faint">
                  {t("sidebar.reconnect_attempt", {
                    attempt: reconnectInfo.attempt,
                    seconds: Math.max(1, Math.round(reconnectInfo.nextInMs / 1000)),
                  })}
                </p>
              ) : null}
            </div>
          </div>
        ) : connecting ? (
          <div className="flex items-center gap-2.5 text-wp-dim">
            <Loader2 className="h-4 w-4 animate-spin text-wp-faint" />
            <span className="text-sm font-semibold tracking-wide">
              {t("sidebar.connecting")}
            </span>
          </div>
        ) : (
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2.5 text-wp-dim">
              <span className="h-2.5 w-2.5 rounded-full bg-wp-danger" />
              <span className="text-sm font-semibold tracking-wide">
                {t("sidebar.disconnected")}
              </span>
            </div>
            <button
              type="button"
              onClick={onReconnect}
              className="rounded-lg bg-wp-panel-2 px-2.5 py-1.5 text-xs font-semibold text-wp-text transition hover:bg-wp-panel-3 active:scale-95"
            >
              {t("sidebar.reconnect")}
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
