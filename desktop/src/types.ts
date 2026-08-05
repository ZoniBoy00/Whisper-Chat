/** Delivery state of an outgoing message. */
export type MessageStatus = "sent" | "delivered";

export interface Message {
  id: string;
  text: string;
  outgoing: boolean;
  timestamp: number;
  /** Outgoing only: "sent" by default, "delivered" once the relay acks. */
  status?: MessageStatus;
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

/** Payload of the `message-status` event emitted on a delivery ack. */
export interface MessageStatusEvent {
  client_id: string;
  status: MessageStatus;
}

/** Settings snapshot returned by the `get_settings` command. */
export interface AppSettings {
  relay_url?: string;
  theme?: "dark" | "light";
}
