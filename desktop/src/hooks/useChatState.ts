import { useCallback, useEffect, useRef, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type { TFunction } from "../i18n/types";
import type {
  ContactInfo,
  GroupInfo,
  Message,
  MessageStatus,
  PresenceInfo,
} from "../types";
import {
  connectRelay,
  deleteMessage as relayDeleteMessage,
  getChatState,
  getProfile,
  onChatMessage,
  onContactUpdated,
  onMessageStatus,
  onPresence,
  onReconnecting,
  onRelayStatus,
  leaveGroup as relayLeaveGroup,
  onTyping,
  publishPrekeys,
  removeContact as relayRemoveContact,
  sendMessage as relaySendMessage,
  sendTyping as relaySendTyping,
  setDisplayName as persistDisplayName,
  startChat,
  transferOwnership as relayTransferOwnership,
} from "../lib/relay";
import { shortPeerId } from "../lib/format";
import { playNotificationSound } from "../lib/sound";

/** Whether the OS-level notification permission has been granted. */
let notificationPermission = false;
/** Whether a permission request is already in flight (avoid re-prompting). */
let notificationPermissionRequested = false;

/** Ensure the OS notification permission is granted, requesting it once per
 *  session when needed. Returns false when the permission is denied or the
 *  plugin is unavailable. */
async function ensureNotificationPermission(): Promise<boolean> {
  if (notificationPermission) return true;
  if (notificationPermissionRequested) return false;
  notificationPermissionRequested = true;
  try {
    if (await isPermissionGranted()) {
      notificationPermission = true;
      return true;
    }
    if (await requestPermission()) {
      notificationPermission = true;
      return true;
    }
    return false;
  } catch {
    return false;
  }
}

/**
 * Show a native desktop notification (via the Tauri notification plugin, not
 * the HTML5 Notification API which is unreliable inside the webview) for an
 * incoming message. Only called while the window is unfocused and
 * notifications are enabled. If the system permission is denied, the toggle
 * stays on but nothing is shown (documented in the Notifications settings
 * tab).
 */
async function showChatNotification(
  peerId: string,
  message: Message,
  contacts: ContactInfo[],
  preview: boolean,
  t: TFunction
): Promise<void> {
  if (!(await ensureNotificationPermission())) return;
  const contact = contacts.find((c) => c.peer_id === peerId);
  const name =
    contact?.display_name ??
    (contact?.username ? `@${contact.username}` : shortPeerId(peerId, 16));
  const body = preview
    ? `${name}: ${message.text}`
    : t("common.new_message_from", { name });
  try {
    sendNotification({ title: "Whisper", body });
  } catch {
    // The plugin may be unavailable in this build; the toggle stays on and
    // nothing is shown.
  }
}

interface UseChatStateParams {
  /** Desktop-notification prefs, hydrated from persisted settings. The
   *  chat-message listener reads them through a ref updated every render. */
  notificationsEnabled: boolean;
  notificationPreview: boolean;
  /** Whether a short chime plays for incoming messages. */
  notificationSound: boolean;
  /** Translation function for the notification body text. */
  t: TFunction;
}

export interface ChatStateApi {
  contacts: ContactInfo[];
  myDisplayName: string | null;
  messages: Record<string, Message[]>;
  groups: GroupInfo[];
  connected: boolean;
  connecting: boolean;
  /** Whether the Rust side is retrying a dropped connection automatically. */
  reconnecting: boolean;
  /** Current auto-reconnect progress; null while not reconnecting. */
  reconnectInfo: { attempt: number; nextInMs: number } | null;
  connectionError: string | null;
  typing: Record<string, boolean>;
  presence: Record<string, PresenceInfo>;
  activePeerId: string | null;
  setActivePeerId: Dispatch<SetStateAction<string | null>>;
  /** Unread incoming-message counts per peer; cleared when opened. */
  unread: Record<string, number>;
  connect: () => Promise<void>;
  refresh: () => Promise<void>;
  sendMessage: (peerId: string, text: string) => Promise<void>;
  sendTyping: (peerId: string, isTyping: boolean) => void;
  saveDisplayName: (name: string) => Promise<void>;
  removeContact: (peerId: string) => Promise<void>;
  addContact: (peerId: string) => Promise<void>;
  updatePresence: (peerId: string, info: PresenceInfo) => void;
  deleteMessage: (peerId: string, messageId: string) => Promise<void>;
  /** Remove the caller from a group and drop it from every local list. */
  leaveGroup: (groupId: string) => Promise<void>;
  /** Transfer group ownership to `peerId`, then resync the roster and our own
   *  role so the UI reflects the new owner immediately. */
  transferOwnership: (groupId: string, peerId: string) => Promise<void>;
}

/** Owns the chat state (contacts, messages, groups, connection, presence,
 *  typing) together with the event listeners that keep it live and the
 *  high-level send/remove/add operations that mutate it. */
export function useChatState({
  notificationsEnabled,
  notificationPreview,
  notificationSound,
  t,
}: UseChatStateParams): ChatStateApi {
  const [contacts, setContacts] = useState<ContactInfo[]>([]);
  const [myDisplayName, setMyDisplayName] = useState<string | null>(null);
  const [messages, setMessages] = useState<Record<string, Message[]>>({});
  const [groups, setGroups] = useState<GroupInfo[]>([]);
  const [connected, setConnected] = useState(false);
  const [connecting, setConnecting] = useState(true);
  const [reconnecting, setReconnecting] = useState(false);
  const [reconnectInfo, setReconnectInfo] = useState<{
    attempt: number;
    nextInMs: number;
  } | null>(null);
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [typing, setTyping] = useState<Record<string, boolean>>({});
  const [presence, setPresence] = useState<Record<string, PresenceInfo>>({});
  const [activePeerId, setActivePeerIdState] = useState<string | null>(null);
  const [unread, setUnread] = useState<Record<string, number>>({});

  // The chat-message listener is registered once but must read the *current*
  // notification prefs and contact list, so they live in a ref updated on
  // every render.
  const notifyPrefs = useRef({ notificationsEnabled, notificationPreview, notificationSound, contacts, t });
  notifyPrefs.current = { notificationsEnabled, notificationPreview, notificationSound, contacts, t };

  // The same applies to the active conversation: the message listener needs
  // it to decide whether an incoming message counts as unread.
  const activePeerIdRef = useRef<string | null>(null);
  activePeerIdRef.current = activePeerId;

  /** Switch the active conversation; opening one clears its unread badge. */
  const setActivePeerId = useCallback<Dispatch<SetStateAction<string | null>>>(
    (value) => {
      setActivePeerIdState((prev) => {
        const next = typeof value === "function" ? value(prev) : value;
        if (next && next !== prev) {
          setUnread((counts) => (counts[next] ? { ...counts, [next]: 0 } : counts));
        }
        return next;
      });
    },
    []
  );

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
          // Unread badge: count incoming messages for conversations that are
          // not the one currently on screen. Opening the chat clears the count
          // via the wrapped `setActivePeerId`.
          if (!message.outgoing && activePeerIdRef.current !== peer_id) {
            setUnread((counts) => ({
              ...counts,
              [peer_id]: (counts[peer_id] ?? 0) + 1,
            }));
          }
          // Incoming message feedback: a short chime (when the notification
          // sound is enabled, regardless of window focus — like WhatsApp) and
          // a desktop notification when the window is unfocused and desktop
          // notifications are enabled (preview text controlled by the setting).
          if (message.outgoing) return;
          const prefs = notifyPrefs.current;
          if (prefs.notificationSound) {
            playNotificationSound();
          }
          if (!prefs.notificationsEnabled) return;
          if (document.hasFocus()) return;
          void showChatNotification(
            peer_id,
            message,
            prefs.contacts,
            prefs.notificationPreview,
            prefs.t
          );
        })
      );
      const statusUnlisten = await register(() =>
        onRelayStatus(({ connected: isConnected }) => {
          if (disposed) return;
          setConnected(isConnected);
          if (isConnected) {
            // A live connection means no auto-reconnect is pending and any
            // earlier connect failure is moot.
            setConnectionError(null);
            setReconnecting(false);
            setReconnectInfo(null);
            void refresh();
          }
        })
      );
      const reconnectingUnlisten = await register(() =>
        onReconnecting(({ active, attempt, next_in_ms }) => {
          if (disposed) return;
          setReconnecting(active);
          setReconnectInfo(active ? { attempt, nextInMs: next_in_ms } : null);
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
        !reconnectingUnlisten ||
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

  const sendMessage = useCallback(async (peerId: string, text: string) => {
    const clientId = crypto.randomUUID();
    // Optimistic insertion; the backend echoes the same client id in the
    // `chat-message` event, which the dedup logic above ignores. The status
    // flips to "delivered" on the relay ack and "read" on a read receipt.
    setMessages((prev) => ({
      ...prev,
      [peerId]: [
        ...(prev[peerId] ?? []),
        { id: clientId, text, outgoing: true, timestamp: Date.now(), status: "sent" },
      ],
    }));
    try {
      await relaySendMessage(peerId, text, clientId);
    } catch (err) {
      setMessages((prev) => ({
        ...prev,
        [peerId]: (prev[peerId] ?? []).filter(
          (m) => m.id !== clientId
        ),
      }));
      setConnectionError(String(err));
    }
  }, []);

  const sendTyping = useCallback((peerId: string, isTyping: boolean) => {
    // Best-effort: without an established session (or while disconnected)
    // there is no session to encrypt the indicator with.
    void relaySendTyping(peerId, isTyping).catch(() => {});
  }, []);

  const saveDisplayName = useCallback(async (name: string) => {
    const trimmed = name.trim();
    await persistDisplayName(trimmed);
    setMyDisplayName(trimmed || null);
  }, []);

  const updatePresence = useCallback((peerId: string, info: PresenceInfo) => {
    setPresence((prev) => ({ ...prev, [peerId]: info }));
  }, []);

  /** Delete one message locally ("delete for me"): the backend drops the row
   *  from the encrypted store and its in-memory history; this removes it from
   *  the React state so no refresh is needed. The peer's copy and any
   *  relay-queued envelopes are untouched. */
  const deleteMessage = useCallback(
    async (targetPeerId: string, messageId: string) => {
      try {
        await relayDeleteMessage(targetPeerId, messageId);
      } catch {
        // Client-local best-effort: the in-memory removal below still applies
        // for this session.
      }
      setMessages((prev) => {
        const list = prev[targetPeerId];
        if (!list || !list.some((m) => m.id === messageId)) return prev;
        return {
          ...prev,
          [targetPeerId]: list.filter((m) => m.id !== messageId),
        };
      });
    },
    []
  );

  /** Remove a contact and its messages on this device (client-local). The
   *  Rust backend drops the contact row, history and session; this keeps the
   *  React state in sync so no refresh is needed. The peer's own copy and any
   *  relay-queued envelopes are untouched — a later message re-establishes
   *  the contact. */
  const removeContact = useCallback(async (targetPeerId: string) => {
    try {
      await relayRemoveContact(targetPeerId);
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
  }, []);

  /**
   * Remove the caller from a group. The backend drops the group contact row,
   * sessions and membership; on success this removes the group from the React
   * state immediately (not just after a `refresh()` round-trip) so it leaves
   * the chat list the moment the action succeeds, and closes it if it was the
   * active conversation. Failures propagate so the dialog can show them — a
   * group must never vanish locally while the relay still lists us as a
   * member. The group is left without an owner when the owner leaves — an
   * acceptable MVP trade-off documented in the UI.
   */
  const leaveGroup = useCallback(async (groupId: string) => {
    await relayLeaveGroup(groupId);
    setContacts((prev) => prev.filter((c) => c.peer_id !== groupId));
    setGroups((prev) => prev.filter((g) => g.group_id !== groupId));
    setMessages((prev) => {
      const next = { ...prev };
      delete next[groupId];
      return next;
    });
    setUnread((counts) => {
      if (!(groupId in counts)) return counts;
      const next = { ...counts };
      delete next[groupId];
      return next;
    });
    setActivePeerId((prev) => (prev === groupId ? null : prev));
  }, []);

  const addContact = useCallback(
    async (peerIdToAdd: string) => {
      try {
        await startChat(peerIdToAdd);
      } catch (err) {
        throw new Error(String(err));
      }
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
    },
    [refresh]
  );

  /**
   * Transfer group ownership to another member. The backend flips the roles
   * (old owner -> admin, `peerId` -> owner); a full refresh resyncs the
   * roster and our own role so the chat list and group panel update right
   * away. Failures propagate so the dialog can surface them.
   */
  const transferOwnership = useCallback(
    async (groupId: string, peerId: string) => {
      await relayTransferOwnership(groupId, peerId);
      await refresh();
    },
    [refresh]
  );

  return {
    contacts,
    myDisplayName,
    messages,
    groups,
    connected,
    connecting,
    reconnecting,
    reconnectInfo,
    connectionError,
    typing,
    presence,
    activePeerId,
    setActivePeerId,
    unread,
    connect,
    refresh,
    sendMessage,
    sendTyping,
    saveDisplayName,
    removeContact,
    addContact,
    updatePresence,
    deleteMessage,
    leaveGroup,
    transferOwnership,
  };
}
