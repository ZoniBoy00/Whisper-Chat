/** The app's theme palette. */
export type Theme = "dark" | "light";

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
  /** True for group chats (keyed by a relay-assigned group ID). */
  isGroup?: boolean;
  /** Group member count; used by the group header subtitle. */
  memberCount?: number;
}

/** A group role: owner (creator), admin or a regular member. */
export type GroupRole = "owner" | "admin" | "member";

/** One member of a group roster with its current role. */
export interface GroupMember {
  peer_id: string;
  role: GroupRole;
}

/** A group's public metadata returned by `get_chat_state` / `get_group_info`. */
export interface GroupInfo {
  group_id: string;
  name: string;
  owner_peer_id: string;
  members: GroupMember[];
  /** Our own role in the group; null while unknown. */
  my_role: GroupRole | null;
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
  /** Our own registered public username; null when unset. */
  my_username: string | null;
  /** Our own avatar path ("/media/{hash}"); null when unset. */
  my_avatar_url: string | null;
  connected: boolean;
  contacts: ContactInfo[];
  messages: Record<string, Message[]>;
  presence: Record<string, PresenceInfo>;
  /** Groups this identity belongs to, with their rosters and roles. */
  groups: GroupInfo[];
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

/** Payload of the `reconnecting` event emitted while the client retries a
 *  dropped connection with exponential backoff. `active: false` ends the
 *  reconnecting state — the connection was re-established, or a manual
 *  disconnect/reset cancelled the retries. */
export interface ReconnectingEvent {
  active: boolean;
  /** One-based retry attempt currently scheduled (or in flight). */
  attempt: number;
  /** Milliseconds until the next `connect()` attempt. */
  next_in_ms: number;
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
 *  name or avatar is learned or refreshed. */
export interface ContactUpdatedEvent {
  peer_id: string;
  display_name: string | null;
  /** Server avatar path ("/media/{hash}"); null when unknown/unchanged. */
  avatar_url?: string | null;
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
  theme?: Theme;
  /** Whether our online status and last-seen are shown to others. */
  presence_visible?: boolean;
  /** Whether we send end-to-end read receipts to the sender. */
  read_receipts?: boolean;
  /** Whether we broadcast typing indicators to the active peer. */
  typing_indicator?: boolean;
  /** Whether desktop notifications are shown for messages while unfocused. */
  notifications_enabled?: boolean;
  /** Whether notifications include the message text. */
  notification_preview?: boolean;
  /** Whether a short chime plays for incoming messages. */
  notification_sound?: boolean;
  /** The UI language ("en" or "fi"). */
  language?: string;
}

/** One captured client log line, returned by the `get_client_logs` command. */
export interface LogEntry {
  /** Epoch milliseconds when the line was written. */
  timestamp: number;
  /** Uppercase level: TRACE, DEBUG, INFO, WARN or ERROR. */
  level: string;
  /** The Rust module path (or "webview") that produced the line. */
  target: string;
  /** The formatted message. */
  message: string;
}
