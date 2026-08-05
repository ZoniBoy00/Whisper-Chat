import { useState } from "react";
import type { Conversation } from "../types";
import { dummyConversations } from "../data/dummy";
import { Sidebar } from "./Sidebar";
import { ChatView } from "./ChatView";

interface MainViewProps {
  peerId: string;
  onReset: () => void;
}

export function MainView({ peerId, onReset }: MainViewProps) {
  const [conversations, setConversations] =
    useState<Conversation[]>(dummyConversations);
  const [activeId, setActiveId] = useState<string | null>(
    dummyConversations[0]?.id ?? null
  );

  const active = conversations.find((c) => c.id === activeId) ?? null;

  const handleSend = (text: string) => {
    if (!activeId) return;
    setConversations((prev) =>
      prev.map((c) =>
        c.id === activeId
          ? {
              ...c,
              messages: [
                ...c.messages,
                {
                  id: crypto.randomUUID(),
                  text,
                  outgoing: true,
                  timestamp: Date.now(),
                },
              ],
            }
          : c
      )
    );
  };

  return (
    <div className="flex h-screen overflow-hidden bg-wp-bg text-wp-text">
      <Sidebar
        peerId={peerId}
        conversations={conversations}
        activeId={activeId}
        onSelect={setActiveId}
        onReset={onReset}
      />
      <ChatView conversation={active} onSend={handleSend} />
    </div>
  );
}
