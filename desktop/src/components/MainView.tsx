import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  ContactInfo,
  Conversation,
  GroupInfo,
  Message,
  MessageStatus,
  PresenceInfo,
  ProfileInfo,
} from "../types";
import {
  connectRelay,
  createGroup,
  demoteMember,
  getChatState,
  getGroupInfo,
  getPresence,
  getProfile,
  getSettings,
  leaveGroup,
  onChatMessage,
  onContactUpdated,
  onMessageStatus,
  onPresence,
  onRelayStatus,
  onTyping,
  promoteMember,
  publishPrekeys,
  registerProfile,
  removeContact,
  removeMember,
  resetRelay,
  sendMessage,
  sendTyping,
  setAvatar,
  setDisplayName as persistDisplayName,
  setPrivacy,
  setRelayUrl as persistRelayUrl,
  setTheme as persistTheme,
  startChat,
  updateSettings,
  watchPresence,
} from "../lib/relay";
import { isGroupId, shortPeerId } from "../lib/format";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { Sidebar } from "./Sidebar";
import { ChatView } from "./ChatView";
import { AddContactDialog } from "./AddContactDialog";
import { GroupInfoDialog } from "./GroupInfoDialog";
import { NewGroupDialog } from "./NewGroupDialog";
import { ProfileDialog } from "./ProfileDialog";
import { SettingsDialog } from "./SettingsDialog";

type Theme = "dark" | "light";

/** How often to re-fetch the active peer's presence (pushes are real-time;
 *  the poll only guarantees freshness across reconnects). */
const PRESENCE_POLL_MS = 30_000;

/**
 * Show an HTML5 desktop notification for an incoming message. Only called
 * while the window is unfocused and notifications are enabled. Permission is
 * requested once per session; if it is not granted the toggle stays on but
 * nothing is shown (documented in the Notifications settings tab).
 */
let notificationPermissionRequested = false;

async function showChatNotification(
  peerId: string,
  message: Message,
  contacts: ContactInfo[],
  preview: boolean
): Promise<void> {
  if (typeof Notification === "undefined") return;
  if (Notification.permission === "denied") return;
  if (Notification.permission !== "granted") {
    if (notificationPermissionRequested) return;
    notificationPermissionRequested = true;
    try {
      const permission = await Notification.requestPermission();
      if (permission !== "granted") return;
    } catch {
      return;
    }
  }
  const contact = contacts.find((c) => c.peer_id === peerId);
  const name =
    contact?.display_name ??
    (contact?.username ? `@${contact.username}` : shortPeerId(peerId, 16));
  const body = preview
    ? `${name}: ${message.text}`
    : `New message from ${name}`;
  try {
    new Notification("Whisper", { body });
  } catch {
    // The webview may not support the Notification API; the toggle stays on
    // and nothing is shown.
  }
}

interface MainViewProps {
  peerId: string;
  onReset: () => void;
}

