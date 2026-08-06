import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { QuoteInfo, Theme } from "../types";
import {
  addGroupMember,
  clearChatHistory,
  createGroup,
  demoteMember,
  exportIdentity,
  getGroupInfo,
  getPendingDeepLink,
  getSettings,
  importIdentity,
  onDeepLink,
  promoteMember,
  registerProfile,
  reloadIdentity,
  removeMember,
  resetRelay,
  setAutostart,
  setAvatar,
  setGroupAvatar,
  setPrivacy,
  setTheme as persistTheme,
  updateSettings,
} from "../lib/relay";
import { buildConversations } from "../lib/chatList";
import { shortPeerId } from "../lib/format";
import { loadPinnedChats, persistPinnedChats } from "../lib/pinned";
import { useI18n } from "../i18n/I18nContext";
import { useChatState } from "../hooks/useChatState";
import { useOwnProfile } from "../hooks/useOwnProfile";
import { usePresencePolling } from "../hooks/usePresencePolling";
import { useToast } from "../hooks/useToast";
import { Sidebar } from "./Sidebar";
import { ChatView } from "./ChatView";
import { AddContactDialog } from "./AddContactDialog";
import { GroupInfoDialog } from "./GroupInfoDialog";
import { NewGroupDialog } from "./NewGroupDialog";
import { ProfileDialog } from "./ProfileDialog";
import { SettingsDialog } from "./SettingsDialog";

interface MainViewProps {
  peerId: string;
  onReset: () => void;
}

/** Extract the target peer ID from a whisper:// deep link (invite or verify).
 *  Returns null for anything that is not a well-formed link. */
function extractPeerIdFromLink(url: string): string | null {
  const match = url.match(
    /^whisper:\/\/(?:invite|verify)\?[^]*\bpeer=([0-9a-f]{24})\b/i
  );
  return match ? match[1] : null;
}

