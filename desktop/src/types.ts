/** Delivery state of an outgoing message. */
export type MessageStatus = "sent" | "delivered" | "read";

export interface Message {
  id: string;
  text: string;
  outgoing: boolean;
  timestamp: number;
  /** Outgoing only: "sent" by default, "delivered" once the relay acks,
   *  "read" once the peer's client decrypts an end-to-end read receipt. */
  status?: MessageStatus;
}

export interface Conversation {
  id: string;
  /** Display name, falling back to a shortened peer ID. */
  name: string;
  /** The peer's advertised display name; null when unset. */
  displayName: string | null;
  peerId: string;
  messages: Message[];
  /** Registered public username (e.g. "@alice"); null when not known. */
  username?: string | null;
  /** Server avatar path ("/media/{hash}"); null when not known. */
  avatarUrl?: string | null;
}

/** A known conversation peer plus the public profile data they advertise. */
export interface ContactInfo {
  peer_id: string;
  display_name: string | null;
  /** Registered public username; null when the peer has none. */
  username?: string | null;
  /** Server avatar path ("/media/{hash}"); null when the peer has none. */
  avatar_url?: string | null;
}

/** Public profile returned by the `get_profile` command (and `search_users`). */
export interface ProfileInfo {
  username: string | null;
  peer_id: string;
  display_name: string | null;
  avatar_url: string | null;
}

/** A peer's presence snapshot: online right now, plus last-seen unix seconds
 *  (null while online or when never seen). */
export interface PresenceInfo {
  online: boolean;
  last_seen: number | null;
}

/** Snapshot returned by the `get_chat_state` command. */
export interface ChatState {
  my_peer_id: string;
  my_display_name: string | null;
  connected: boolean;
  contacts: ContactInfo[];
  messages: Record<string, Message[]>;
  presence: Record<string, PresenceInfo>;
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

/** Payload of the `message-status` event emitted on a delivery or read update. */
export interface MessageStatusEvent {
  client_id: string;
  status: MessageStatus;
}

/** Payload of the `typing` event emitted when a peer starts/stops typing. */
export interface TypingEvent {
  peer_id: string;
  is_typing: boolean;
}

/** Payload of the `contact-updated` event emitted when a contact's display
 *  name is learned or refreshed. */
export interface ContactUpdatedEvent {
  peer_id: string;
  display_name: string | null;
}

/** Payload of the `presence` event emitted on a presence push or reply. */
export interface PresenceEvent {
  peer_id: string;
  online: boolean;
  last_seen: number | null;
}

/** Settings snapshot returned by the `get_settings` command. */
export interface AppSettings {
  relay_url?: string;
  theme?: "dark" | "light";
}
