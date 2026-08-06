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
  FriendRequestIncoming,
  GroupInfo,
  Message,
  MessageStatus,
  PresenceInfo,
  QuoteInfo,
} from "../types";
import {
  acceptFriendRequest as relayAcceptFriendRequest,
  connectRelay,
  declineFriendRequest as relayDeclineFriendRequest,
  deleteMessage as relayDeleteMessage,
  getChatState,
  getFriendRequests,
  onChatMessage,
  onContactAdded,
  onContactRemoved,
  onContactUpdated,
  onFriendRequest,
  onFriendRequestDeclined,
  onGroupRemoved,
  onMessageReaction,
  onMessageStatus,
  onPresence,
  onReconnecting,
  onRelayStatus,
  getGroupInfo as relayGetGroupInfo,
  leaveGroup as relayLeaveGroup,
  onTyping,
  publishPrekeys,
  relayErrorCode,
  removeContact as relayRemoveContact,
  sendFriendRequest as relaySendFriendRequest,
  sendMessage as relaySendMessage,
  sendReaction as relaySendReaction,
  sendTyping as relaySendTyping,
  setDisplayName as persistDisplayName,
  transferOwnership as relayTransferOwnership,
} from "../lib/relay";
import { shortPeerId } from "../lib/format";
import { playNotificationSound } from "../lib/sound";
import { useToast } from "./useToast";

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
  /** Incoming friend requests (requester + display name), in arrival order. */
  friendRequestsIncoming: FriendRequestIncoming[];
  /** Outgoing pending friend requests: peer IDs we asked, unanswered. */
  friendRequestsOutgoing: string[];
  myDisplayName: string | null;
  /** Our own peer ID (fingerprint), seeded from the chat-state snapshot. */
  myPeerId: string | null;
  /** Our own avatar path ("/media/{hash}") from the persisted chat-state
   *  snapshot; null when unset. Reliable without a live relay round-trip. */
  myAvatarUrl: string | null;
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
  sendMessage: (peerId: string, text: string, quote?: QuoteInfo | null) => Promise<void>;
  /** React or un-react to a message. `active` is the caller-computed absolute
   *  state (true = react, false = remove my reaction). */
  reactToMessage: (
    peerId: string,
    messageId: string,
    emoji: string,
    active: boolean
  ) => void;
  sendTyping: (peerId: string, isTyping: boolean) => void;
  saveDisplayName: (name: string) => Promise<void>;
  removeContact: (peerId: string) => Promise<void>;
  /** Send a friend request; the peer becomes an accepted contact once they
   *  accept. Failures (relay error codes) propagate to the caller. */
  sendFriendRequest: (peerId: string) => Promise<void>;
  /** Accept an incoming friend request; both sides become contacts. */
  acceptFriendRequest: (peerId: string) => Promise<void>;
  /** Decline an incoming friend request. */
  declineFriendRequest: (peerId: string) => Promise<void>;
  updatePresence: (peerId: string, info: PresenceInfo) => void;
  deleteMessage: (peerId: string, messageId: string) => Promise<void>;
  /** Remove the caller from a group and drop it from every local list. */
  leaveGroup: (groupId: string) => Promise<void>;
  /** Confirm we still have access to a group; drops it locally when not. */
  verifyGroupAccess: (groupId: string) => Promise<boolean>;
  /** Transfer group ownership to `peerId`, then resync the roster and our own
   *  role so the UI reflects the new owner immediately. */
  transferOwnership: (groupId: string, peerId: string) => Promise<void>;
  /** Wipe every message and unread badge from the React state after a
   *  `clear_chat_history` backend call. */
  clearHistory: () => void;
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
  const { toast } = useToast();
  const [contacts, setContacts] = useState<ContactInfo[]>([]);
  const [friendRequestsIncoming, setFriendRequestsIncoming] = useState<
    FriendRequestIncoming[]
  >([]);
  const [friendRequestsOutgoing, setFriendRequestsOutgoing] = useState<string[]>(
    []
  );
  const [myDisplayName, setMyDisplayName] = useState<string | null>(null);
  const [myPeerId, setMyPeerId] = useState<string | null>(null);
  const [myAvatarUrl, setMyAvatarUrl] = useState<string | null>(null);
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
      setMyPeerId(state.my_peer_id);
      setMyAvatarUrl(state.my_avatar_url);
      setPresence(state.presence);
      setFriendRequestsIncoming(state.friend_requests_incoming);
      setFriendRequestsOutgoing(state.friend_requests_outgoing);
    } catch {
      // Transient failure; event listeners resync the next state change.
    }
  }, []);

  /** Re-fetch the pending friend-request snapshot from the relay. The in-memory
   *  lists are seeded after every connect and refreshed after every request
   *  mutation, so the Requests section stays authoritative without waiting for
   *  a live push. */
  const loadFriendRequests = useCallback(async () => {
    try {
      const requests = await getFriendRequests();
      setFriendRequestsIncoming(requests.incoming);
      setFriendRequestsOutgoing(requests.outgoing);
    } catch {
      // Best-effort: a live push resyncs the next time the state changes.
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
            // Seed the Requests section from the relay (the in-memory lists
            // reset on every process run).
            void loadFriendRequests();
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
      // Reactions arrive as absolute state signals (active true/false), so
      // the message map is updated idempotently — a redelivered envelope can
      // never flip a pill the wrong way.
      const reactionUnlisten = await register(() =>
        onMessageReaction(({ peer_id, message_id, sender, emoji, active }) => {
          if (disposed) return;
          setMessages((prev) => {
            const list = prev[peer_id];
            if (!list) return prev;
            const target = list.find((m) => m.id === message_id);
            if (!target) return prev;
            const reactions = active
              ? [
                  ...(target.reactions ?? []).filter((r) => r.sender !== sender),
                  { sender, emoji },
                ]
              : (target.reactions ?? []).filter((r) => r.sender !== sender);
            return {
              ...prev,
              [peer_id]: list.map((m) =>
                m.id === message_id ? { ...m, reactions } : m
              ),
            };
          });
        })
      );
      const contactUpdatedUnlisten = await register(() =>
        onContactUpdated(({ peer_id, display_name, avatar_url }) => {
          if (disposed) return;
          setContacts((prev) =>
            prev.some((c) => c.peer_id === peer_id)
              ? prev.map((c) =>
                  c.peer_id === peer_id
                    ? {
                        ...c,
                        // `null` means "unchanged" (the backend COALESCEs),
                        // so a profile without a field never wipes a known one.
                        display_name: display_name ?? c.display_name,
                        avatar_url: avatar_url ?? c.avatar_url,
                      }
                    : c
                )
              : [
                  ...prev,
                  {
                    peer_id,
                    display_name: display_name ?? null,
                    avatar_url: avatar_url ?? null,
                  },
                ]
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
      const groupRemovedUnlisten = await register(() =>
        onGroupRemoved(({ group_id }) => {
          if (disposed) return;
          // The owner removed us from a group: drop it from every local list
          // and close the conversation if it was open. This is an online-only
          // push in the MVP — an offline member learns about the removal on
          // its next connect, when the relay stops listing it as a member.
          setContacts((prev) => prev.filter((c) => c.peer_id !== group_id));
          setGroups((prev) => prev.filter((g) => g.group_id !== group_id));
          setMessages((prev) => {
            if (!(group_id in prev)) return prev;
            const next = { ...prev };
            delete next[group_id];
            return next;
          });
          setUnread((counts) => {
            if (!(group_id in counts)) return counts;
            const next = { ...counts };
            delete next[group_id];
            return next;
          });
          setActivePeerId((prev) => (prev === group_id ? null : prev));
          toast(t("toast.group_removed"), "error");
        })
      );
      const friendRequestUnlisten = await register(() =>
        onFriendRequest(({ peer_id, display_name }) => {
          if (disposed) return;
          // A new incoming request: add it to the Requests section and toast.
          // A duplicate push (the snapshot reply re-lists it) never adds twice.
          setFriendRequestsIncoming((prev) =>
            prev.some((request) => request.peer_id === peer_id)
              ? prev
              : [...prev, { peer_id, display_name }]
          );
          const name = display_name ?? shortPeerId(peer_id, 16);
          toast(t("contacts.request_received", { name }), "info");
        })
      );
      const contactAddedUnlisten = await register(() =>
        onContactAdded(({ peer_id, display_name }) => {
          if (disposed) return;
          // A peer became an accepted contact (my outgoing request was
          // accepted). Resync so the peer appears in the chat list and the
          // pending lists drop it.
          void loadFriendRequests();
          void refresh();
          const name = display_name ?? shortPeerId(peer_id, 16);
          toast(t("contacts.you_are_contacts", { name }), "success");
        })
      );
      const requestDeclinedUnlisten = await register(() =>
        onFriendRequestDeclined(({ peer_id }) => {
          if (disposed) return;
          setFriendRequestsOutgoing((prev) =>
            prev.filter((id) => id !== peer_id)
          );
          toast(
            t("contacts.request_declined", { name: shortPeerId(peer_id, 16) }),
            "info"
          );
        })
      );
      const contactRemovedUnlisten = await register(() =>
        onContactRemoved(({ peer_id }) => {
          if (disposed) return;
          // A contact relationship ended (either side removed it): drop the
          // peer from every local list and close the conversation if it was
          // open. `removeContact` itself stays optimistic — this push is the
          // single toast source so both directions report exactly once.
          setContacts((prev) => prev.filter((c) => c.peer_id !== peer_id));
          setMessages((prev) => {
            if (!(peer_id in prev)) return prev;
            const next = { ...prev };
            delete next[peer_id];
            return next;
          });
          setPresence((prev) => {
            if (!(peer_id in prev)) return prev;
            const next = { ...prev };
            delete next[peer_id];
            return next;
          });
          setFriendRequestsIncoming((prev) =>
            prev.filter((request) => request.peer_id !== peer_id)
          );
          setFriendRequestsOutgoing((prev) =>
            prev.filter((id) => id !== peer_id)
          );
          setUnread((counts) => {
            if (!(peer_id in counts)) return counts;
            const next = { ...counts };
            delete next[peer_id];
            return next;
          });
          setActivePeerId((prev) => (prev === peer_id ? null : prev));
          toast(t("contacts.contact_removed"), "info");
        })
      );
      if (
        disposed ||
        !chatUnlisten ||
        !statusUnlisten ||
        !reconnectingUnlisten ||
        !messageStatusUnlisten ||
        !typingUnlisten ||
        !reactionUnlisten ||
        !contactUpdatedUnlisten ||
        !presenceUnlisten ||
        !groupRemovedUnlisten ||
        !friendRequestUnlisten ||
        !contactAddedUnlisten ||
        !requestDeclinedUnlisten ||
        !contactRemovedUnlisten
      ) {
        return;
      }
      // connect() must settle before refresh() so the snapshot reflects the
      // established connection — offline messages included — and the UI is
      // consistent the moment it first renders.
      void (async () => {
        await connect();
        await refresh();
        await loadFriendRequests();
      })();
    };

    void setup();

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
    };
  }, [connect, refresh, loadFriendRequests]);

  const sendMessage = useCallback(
    async (peerId: string, text: string, quote?: QuoteInfo | null) => {
      const clientId = crypto.randomUUID();
      // Optimistic insertion; the backend echoes the same client id in the
      // `chat-message` event, which the dedup logic above ignores. The status
      // flips to "delivered" on the relay ack and "read" on a read receipt.
      setMessages((prev) => ({
        ...prev,
        [peerId]: [
          ...(prev[peerId] ?? []),
          {
            id: clientId,
            text,
            outgoing: true,
            timestamp: Date.now(),
            status: "sent",
            quote: quote ?? undefined,
          },
        ],
      }));
      try {
        await relaySendMessage(peerId, text, clientId, quote);
      } catch (err) {
        setMessages((prev) => ({
          ...prev,
          [peerId]: (prev[peerId] ?? []).filter(
            (m) => m.id !== clientId
          ),
        }));
        // The relay only routes messages between accepted contacts. A
        // `not_contacts` rejection means the peer is not (or no longer) a
        // friend: surface a clear translated toast instead of a raw relay code.
        // The `contact-removed` push (when the relationship really ended) then
        // drops the peer from the list, so the open chat is not yanked away here.
        if (relayErrorCode(err) === "not_contacts") {
          toast(t("contacts.not_in_contacts"), "error");
        } else {
          setConnectionError(String(err));
        }
      }
    },
    [toast, t]
  );

  /** React to a message (or un-react, per the caller-computed `active` state).
   *  The optimistic in-memory update mirrors what the peer applies on their
   *  side; the reaction envelope does not echo back to the sender, so no
   *  dedup against a `message-reaction` event is needed here. */
  const reactToMessage = useCallback(
    (peerId: string, messageId: string, emoji: string, active: boolean) => {
      if (!myPeerId) return;
      setMessages((prev) => {
        const list = prev[peerId];
        if (!list) return prev;
        const target = list.find((m) => m.id === messageId);
        if (!target) return prev;
        const reactions = active
          ? [
              ...(target.reactions ?? []).filter((r) => r.sender !== myPeerId),
              { sender: myPeerId, emoji },
            ]
          : (target.reactions ?? []).filter((r) => r.sender !== myPeerId);
        return {
          ...prev,
          [peerId]: list.map((m) =>
            m.id === messageId ? { ...m, reactions } : m
          ),
        };
      });
      void relaySendReaction(peerId, messageId, emoji, active).catch(() => {
        // Best-effort like receipts: a transient failure only skips the
        // remote pill for now; the local one stays consistent with the UI.
      });
    },
    [myPeerId]
  );

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

  /** Remove the accepted contact relationship with `peerId` on both sides
   *  (server-level). The Rust backend sends `remove_contact`, drops the local
   *  contact row, history and session, and the relay's `contact_removed` push
   *  (handled by the listener above) toasts and keeps the other end in sync.
   *  This keeps the React state in sync so no refresh is needed. */
  const removeContact = useCallback(async (targetPeerId: string) => {
    try {
      await relayRemoveContact(targetPeerId);
    } catch {
      // Best-effort: the in-memory removal below still applies for this
      // session, and the `contact-removed` push resyncs on the next connect.
    }
    setContacts((prev) => prev.filter((c) => c.peer_id !== targetPeerId));
    setMessages((prev) => {
      const next = { ...prev };
      delete next[targetPeerId];
      return next;
    });
    setPresence((prev) => {
      const next = { ...prev };
      delete next[targetPeerId];
      return next;
    });
    setFriendRequestsIncoming((prev) =>
      prev.filter((request) => request.peer_id !== targetPeerId)
    );
    setFriendRequestsOutgoing((prev) =>
      prev.filter((id) => id !== targetPeerId)
    );
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
  /**
   * Drop a group from every local list (contacts, groups, messages, unread)
   * and close the conversation if it was open. Shared by the leave flow, the
   * `group-removed` push and the access-verification fallback below.
   */
  const dropGroupLocally = useCallback((groupId: string) => {
    setContacts((prev) => prev.filter((c) => c.peer_id !== groupId));
    setGroups((prev) => prev.filter((g) => g.group_id !== groupId));
    setMessages((prev) => {
      if (!(groupId in prev)) return prev;
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

  const leaveGroup = useCallback(async (groupId: string) => {
    await relayLeaveGroup(groupId);
    dropGroupLocally(groupId);
  }, [dropGroupLocally]);

  /**
   * Verify this identity still has access to `groupId` before opening the
   * group info panel. When the relay answers `not_a_member`/`group_not_found`
   * (we left the group, or were removed, possibly while the event was missed),
   * the stale group is dropped locally so it stops "lingering" with every
   * action failing. Returns true only when access is confirmed.
   */
  const verifyGroupAccess = useCallback(
    async (groupId: string): Promise<boolean> => {
      try {
        await relayGetGroupInfo(groupId);
        return true;
      } catch (err) {
        const code = relayErrorCode(err);
        if (code === "not_a_member" || code === "group_not_found") {
          dropGroupLocally(groupId);
          toast(t("toast.group_removed"), "error");
          return false;
        }
        throw err;
      }
    },
    [dropGroupLocally, toast, t]
  );

  /** Send a friend request to `peerId`. The peer becomes an accepted contact
   *  once they accept; until then they stay in the Requests section and are
   *  not chatable. Failures (relay error codes such as `already_contacts`,
   *  `cannot_add_self`, `not_found`) propagate so the caller can show a
   *  translated message. */
  const sendFriendRequest = useCallback(
    async (peerIdToAdd: string) => {
      await relaySendFriendRequest(peerIdToAdd);
      await loadFriendRequests();
    },
    [loadFriendRequests]
  );

  /** Accept an incoming friend request: both sides become contacts. The relay
   *  replies with the fresh snapshot (which resyncs the Requests section) and
   *  pushes `friend_request_accepted` to the requester. */
  const acceptFriendRequest = useCallback(
    async (peerId: string) => {
      await relayAcceptFriendRequest(peerId);
      await loadFriendRequests();
    },
    [loadFriendRequests]
  );

  /** Decline an incoming friend request: it disappears from the Requests
   *  section and the requester receives a `friend_request_declined` push. */
  const declineFriendRequest = useCallback(
    async (peerId: string) => {
      await relayDeclineFriendRequest(peerId);
      await loadFriendRequests();
    },
    [loadFriendRequests]
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

  /** Clear every message and unread badge locally after the backend has wiped
   *  the store. Contacts, groups, presence and the active conversation stay —
   *  only the decrypted history disappears. */
  const clearHistory = useCallback(() => {
    setMessages({});
    setUnread({});
  }, []);

  return {
    contacts,
    friendRequestsIncoming,
    friendRequestsOutgoing,
    myDisplayName,
    myPeerId,
    myAvatarUrl,
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
    reactToMessage,
    sendTyping,
    saveDisplayName,
    removeContact,
    sendFriendRequest,
    acceptFriendRequest,
    declineFriendRequest,
    updatePresence,
    deleteMessage,
    leaveGroup,
  verifyGroupAccess,
    transferOwnership,
    clearHistory,
  };
}
