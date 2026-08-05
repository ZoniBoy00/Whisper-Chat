export interface Message {
  id: string;
  text: string;
  outgoing: boolean;
  timestamp: number;
}

export interface Conversation {
  id: string;
  name: string;
  peerId: string;
  messages: Message[];
}

/** Snapshot returned by the `get_chat_state` command. */
export interface ChatState {
  my_peer_id: string;
  connected: boolean;
  contacts: string[];
  messages: Record<string, Message[]>;
}

/** Payload of the `chat-message` event emitted for new plaintext. */
export interface ChatMessageEvent {
  peer_id: string;
  message: Message;
}

/** Payload of the `relay-status` event emitted on connect/disconnect. */
export interface RelayStatusEvent {
  connected: boolean;
}