export function MainView({ peerId, onReset }: MainViewProps) {
  const [contacts, setContacts] = useState<ContactInfo[]>([]);
  const [myDisplayName, setMyDisplayName] = useState<string | null>(null);
  // Our own public profile (username + avatar) fetched from the directory.
  const [myProfile, setMyProfile] = useState<ProfileInfo | null>(null);
  const [messages, setMessages] = useState<Record<string, Message[]>>({});
  const [groups, setGroups] = useState<GroupInfo[]>([]);
  const [connected, setConnected] = useState(false);
  const [connecting, setConnecting] = useState(true);
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [activePeerId, setActivePeerId] = useState<string | null>(null);
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [newGroupOpen, setNewGroupOpen] = useState(false);
  const [groupInfoGroupId, setGroupInfoGroupId] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [theme, setTheme] = useState<Theme>("dark");
  const [relayUrl, setRelayUrl] = useState("");
  // Privacy / notification preferences, hydrated from the settings store on
  // mount and persisted on change.
  const [presenceVisible, setPresenceVisible] = useState(true);
  const [readReceipts, setReadReceipts] = useState(true);
  const [typingIndicator, setTypingIndicator] = useState(true);
  const [notificationsEnabled, setNotificationsEnabled] = useState(true);
  const [notificationPreview, setNotificationPreview] = useState(true);
  // Peer whose profile dialog is open; null when closed.
  const [profilePeerId, setProfilePeerId] = useState<string | null>(null);
  // Per-peer typing state fed by the `typing` event (with a 5s auto-timeout
  // on the backend, so it can never get stuck on "on").
  const [typing, setTyping] = useState<Record<string, boolean>>({});
  // Per-peer presence (online status + last-seen), fed by pushes and the poll.
  const [presence, setPresence] = useState<Record<string, PresenceInfo>>({});

  // The chat-message listener is registered once but must read the *current*
  // notification prefs and contact list, so they live in a ref updated on
  // every render.
  const notifyPrefs = useRef({ notificationsEnabled, notificationPreview, contacts });
  notifyPrefs.current = { notificationsEnabled, notificationPreview, contacts };

  const connect = useCallback(async () => {
    setConnecting(true);
    setConnectionError(null);
    try {
      await connectRelay();
      await publishPrekeys();
    } catch (err) {
      setConnectionError(String(err));
    } finally {
      setConnecting(false);
    }
  }, []);

  const refresh = useCallback(async () => {
    try {
      const state = await getChatState();
      setContacts(state.contacts);
      setMessages(state.messages);
      setGroups(state.groups);
      setConnected(state.connected);
      setMyDisplayName(state.my_display_name);
      setPresence(state.presence);
    } catch {
      // Transient failure; event listeners resync the next state change.
    }
  }, []);

  // Fetch our own public profile (username + avatar) from the directory.
  // Rejects with `no_profile` while unregistered, or when the backend command
  // isn't wired up yet — both fall back to the unregistered UI.
  const refreshOwnProfile = useCallback(async () => {
    try {
      const profile = await getProfile(peerId);
      setMyProfile(profile);
    } catch {
      // `no_profile` (unregistered yet) or the command isn't wired on the
      // backend — treat as unregistered. Also clears stale data after an
      // identity reset, since MainView persists across peerId changes.
      setMyProfile(null);
    }
  }, [peerId]);

  useEffect(() => {
    void refreshOwnProfile();
  }, [refreshOwnProfile]);

  // Keep the DOM attribute in sync with the active theme. The stylesheet
  // defines a light variant under `[data-theme="light"]`; anything else
  // falls back to the default dark palette.
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

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
        if (settings.relay_url) setRelayUrl(settings.relay_url);
        if (settings.presence_visible != null) setPresenceVisible(settings.presence_visible);
        setReadReceipts(settings.read_receipts ?? true);
        setTypingIndicator(settings.typing_indicator ?? true);
        setNotificationsEnabled(settings.notifications_enabled ?? true);
        setNotificationPreview(settings.notification_preview ?? true);
      } catch {
        // Settings are best-effort; the defaults (dark, default relay) apply.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    let disposed = false;

    // Register a listener and immediately reap it if the effect was cleaned
    // up mid-registration (React StrictMode double-mounts in dev).
    const register = async (
      subscribe: () => Promise<UnlistenFn>
    ): Promise<UnlistenFn | null> => {
      const unlisten = await subscribe();
      if (disposed) {
        unlisten();
        return null;
      }
      unlisteners.push(unlisten);
      return unlisten;
    };

    const setup = async () => {
      // Listeners are registered BEFORE the first connection attempt, so the
      // `relay-status` and `chat-message` events can never race past the
      // subscription window.
      const chatUnlisten = await register(() =>
        onChatMessage(({ peer_id, message }) => {
          if (disposed) return;
          setContacts((prev) =>
            prev.some((c) => c.peer_id === peer_id)
              ? prev
              : [...prev, { peer_id, display_name: null }]
          );
          setMessages((prev) => {
            const list = prev[peer_id] ?? [];
            if (list.some((m) => m.id === message.id)) return prev;
            return { ...prev, [peer_id]: [...list, message] };
          });
          setActivePeerId((prev) => prev ?? peer_id);
          // Desktop notification: incoming message + window unfocused +
          // notifications enabled (preview text controlled by the setting).
          if (message.outgoing) return;
          const prefs = notifyPrefs.current;
          if (!prefs.notificationsEnabled) return;
          if (document.hasFocus()) return;
          void showChatNotification(peer_id, message, prefs.contacts, prefs.notificationPreview);
        })
      );
      const statusUnlisten = await register(() =>
        onRelayStatus(({ connected: isConnected }) => {
          if (disposed) return;
          setConnected(isConnected);
          if (isConnected) void refresh();
        })
      );
      const messageStatusUnlisten = await register(() =>
        onMessageStatus(({ client_id, status }) => {
          if (disposed) return;
          // Status events carry only the client id, so match it against every
          // peer's history and flip the matching message. `status` is either
          // "delivered" (relay ack) or "read" (end-to-end read receipt).
          const rank: Record<MessageStatus, number> = { sent: 0, delivered: 1, read: 2 };
          setMessages((prev) => {
            let changed = false;
            const next: Record<string, Message[]> = {};
            for (const [peer, list] of Object.entries(prev)) {
              next[peer] = list.map((m) => {
                if (m.id !== client_id) return m;
                // Promotion-only: an out-of-order event must never downgrade
                // an already "read" (or "delivered") message.
                if (rank[status] <= rank[m.status ?? "delivered"]) return m;
                changed = true;
                return { ...m, status };
              });
            }
            return changed ? next : prev;
          });
        })
      );
      const typingUnlisten = await register(() =>
        onTyping(({ peer_id, is_typing }) => {
          if (disposed) return;
          setTyping((prev) =>
            prev[peer_id] === is_typing
              ? prev
              : { ...prev, [peer_id]: is_typing }
          );
        })
      );
      const contactUpdatedUnlisten = await register(() =>
        onContactUpdated(({ peer_id, display_name }) => {
          if (disposed) return;
          setContacts((prev) =>
            prev.some((c) => c.peer_id === peer_id)
              ? prev.map((c) =>
                  c.peer_id === peer_id ? { ...c, display_name } : c
                )
              : [...prev, { peer_id, display_name }]
          );
        })
      );
      const presenceUnlisten = await register(() =>
        onPresence(({ peer_id, online, last_seen }) => {
          if (disposed) return;
          setPresence((prev) =>
            prev[peer_id]?.online === online &&
            prev[peer_id]?.last_seen === last_seen
              ? prev
              : { ...prev, [peer_id]: { online, last_seen } }
          );
        })
      );
      if (
        disposed ||
        !chatUnlisten ||
        !statusUnlisten ||
        !messageStatusUnlisten ||
        !typingUnlisten ||
        !contactUpdatedUnlisten ||
        !presenceUnlisten
      ) {
        return;
      }
      // connect() must settle before refresh() so the snapshot reflects the
      // established connection — offline messages included — and the UI is
      // consistent the moment it first renders.
      void (async () => {
        await connect();
        await refresh();
      })();
    };

    void setup();

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
    };
  }, [connect, refresh]);

  // Keep the active peer's presence current: a `watch_presence` subscription
  // delivers real-time online/offline pushes, while a 30-second `get_presence`
  // poll seeds the initial state and covers events missed across reconnects.
  // Re-running on `connected` re-subscribes after every reconnect.
  useEffect(() => {
    if (!activePeerId) return;
    let cancelled = false;
    const poll = async () => {
      try {
        const info = await getPresence(activePeerId);
        if (!cancelled) {
          setPresence((prev) => ({ ...prev, [activePeerId]: info }));
        }
      } catch {
        // Best-effort: a transient failure (e.g. while disconnected) is
        // recovered by the next poll or by a presence push.
      }
    };
    if (connected) {
      void watchPresence(activePeerId).catch(() => {});
      void poll();
    }
    const timer = setInterval(poll, PRESENCE_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [activePeerId, connected]);

  const handleThemeChange = useCallback((next: Theme) => {
    setTheme(next);
    void persistTheme(next).catch(() => {
      // The theme is applied in memory immediately; persistence only affects
      // the next launch, so a failure here is non-fatal.
    });
  }, []);

  const handleRelayUrlSave = useCallback(
    async (url: string) => {
      const trimmed = url.trim();
      await persistRelayUrl(trimmed);
      setRelayUrl(trimmed);
      // Reconnect so the new endpoint takes effect, then resync state.
      await connect();
      await refresh();
    },
    [connect, refresh]
  );

  const handleSend = useCallback(
    async (text: string) => {
      if (!activePeerId) return;
      const clientId = crypto.randomUUID();
      // Optimistic insertion; the backend echoes the same client id in the
      // `chat-message` event, which the dedup logic above ignores. The status
      // flips to "delivered" on the relay ack and "read" on a read receipt.
      setMessages((prev) => ({
        ...prev,
        [activePeerId]: [
          ...(prev[activePeerId] ?? []),
          { id: clientId, text, outgoing: true, timestamp: Date.now(), status: "sent" },
        ],
      }));
      try {
        await sendMessage(activePeerId, text, clientId);
      } catch (err) {
        setMessages((prev) => ({
          ...prev,
          [activePeerId]: (prev[activePeerId] ?? []).filter(
            (m) => m.id !== clientId
          ),
        }));
        setConnectionError(String(err));
      }
    },
    [activePeerId]
  );

  const handleTypingChange = useCallback(
    (isTyping: boolean) => {
      if (!activePeerId) return;
      // Best-effort: without an established session (or while disconnected)
      // there is no session to encrypt the indicator with.
      void sendTyping(activePeerId, isTyping).catch(() => {});
    },
    [activePeerId]
  );

  const handleSaveDisplayName = useCallback(
    async (name: string) => {
      const trimmed = name.trim();
      await persistDisplayName(trimmed);
      setMyDisplayName(trimmed || null);
    },
    []
  );

  const handleRegisterUsername = useCallback(
    async (username: string) => {
      await registerProfile(username);
      // Re-fetch the profile so the UI picks up the new username.
      await refreshOwnProfile();
    },
    [refreshOwnProfile]
  );

  const handleSetAvatar = useCallback(
    async (avatarBase64: string) => {
      await setAvatar(avatarBase64);
      // Re-fetch the profile so the avatar_url (and preview) refresh.
      await refreshOwnProfile();
    },
    [refreshOwnProfile]
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

  // Profile dialog wiring: opening focuses the active conversation's peer;
  // "Message" just closes the dialog (the chat is already open).
  const handleOpenProfile = useCallback(() => {
    if (activePeerId) setProfilePeerId(activePeerId);
  }, [activePeerId]);

  /** Remove a contact and its messages on this device (client-local). The
   *  Rust backend drops the contact row, history and session; this keeps the
   *  React state in sync so no refresh is needed. The peer's own copy and any
   *  relay-queued envelopes are untouched — a later message re-establishes
   *  the contact. */
  const handleRemoveContact = useCallback(async (targetPeerId: string) => {
    try {
      await removeContact(targetPeerId);
    } catch {
      // Client-local best-effort: the in-memory removal below still applies
      // for this session.
    }
    setContacts((prev) => prev.filter((c) => c.peer_id !== targetPeerId));
    setMessages((prev) => {
      const next = { ...prev };
      delete next[targetPeerId];
      return next;
    });
    setActivePeerId((prev) => (prev === targetPeerId ? null : prev));
    setProfilePeerId(null);
  }, []);

  // ---- Group chat wiring --------------------------------------------------

  /** Create a group with the given name and members, then resync so the chat
   *  list shows it immediately. */
  const handleCreateGroup = useCallback(
    async (name: string, memberIds: string[]): Promise<string> => {
      const groupId = await createGroup(name, memberIds);
      await refresh();
      setActivePeerId(groupId);
      return groupId;
    },
    [refresh]
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
      await refresh();
    },
    [refresh]
  );

  const handleDemote = useCallback(
    async (groupId: string, peerId: string) => {
      await demoteMember(groupId, peerId);
      await refresh();
    },
    [refresh]
  );

  const handleRemoveMember = useCallback(
    async (groupId: string, peerId: string) => {
      await removeMember(groupId, peerId);
      await refresh();
    },
    [refresh]
  );

  const handleLeaveGroup = useCallback(
    async (groupId: string) => {
      await leaveGroup(groupId);
      setGroupInfoGroupId(null);
      await refresh();
      // Close the conversation if the active chat was the group we left.
      setActivePeerId((prev) => (prev === groupId ? null : prev));
    },
    [refresh]
  );

  const handleAddContact = useCallback(
    async (peerIdToAdd: string) => {      try {
        await startChat(peerIdToAdd);
        setContacts((prev) =>
          prev.some((c) => c.peer_id === peerIdToAdd)
            ? prev
            : [...prev, { peer_id: peerIdToAdd, display_name: null }]
        );
        setActivePeerId(peerIdToAdd);
        // Best-effort enrichment: pull the peer's public profile (display
        // name, username, avatar) so the contact renders fully right away.
        try {
          const profile = await getProfile(peerIdToAdd);
          setContacts((prev) =>
            prev.map((c) =>
              c.peer_id === peerIdToAdd
                ? {
                    ...c,
                    display_name: profile.display_name,
                    username: profile.username,
                    avatar_url: profile.avatar_url,
                  }
                : c
            )
          );
        } catch {
          // No registered profile (or lookup unavailable) — display name and
          // presence still arrive via the usual events.
        }
        void refresh();
      } catch (err) {
        throw new Error(String(err));
      }
    },
    [refresh]
  );

  const handleReset = useCallback(() => {
    void resetRelay();
    onReset();
  }, [onReset]);

  // Conversations ordered by recency of the last message so the chat list
  // behaves like Signal/WhatsApp: most recent activity first. Groups appear
  // in the same list (keyed by their group ID with the group name as the
  // display name); their letter avatar and member count distinguish them.
  const conversations: Conversation[] = useMemo(() => {
    const groupById = new Map(groups.map((g) => [g.group_id, g]));
    return contacts
      .map((contact) => {
        const isGroup = isGroupId(contact.peer_id);
        const group = isGroup ? groupById.get(contact.peer_id) : undefined;
        return {
          id: contact.peer_id,
          name: isGroup
            ? group?.name ?? contact.display_name ?? shortPeerId(contact.peer_id)
            : contact.display_name ?? shortPeerId(contact.peer_id),
          displayName: isGroup ? null : contact.display_name,
          peerId: contact.peer_id,
          username: isGroup ? undefined : contact.username,
          avatarUrl: isGroup ? undefined : contact.avatar_url,
          isGroup,
          memberCount: group?.members.length,
          messages: messages[contact.peer_id] ?? [],
        };
      })
      .sort((a, b) => {
        const lastA = a.messages[a.messages.length - 1]?.timestamp ?? 0;
        const lastB = b.messages[b.messages.length - 1]?.timestamp ?? 0;
        return lastB - lastA;
      });
  }, [contacts, groups, messages]);

  const active =
    conversations.find((c) => c.peerId === activePeerId) ?? null;

  // The contact shown in the profile dialog; falls back gracefully to the
  // peer ID when the conversation was just removed.
  const profileTarget = profilePeerId
    ? conversations.find((c) => c.peerId === profilePeerId) ?? null
    : null;

  return (
    <div className="flex h-screen overflow-hidden bg-wp-bg text-wp-text">
      <Sidebar
        peerId={peerId}
        myDisplayName={myDisplayName}
        conversations={conversations}
        presence={presence}
        activeId={activePeerId}
        connected={connected}
        connecting={connecting}
        connectionError={connectionError}
        relayUrl={relayUrl}
        onSelect={setActivePeerId}
        onAddContact={() => setAddDialogOpen(true)}
        onNewGroup={() => setNewGroupOpen(true)}
        onStartChat={handleAddContact}
        onOpenSettings={() => setSettingsOpen(true)}
        onReconnect={() => void connect()}
        onReset={handleReset}
      />
      <ChatView
        conversation={active}
        isTyping={active ? typing[active.peerId] ?? false : false}
        presence={active ? presence[active.peerId] ?? null : null}
        relayUrl={relayUrl}
        onSend={(t) => void handleSend(t)}
        onTypingChange={handleTypingChange}
        onOpenProfile={handleOpenProfile}
        onOpenGroupInfo={
          active?.isGroup ? () => setGroupInfoGroupId(active.peerId) : undefined
        }
      />
      <AddContactDialog
        open={addDialogOpen}
        onOpenChange={setAddDialogOpen}
        onAdd={handleAddContact}
      />
      <NewGroupDialog
        open={newGroupOpen}
        onOpenChange={setNewGroupOpen}
        onCreate={handleCreateGroup}
        myPeerId={peerId}
      />
      <GroupInfoDialog
        open={groupInfoGroupId !== null}
        groupId={groupInfoGroupId}
        onOpenChange={(open) => {
          if (!open) setGroupInfoGroupId(null);
        }}
        onFetchInfo={handleFetchGroupInfo}
        onPromote={handlePromote}
        onDemote={handleDemote}
        onRemove={handleRemoveMember}
        onLeave={handleLeaveGroup}
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
        initialPresence={profilePeerId ? presence[profilePeerId] ?? null : null}
        onMessage={() => setProfilePeerId(null)}
        onRemoveContact={(id) => void handleRemoveContact(id)}
      />
      <SettingsDialog
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        peerId={peerId}
        myDisplayName={myDisplayName}
        myUsername={myProfile?.username ?? null}
        myAvatarUrl={myProfile?.avatar_url ?? null}
        theme={theme}
        onThemeChange={handleThemeChange}
        relayUrl={relayUrl}
        onSaveRelayUrl={handleRelayUrlSave}
        onSaveDisplayName={handleSaveDisplayName}
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
      />
    </div>
  );
}
