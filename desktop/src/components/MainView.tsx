import { useCallback, useEffect, useMemo, useState } from "react";
import type { ContactInfo, Conversation, Message, MessageStatus } from "../types";
import {
  connectRelay,
  getChatState,
  getSettings,
  onChatMessage,
  onContactUpdated,
  onMessageStatus,
  onRelayStatus,
  onTyping,
  publishPrekeys,
  resetRelay,
  sendMessage,
  sendTyping,
  setDisplayName as persistDisplayName,
  setRelayUrl as persistRelayUrl,
  setTheme as persistTheme,
  startChat,
} from "../lib/relay";
import { shortPeerId } from "../lib/format";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { Sidebar } from "./Sidebar";
import { ChatView } from "./ChatView";
import { AddContactDialog } from "./AddContactDialog";
import { SettingsDialog } from "./SettingsDialog";

type Theme = "dark" | "light";

interface MainViewProps {
  peerId: string;
  onReset: () => void;
}

export function MainView({ peerId, onReset }: MainViewProps) {
  const [contacts, setContacts] = useState<ContactInfo[]>([]);
  const [myDisplayName, setMyDisplayName] = useState<string | null>(null);
  const [messages, setMessages] = useState<Record<string, Message[]>>({});
  const [connected, setConnected] = useState(false);
  const [connecting, setConnecting] = useState(true);
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [activePeerId, setActivePeerId] = useState<string | null>(null);
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [theme, setTheme] = useState<Theme>("dark");
  const [relayUrl, setRelayUrl] = useState("");
  // Per-peer typing state fed by the `typing` event (with a 5s auto-timeout
  // on the backend, so it can never get stuck on "on").
  const [typing, setTyping] = useState<Record<string, boolean>>({});

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
      setConnected(state.connected);
      setMyDisplayName(state.my_display_name);
    } catch {
      // Transient failure; event listeners resync the next state change.
    }
  }, []);

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
      if (
        disposed ||
        !chatUnlisten ||
        !statusUnlisten ||
        !messageStatusUnlisten ||
        !typingUnlisten ||
        !contactUpdatedUnlisten
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

  const handleAddContact = useCallback(
    async (peerIdToAdd: string) => {
      try {
        await startChat(peerIdToAdd);
        setContacts((prev) =>
          prev.some((c) => c.peer_id === peerIdToAdd)
            ? prev
            : [...prev, { peer_id: peerIdToAdd, display_name: null }]
        );
        setActivePeerId(peerIdToAdd);
        // Pull the freshly-fetched display name for the new contact.
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
  // behaves like Signal/WhatsApp: most recent activity first.
  const conversations: Conversation[] = useMemo(
    () =>
      contacts
        .map((contact) => ({
          id: contact.peer_id,
          name:
            contact.display_name ?? shortPeerId(contact.peer_id),
          displayName: contact.display_name,
          peerId: contact.peer_id,
          messages: messages[contact.peer_id] ?? [],
        }))
        .sort((a, b) => {
          const lastA = a.messages[a.messages.length - 1]?.timestamp ?? 0;
          const lastB = b.messages[b.messages.length - 1]?.timestamp ?? 0;
          return lastB - lastA;
        }),
    [contacts, messages]
  );

  const active =
    conversations.find((c) => c.peerId === activePeerId) ?? null;

  return (
    <div className="flex h-screen overflow-hidden bg-wp-bg text-wp-text">
      <Sidebar
        peerId={peerId}
        myDisplayName={myDisplayName}
        conversations={conversations}
        activeId={activePeerId}
        connected={connected}
        connecting={connecting}
        connectionError={connectionError}
        onSelect={setActivePeerId}
        onAddContact={() => setAddDialogOpen(true)}
        onOpenSettings={() => setSettingsOpen(true)}
        onReconnect={() => void connect()}
        onReset={handleReset}
      />
      <ChatView
        conversation={active}
        isTyping={active ? typing[active.peerId] ?? false : false}
        onSend={(t) => void handleSend(t)}
        onTypingChange={handleTypingChange}
      />
      <AddContactDialog
        open={addDialogOpen}
        onOpenChange={setAddDialogOpen}
        onAdd={handleAddContact}
      />
      <SettingsDialog
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        peerId={peerId}
        myDisplayName={myDisplayName}
        theme={theme}
        onThemeChange={handleThemeChange}
        relayUrl={relayUrl}
        onSaveRelayUrl={handleRelayUrlSave}
        onSaveDisplayName={handleSaveDisplayName}
        onReset={handleReset}
      />
    </div>
  );
}