export function MainView({ peerId, onReset }: MainViewProps) {
  const { t } = useI18n();
  const { toast } = useToast();
  const [theme, setTheme] = useState<Theme>("dark");
  const [relayUrl, setRelayUrl] = useState("");
  // Privacy / notification preferences, hydrated from the settings store on
  // mount and persisted on change.
  const [presenceVisible, setPresenceVisible] = useState(true);
  const [readReceipts, setReadReceipts] = useState(true);
  const [typingIndicator, setTypingIndicator] = useState(true);
  const [notificationsEnabled, setNotificationsEnabled] = useState(true);
  const [notificationPreview, setNotificationPreview] = useState(true);
  const [notificationSound, setNotificationSound] = useState(true);
  // Behavioral preferences (General tab): minimize-to-tray, autostart,
  // Enter-to-send and the message font scale.
  const [minimizeToTray, setMinimizeToTray] = useState(false);
  const [enterToSend, setEnterToSend] = useState(true);
  const [messageFontScale, setMessageFontScale] = useState("normal");
  const [autostart, setAutostartState] = useState(false);
  // Dialog open state for the various overlay panels.
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [newGroupOpen, setNewGroupOpen] = useState(false);
  const [groupInfoGroupId, setGroupInfoGroupId] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  // Pre-fill for the Add-contact dialog: set when a whisper:// deep link or
  // invite paste opens it, so the target peer is already in the field.
  const [inviteValue, setInviteValue] = useState("");
  // Peer whose profile dialog is open; null when closed.
  const [profilePeerId, setProfilePeerId] = useState<string | null>(null);
  // Pinned conversations (client-side, persisted per identity in localStorage).
  const [pinnedIds, setPinnedIds] = useState<string[]>(() =>
    loadPinnedChats(peerId)
  );
  // In-memory drafts per conversation (keyed by peer/group id). Only the
  // composer text lives here; drafts are never persisted to the store.
  const [drafts, setDrafts] = useState<Record<string, string>>({});

  const chat = useChatState({
    notificationsEnabled,
    notificationPreview,
    notificationSound,
    t,
  });
  const { myProfile, refreshOwnProfile } = useOwnProfile(peerId, chat.connected);
  // The peer we last sent an "is typing" signal to; null when idle. Tracked so
  // switching conversations can emit a typing-stop to exactly that peer.
  const typingPeerRef = useRef<string | null>(null);
  // Previous active peer, so the switch effect below can tell when it changed.
  const prevActivePeerRef = useRef<string | null>(chat.activePeerId);

  // Real-time presence pushes come through the `presence` event (registered in
  // useChatState); the poll re-seeds the active peer and covers reconnects.
  usePresencePolling({
    activePeerId: chat.activePeerId,
    connected: chat.connected,
    onPresence: chat.updatePresence,
  });

  // Keep the DOM attribute in sync with the active theme. The stylesheet
  // defines a light variant under `[data-theme="light"]`; anything else
  // falls back to the default dark palette.
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  // Message bubble font scale is applied the same way: a data attribute on the
  // <html> element that `styles.css` uses to resize `.wp-msg`.
  useEffect(() => {
    document.documentElement.dataset.messageScale = messageFontScale;
  }, [messageFontScale]);

  // Deep links: a whisper:// invite that launched the app (drained from the
  // pending queue) or arrived while running (live event) pre-fills the
  // Add-contact dialog so the target peer is one click away.
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const openInvite = (url: string) => {
      if (disposed || !extractPeerIdFromLink(url)) return;
      setInviteValue(url);
      setAddDialogOpen(true);
    };
    void getPendingDeepLink().then((links) => {
      if (disposed) return;
      for (const link of links) openInvite(link);
    });
    void onDeepLink((url) => openInvite(url)).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // Load persisted settings (relay URL + theme) once on mount so the UI
  // reflects the user's saved choices immediately.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const settings = await getSettings();
        if (cancelled) return;
        if (settings.theme === "dark" || settings.theme === "light") {
          setTheme(settings.theme);
        }
        if (settings.relay_url) {
          setRelayUrl(settings.relay_url);
        } else {
          // The relay URL is hardcoded in the client; keep the state in sync
          // even before the first connect persists the effective endpoint, so
          // `/media/{hash}` avatar paths always resolve.
          setRelayUrl("ws://127.0.0.1:8080/ws");
        }
        if (settings.presence_visible != null) setPresenceVisible(settings.presence_visible);
        setReadReceipts(settings.read_receipts ?? true);
        setTypingIndicator(settings.typing_indicator ?? true);
        setNotificationsEnabled(settings.notifications_enabled ?? true);
        setNotificationPreview(settings.notification_preview ?? true);
        setNotificationSound(settings.notification_sound ?? true);
        if (settings.minimize_to_tray != null) setMinimizeToTray(settings.minimize_to_tray);
        setEnterToSend(settings.enter_to_send ?? true);
        const scale = settings.message_font_scale;
        if (scale === "small" || scale === "normal" || scale === "large") {
          setMessageFontScale(scale);
        }
        if (settings.autostart != null) setAutostartState(settings.autostart);
      } catch {
        // Settings are best-effort; the defaults (dark, default relay) apply.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Once the relay connects, the Rust side has persisted the *effective*
  // endpoint (settings → env var → default). Re-reading settings then makes
  // the relay URL state reflect the URL the client actually talks to, so
  // `/media/{hash}` avatar paths resolve to the right origin.
  useEffect(() => {
    if (!chat.connected) return;
    let cancelled = false;
    void (async () => {
      try {
        const settings = await getSettings();
        if (cancelled) return;
        if (settings.relay_url) {
          setRelayUrl(settings.relay_url);
        } else {
          setRelayUrl("ws://127.0.0.1:8080/ws");
        }
      } catch {
        // Best-effort: the initial mount already loaded the settings.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [chat.connected]);

  const handleThemeChange = useCallback((next: Theme) => {
    setTheme(next);
    void persistTheme(next).catch(() => {
      // The theme is applied in memory immediately; persistence only affects
      // the next launch, so a failure here is non-fatal.
    });
  }, []);

  const handleSend = useCallback(
    (text: string, quote?: QuoteInfo | null) => {
      const active = chat.activePeerId;
      if (!active) return;
      void chat.sendMessage(active, text, quote);
      // Sending ends the draft for this conversation and the typing state.
      setDrafts((prev) =>
        prev[active] !== undefined ? { ...prev, [active]: "" } : prev
      );
      if (typingPeerRef.current === active) typingPeerRef.current = null;
    },
    [chat.activePeerId, chat.sendMessage]
  );

  const handleTypingChange = useCallback(
    (isTyping: boolean) => {
      const active = chat.activePeerId;
      if (!active) return;
      // Remember who we are signalling so a conversation switch can stop it.
      if (isTyping) typingPeerRef.current = active;
      else if (typingPeerRef.current === active) {
        typingPeerRef.current = null;
      }
      chat.sendTyping(active, isTyping);
    },
    [chat.activePeerId, chat.sendTyping]
  );

  const handleDraftChange = useCallback(
    (text: string) => {
      const active = chat.activePeerId;
      if (!active) return;
      setDrafts((prev) =>
        prev[active] === text ? prev : { ...prev, [active]: text }
      );
    },
    [chat.activePeerId]
  );

  // Leaving a conversation while typing (or with a non-empty draft) must send
  // a typing-stop to the peer we were writing to — otherwise the old
  // conversation shows "typing…" forever because the composer's stop timer now
  // fires against the newly active peer.
  useEffect(() => {
    const previous = prevActivePeerRef.current;
    prevActivePeerRef.current = chat.activePeerId;
    if (!previous || previous === chat.activePeerId) return;
    const wasTyping = typingPeerRef.current === previous;
    const hadDraft = (drafts[previous] ?? "").trim() !== "";
    if (wasTyping || hadDraft) {
      if (typingPeerRef.current === previous) typingPeerRef.current = null;
      chat.sendTyping(previous, false);
    }
  }, [chat.activePeerId, chat.sendTyping, drafts]);

  const handleRegisterUsername = useCallback(
    async (username: string) => {
      await registerProfile(username);
      // Re-fetch the profile so the UI picks up the new username.
      await refreshOwnProfile();
      toast(t("toast.username_registered"), "success");
    },
    [refreshOwnProfile, toast, t]
  );

  const handleSetAvatar = useCallback(
    async (avatarBase64: string) => {
      const username = myProfile?.username;
      if (!username) {
        throw new Error(t("general.register_username_first"));
      }
      await setAvatar(username, avatarBase64);
      // Re-fetch the profile so the avatar_url (and preview) refresh, and
      // resync the chat-state snapshot so the sidebar header avatar follows.
      await Promise.all([refreshOwnProfile(), chat.refresh()]);
      toast(t("toast.avatar_updated"), "success");
    },
    [myProfile?.username, refreshOwnProfile, chat.refresh, toast, t]
  );

  // Privacy / notification preference handlers: apply in memory immediately
  // and persist best-effort (a store failure must never block the toggle).
  const handlePresenceVisibleChange = useCallback((value: boolean) => {
    setPresenceVisible(value);
    void setPrivacy(value).catch(() => {});
  }, []);

  const handleReadReceiptsChange = useCallback((value: boolean) => {
    setReadReceipts(value);
    void updateSettings({ read_receipts: value }).catch(() => {});
  }, []);

  const handleTypingIndicatorChange = useCallback((value: boolean) => {
    setTypingIndicator(value);
    void updateSettings({ typing_indicator: value }).catch(() => {});
  }, []);

  const handleNotificationsEnabledChange = useCallback((value: boolean) => {
    setNotificationsEnabled(value);
    void updateSettings({ notifications_enabled: value }).catch(() => {});
  }, []);

  const handleNotificationPreviewChange = useCallback((value: boolean) => {
    setNotificationPreview(value);
    void updateSettings({ notification_preview: value }).catch(() => {});
  }, []);

  const handleNotificationSoundChange = useCallback((value: boolean) => {
    setNotificationSound(value);
    void updateSettings({ notification_sound: value }).catch(() => {});
  }, []);

  const handleMinimizeToTrayChange = useCallback((value: boolean) => {
    setMinimizeToTray(value);
    void updateSettings({ minimize_to_tray: value }).catch(() => {});
  }, []);

  const handleEnterToSendChange = useCallback((value: boolean) => {
    setEnterToSend(value);
    void updateSettings({ enter_to_send: value }).catch(() => {});
  }, []);

  const handleMessageFontScaleChange = useCallback((value: string) => {
    setMessageFontScale(value);
    void updateSettings({ message_font_scale: value }).catch(() => {});
  }, []);

  /** Autostart registers the app in the OS. On failure the toggle (and the
   *  persisted preference) is reverted so the UI never claims a registration
   *  that is not really in place. */
  const handleAutostartChange = useCallback(
    (value: boolean) => {
      setAutostartState(value);
      void setAutostart(value)
        .then(() => {
          void updateSettings({ autostart: value }).catch(() => {});
          toast(
            value ? t("toast.autostart_enabled") : t("toast.autostart_disabled"),
            "success"
          );
        })
        .catch((err) => {
          setAutostartState(!value);
          void updateSettings({ autostart: !value }).catch(() => {});
          const message = String(err).replace(/^Error:\s*/, "");
          toast(message, "error");
        });
    },
    [toast, t]
  );

  /** Backup the identity file through a native save dialog. */
  const handleExportIdentity = useCallback(async () => {
    try {
      await exportIdentity();
      toast(t("toast.identity_exported"), "success");
    } catch (err) {
      const message = String(err).replace(/^Error:\s*/, "");
      toast(message, "error");
    }
  }, [toast, t]);

  /** Import a previously backed-up identity file. After the file is in place
   *  the cached identity is dropped and the webview reloads, so the restored
   *  identity takes effect without a full app restart. */
  const handleImportIdentity = useCallback(async () => {
    try {
      await importIdentity();
      await reloadIdentity();
      toast(t("toast.identity_imported"), "success");
      toast(t("toast.identity_import_restart"), "info");
      // Give the toasts a moment to render before the reload clears them.
      window.setTimeout(() => window.location.reload(), 1500);
    } catch (err) {
      const message = String(err).replace(/^Error:\s*/, "");
      toast(message, "error");
    }
  }, [toast, t]);

  /** Clear the whole message history on this device, then drop every message
   *  and unread badge from the React state so the UI updates instantly. */
  const handleClearHistory = useCallback(async () => {
    try {
      await clearChatHistory();
      chat.clearHistory();
      toast(t("toast.history_cleared"), "success");
    } catch (err) {
      const message = String(err).replace(/^Error:\s*/, "");
      toast(message, "error");
    }
  }, [chat.clearHistory, toast, t]);

  // Profile dialog wiring: opening focuses the active conversation's peer;
  // "Message" just closes the dialog (the chat is already open).
  const handleOpenProfile = useCallback(() => {
    if (chat.activePeerId) setProfilePeerId(chat.activePeerId);
  }, [chat.activePeerId]);

  /** Open the profile dialog for a specific peer (context-menu action). */
  const handleOpenProfileFor = useCallback((targetPeerId: string) => {
    setProfilePeerId(targetPeerId);
  }, []);

  /** Open the group info panel for a specific group (context-menu action).
   *  Access is verified first: a stale group (left or removed, with a missed
   *  event) is dropped instead of lingering with every action failing. */
  const handleOpenGroupInfoFor = useCallback(
    (groupId: string) => {
      void (async () => {
        const ok = await chat.verifyGroupAccess(groupId);
        if (ok) setGroupInfoGroupId(groupId);
      })();
    },
    [chat.verifyGroupAccess]
  );

  const handleRemoveContact = useCallback(
    (targetPeerId: string) => {
      setProfilePeerId(null);
      void chat.removeContact(targetPeerId);
      setPinnedIds((prev) => {
        if (!prev.includes(targetPeerId)) return prev;
        const next = prev.filter((id) => id !== targetPeerId);
        persistPinnedChats(peerId, next);
        return next;
      });
    },
    [chat.removeContact, peerId]
  );

  /** "Delete for me" from the message context menu: client-local removal. */
  const handleDeleteMessage = useCallback(
    (messageId: string) => {
      if (chat.activePeerId) void chat.deleteMessage(chat.activePeerId, messageId);
    },
    [chat.activePeerId, chat.deleteMessage]
  );

  // ---- Group chat wiring --------------------------------------------------

  /** Create a group with the given name and members, then resync so the chat
   *  list shows it immediately. */
  const handleCreateGroup = useCallback(
    async (name: string, memberIds: string[]): Promise<string> => {
      const groupId = await createGroup(name, memberIds);
      await chat.refresh();
      chat.setActivePeerId(groupId);
      toast(t("toast.group_created"), "success");
      return groupId;
    },
    [chat.refresh, chat.setActivePeerId, toast, t]
  );

  /** Fetch fresh group metadata (name, roster, roles) for the info panel. */
  const handleFetchGroupInfo = useCallback(
    (groupId: string) => getGroupInfo(groupId),
    []
  );

  /** Group admin actions: run the relay call, then resync the roster. */
  const handlePromote = useCallback(
    async (groupId: string, peerId: string) => {
      await promoteMember(groupId, peerId);
      await chat.refresh();
      toast(t("toast.member_promoted"), "success");
    },
    [chat.refresh, toast, t]
  );

  const handleDemote = useCallback(
    async (groupId: string, peerId: string) => {
      await demoteMember(groupId, peerId);
      await chat.refresh();
      toast(t("toast.member_demoted"), "success");
    },
    [chat.refresh, toast, t]
  );

  const handleRemoveMember = useCallback(
    async (groupId: string, peerId: string) => {
      await removeMember(groupId, peerId);
      await chat.refresh();
      toast(t("toast.member_removed"), "success");
    },
    [chat.refresh, toast, t]
  );

  /** Add a member to a group after creation (owner/admin). The backend shares
   *  every existing member's Megolm key to the newcomer; a refresh resyncs the
   *  roster so the member count and the info panel update right away. */
  const handleAddMember = useCallback(
    async (groupId: string, peerId: string) => {
      await addGroupMember(groupId, peerId);
      await chat.refresh();
      toast(t("toast.member_added"), "success");
    },
    [chat.refresh, toast, t]
  );

  /** Set a group's avatar (owner/admin). The backend stores the blob
   *  content-addressed; a refresh repoints the group's avatar_url so the chat
   *  list and header render the new photo. */
  const handleSetGroupAvatar = useCallback(
    async (groupId: string, avatarBase64: string) => {
      await setGroupAvatar(groupId, avatarBase64);
      await chat.refresh();
      toast(t("toast.group_avatar_updated"), "success");
    },
    [chat.refresh, toast, t]
  );

  const handleLeaveGroup = useCallback(
    async (groupId: string) => {
      try {
        await chat.leaveGroup(groupId);
        setGroupInfoGroupId(null);
        setPinnedIds((prev) => {
          if (!prev.includes(groupId)) return prev;
          const next = prev.filter((id) => id !== groupId);
          persistPinnedChats(peerId, next);
          return next;
        });
        toast(t("toast.group_left"), "success");
      } catch (err) {
        // Never swallow a failure: the group stays on the roster (and in the
        // list) until the leave actually goes through, and the user must see
        // why instead of a silent no-op.
        const message = String(err).replace(/^Error:\s*/, "");
        toast(message, "error");
      }
    },
    [chat.leaveGroup, peerId, toast, t]
  );

  const handleReset = useCallback(() => {
    void resetRelay();
    onReset();
  }, [onReset]);

  /** Send a friend request (AddContactDialog + directory search results). The
   *  peer becomes chatable once they accept; failures propagate so the dialog
   *  can show the translated code. */
  const handleSendFriendRequest = useCallback(
    async (targetPeerId: string) => {
      await chat.sendFriendRequest(targetPeerId);
      toast(t("contacts.request_sent"), "success");
    },
    [chat, t, toast]
  );

  /** Accept an incoming friend request; both sides become contacts. The relay
   *  pushes `friend_request_accepted` to BOTH peers, so the `contact-added`
   *  listener toasts and resyncs — nothing extra to show here. */
  const handleAcceptRequest = useCallback(
    async (targetPeerId: string) => {
      try {
        await chat.acceptFriendRequest(targetPeerId);
        void chat.refresh();
      } catch (err) {
        toast(String(err).replace(/^Error:\s*/, ""), "error");
      }
    },
    [chat, toast]
  );

  /** Decline an incoming friend request; it disappears from the Requests
   *  section and the requester is notified. */
  const handleDeclineRequest = useCallback(
    async (targetPeerId: string) => {
      try {
        await chat.declineFriendRequest(targetPeerId);
        toast(t("contacts.request_declined", { name: shortPeerId(targetPeerId, 16) }), "info");
      } catch (err) {
        toast(String(err).replace(/^Error:\s*/, ""), "error");
      }
    },
    [chat, t, toast]
  );

  // Conversations ordered by recency of the last message so the chat list
  // behaves like Signal/WhatsApp: most recent activity first. Pinned chats
  // (Signal/Telegram style) sort above everything else. Pending (non-accepted)
  // contacts are not chatable and never appear here — they live in the
  // Sidebar's Requests section instead.
  const conversations = useMemo(
    () =>
      buildConversations(
        chat.contacts.filter((c) => c.status !== "pending"),
        chat.groups,
        chat.messages,
        pinnedIds
      ),
    [chat.contacts, chat.groups, chat.messages, pinnedIds]
  );

  /** Pin/unpin a chat; the choice is persisted per identity. */
  const handleTogglePin = useCallback(
    (targetPeerId: string) => {
      setPinnedIds((prev) => {
        const next = prev.includes(targetPeerId)
          ? prev.filter((id) => id !== targetPeerId)
          : [...prev, targetPeerId];
        persistPinnedChats(peerId, next);
        return next;
      });
    },
    [peerId]
  );

  const active =
    conversations.find((c) => c.peerId === chat.activePeerId) ?? null;

  // The contact shown in the profile dialog; falls back gracefully to the
  // peer ID when the conversation was just removed.
  const profileTarget = profilePeerId
    ? conversations.find((c) => c.peerId === profilePeerId) ?? null
    : null;

  return (
    <div className="flex h-screen overflow-hidden bg-wp-bg text-wp-text">
      <Sidebar
        peerId={peerId}
        myDisplayName={chat.myDisplayName}
        myAvatarUrl={chat.myAvatarUrl ?? myProfile?.avatar_url ?? null}
        conversations={conversations}
        presence={chat.presence}
        activeId={chat.activePeerId}
        connected={chat.connected}
        connecting={chat.connecting}
        reconnecting={chat.reconnecting}
        reconnectInfo={chat.reconnectInfo}
        connectionError={chat.connectionError}
        relayUrl={relayUrl}
        friendRequestsIncoming={chat.friendRequestsIncoming}
        friendRequestsOutgoing={chat.friendRequestsOutgoing}
        onSelect={chat.setActivePeerId}
        onAddContact={() => setAddDialogOpen(true)}
        onNewGroup={() => setNewGroupOpen(true)}
        onStartChat={handleSendFriendRequest}
        onAcceptRequest={handleAcceptRequest}
        onDeclineRequest={handleDeclineRequest}
        onOpenSettings={() => setSettingsOpen(true)}
        onReconnect={() => void chat.connect()}
        onReset={handleReset}
        onOpenProfile={handleOpenProfileFor}
        onOpenGroupInfo={handleOpenGroupInfoFor}
        onRemoveContact={handleRemoveContact}
        onLeaveGroup={handleLeaveGroup}
        pinnedIds={pinnedIds}
        onTogglePin={handleTogglePin}
        unread={chat.unread}
      />
      <ChatView
        conversation={active}
        isTyping={active ? chat.typing[active.peerId] ?? false : false}
        presence={active ? chat.presence[active.peerId] ?? null : null}
        relayUrl={relayUrl}
        myPeerId={peerId}
        onSend={handleSend}
        onTypingChange={handleTypingChange}
        enterToSend={enterToSend}
        onOpenProfile={handleOpenProfile}
        onOpenGroupInfo={
          active?.isGroup ? () => setGroupInfoGroupId(active.peerId) : undefined
        }
        onDeleteMessage={handleDeleteMessage}
        onReact={(messageId, emoji, activeState) =>
          chat.reactToMessage(
            chat.activePeerId ?? "",
            messageId,
            emoji,
            activeState
          )
        }
        draft={chat.activePeerId ? drafts[chat.activePeerId] ?? "" : ""}
        onDraftChange={handleDraftChange}
      />
      <AddContactDialog
        open={addDialogOpen}
        onOpenChange={setAddDialogOpen}
        onAdd={handleSendFriendRequest}
        myPeerId={peerId}
        initialValue={inviteValue}
      />
      <NewGroupDialog
        open={newGroupOpen}
        onOpenChange={setNewGroupOpen}
        onCreate={handleCreateGroup}
        myPeerId={peerId}
        contacts={chat.contacts}
      />
      <GroupInfoDialog
        open={groupInfoGroupId !== null}
        groupId={groupInfoGroupId}
        onOpenChange={(open) => {
          if (!open) setGroupInfoGroupId(null);
        }}
        onFetchInfo={handleFetchGroupInfo}
        onAddMember={handleAddMember}
        onSetGroupAvatar={handleSetGroupAvatar}
        onPromote={handlePromote}
        onDemote={handleDemote}
        onRemove={handleRemoveMember}
        onLeave={handleLeaveGroup}
        onTransferOwnership={chat.transferOwnership}
        contacts={chat.contacts}
        relayUrl={relayUrl}
      />
      <ProfileDialog
        open={profilePeerId !== null}
        onOpenChange={(open) => {
          if (!open) setProfilePeerId(null);
        }}
        peerId={profilePeerId ?? ""}
        relayUrl={relayUrl}
        fallbackDisplayName={profileTarget?.displayName ?? null}
        fallbackUsername={profileTarget?.username ?? null}
        fallbackAvatarUrl={profileTarget?.avatarUrl ?? null}
        initialPresence={profilePeerId ? chat.presence[profilePeerId] ?? null : null}
        onMessage={() => setProfilePeerId(null)}
        onRemoveContact={handleRemoveContact}
      />
      <SettingsDialog
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        peerId={peerId}
        myDisplayName={chat.myDisplayName}
        myUsername={myProfile?.username ?? null}
        myAvatarUrl={chat.myAvatarUrl ?? myProfile?.avatar_url ?? null}
        theme={theme}
        onThemeChange={handleThemeChange}
        relayUrl={relayUrl}
        onSaveDisplayName={chat.saveDisplayName}
        onRegisterUsername={handleRegisterUsername}
        onSetAvatar={handleSetAvatar}
        onReset={handleReset}
        presenceVisible={presenceVisible}
        onPresenceVisibleChange={handlePresenceVisibleChange}
        readReceipts={readReceipts}
        onReadReceiptsChange={handleReadReceiptsChange}
        typingIndicator={typingIndicator}
        onTypingIndicatorChange={handleTypingIndicatorChange}
        notificationsEnabled={notificationsEnabled}
        onNotificationsEnabledChange={handleNotificationsEnabledChange}
        notificationPreview={notificationPreview}
        onNotificationPreviewChange={handleNotificationPreviewChange}
        notificationSound={notificationSound}
        onNotificationSoundChange={handleNotificationSoundChange}
        autostart={autostart}
        onAutostartChange={handleAutostartChange}
        minimizeToTray={minimizeToTray}
        onMinimizeToTrayChange={handleMinimizeToTrayChange}
        enterToSend={enterToSend}
        onEnterToSendChange={handleEnterToSendChange}
        messageFontScale={messageFontScale}
        onMessageFontScaleChange={handleMessageFontScaleChange}
        onExportIdentity={handleExportIdentity}
        onImportIdentity={handleImportIdentity}
        onClearHistory={handleClearHistory}
      />
    </div>
  );
}
