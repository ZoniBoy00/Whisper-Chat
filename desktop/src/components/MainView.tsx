import { useCallback, useEffect, useMemo, useState } from "react";
import type { Conversation, Message } from "../types";
import {
  connectRelay,
  getChatState,
  onChatMessage,
  onRelayStatus,
  publishPrekeys,
  resetRelay,
  sendMessage,
  startChat,
} from "../lib/relay";
import { shortPeerId } from "../lib/format";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { Sidebar } from "./Sidebar";
import { ChatView } from "./ChatView";
import { AddContactDialog } from "./AddContactDialog";

interface MainViewProps {
  peerId: string;
  onReset: () => void;
}

export function MainView({ peerId, onReset }: MainViewProps) {
  const [contacts, setContacts] = useState<string[]>([]);
  const [messages, setMessages] = useState<Record<string, Message[]>>({});
  const [connected, setConnected] = useState(false);
  const [connecting, setConnecting] = useState(true);
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [activePeerId, setActivePeerId] = useState<string | null>(null);
  const [addDialogOpen, setAddDialogOpen] = useState(false);

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
    } catch {
      // Transient failure; event listeners resync the next state change.
    }
  }, []);

  useEffect(() => {
    void connect();
    void refresh();

    const unlisteners: UnlistenFn[] = [];
    let disposed = false;

    const setup = async () => {
      unlisteners.push(
        await onChatMessage(({ peer_id, message }) => {
          if (disposed) return;
          setContacts((prev) =>
            prev.includes(peer_id) ? prev : [...prev, peer_id]
          );
          setMessages((prev) => {
            const list = prev[peer_id] ?? [];
            if (list.some((m) => m.id === message.id)) return prev;
            return { ...prev, [peer_id]: [...list, message] };
          });
          setActivePeerId((prev) => prev ?? peer_id);
        })
      );
      unlisteners.push(
        await onRelayStatus(({ connected: status }) => {
          if (disposed) return;
          setConnected(status);
          if (status) void refresh();
        })
      );
    };
    void setup();

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
    };
  }, [connect, refresh]);

  const handleSend = useCallback(
    async (text: string) => {
      if (!activePeerId) return;
      const clientId = crypto.randomUUID();
      // Optimistic insertion; the backend echoes the same client id in the
      // `chat-message` event, which the dedup logic above ignores.
      setMessages((prev) => ({
        ...prev,
        [activePeerId]: [
          ...(prev[activePeerId] ?? []),
          { id: clientId, text, outgoing: true, timestamp: Date.now() },
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

  const handleAddContact = useCallback(async (peerIdToAdd: string) => {
    try {
      await startChat(peerIdToAdd);
      setContacts((prev) =>
        prev.includes(peerIdToAdd) ? prev : [...prev, peerIdToAdd]
      );
      setActivePeerId(peerIdToAdd);
    } catch (err) {
      throw new Error(String(err));
    }
  }, []);

  const handleReset = useCallback(() => {
    void resetRelay();
    onReset();
  }, [onReset]);

  const conversations: Conversation[] = useMemo(
    () =>
      contacts.map((id) => ({
        id,
        name: shortPeerId(id),
        peerId: id,
        messages: messages[id] ?? [],
      })),
    [contacts, messages]
  );

  const active =
    conversations.find((c) => c.peerId === activePeerId) ?? null;

  return (
    <div className="flex h-screen overflow-hidden bg-wp-bg text-wp-text">
      <Sidebar
        peerId={peerId}
        conversations={conversations}
        activeId={activePeerId}
        connected={connected}
        connecting={connecting}
        connectionError={connectionError}
        onSelect={setActivePeerId}
        onAddContact={() => setAddDialogOpen(true)}
        onReconnect={() => void connect()}
        onReset={handleReset}
      />
      <ChatView conversation={active} onSend={(t) => void handleSend(t)} />
      <AddContactDialog
        open={addDialogOpen}
        onOpenChange={setAddDialogOpen}
        onAdd={handleAddContact}
      />
    </div>
  );
}
