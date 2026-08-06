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
  /** Quoted-reply context, when this message replies to an earlier one. */
  quote?: QuoteInfo;
  /** Emoji reactions attached to this message, in arrival order. */
  reactions?: ReactionInfo[];
}

/** The quoted message a reply refers to (snapshot sent inside the encrypted
 *  payload, so the reply renders even if the original is later deleted). */
export interface QuoteInfo {
  message_id: string;
  /** Snapshot of the quoted message's plaintext. */
  text: string;
  /** Peer ID of the quoted message's sender. */
  sender: string;
  /** Display name of the quoted sender, when known. */
  sender_name?: string | null;
}

/** One emoji reaction attached to a message by a peer. */
export interface ReactionInfo {
  sender: string;
  emoji: string;
}

/** Payload of the `message-reaction` event emitted when a reaction is applied.
 *  The UI toggles the pill under the affected bubble. */
export interface ReactionEvent {
  /** Conversation key: peer ID for 1:1 chats, group ID for groups. */
  peer_id: string;
  /** The id of the reacted-to message. */
  message_id: string;
  /** Peer ID of the reacting peer. */
  sender: string;
  /** The reaction emoji. */
  emoji: string;
  /** Whether the reaction was added (`true`) or removed (`false`). */
  active: boolean;
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
  /** Server avatar path ("/media/{hash}"); null when the group has no photo. */
  avatar_url?: string | null;
}

/** A known conversation peer plus the public profile data they advertise. */
export interface ContactInfo {
  peer_id: string;
  display_name: string | null;
  /** Registered public username; null when the peer has none. */
  username?: string | null;
  /** Server avatar path ("/media/{hash}"); null when the peer has none. */
  avatar_url?: string | null;
  /** Relationship status with this 1:1 peer: "accepted" (friends, chatable)
   *  or "pending" (a friend request is outstanding). Groups always report
   *  "accepted"; undefined while unknown. */
  status?: "accepted" | "pending";
}

/** One incoming friend request: the requester's peer ID plus the display name
 *  they advertise (null when unset). */
export interface FriendRequestIncoming {
  peer_id: string;
  display_name: string | null;
}

/** The full friend-request snapshot returned by `get_friend_requests`. */
export interface FriendRequests {
  /** Incoming requests (requester + display name), in arrival order. */
  incoming: FriendRequestIncoming[];
  /** Outgoing pending requests: peer IDs we asked who have not answered. */
  outgoing: string[];
}

/** Public profile returned by the `get_profile` command (and `search_users`). */
export interface ProfileInfo {
  username: string | null;
  peer_id: string;
  display_name: string | null;
  avatar_url: string | null;
}

/** Safety number info for one contact, returned by `get_safety_number`. */
export interface SafetyNumberInfo {
  /** The 60-digit grouped safety number shared with the peer. */
  safety_number: string;
  /** The compact 8-hex tag, for quick verbal comparison. */
  short: string;
  /** Whether we have marked this contact as verified. */
  verified: boolean;
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
  /** Incoming friend requests (requester + display name), in arrival order. */
  friend_requests_incoming: FriendRequestIncoming[];
  /** Outgoing pending friend requests: peer IDs we asked, unanswered. */
  friend_requests_outgoing: string[];
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

/** Payload of the `typing` event emitted when a peer starts/stops typing.
 *  `sender` is the composing member for GROUP chats (the `peer_id` is then
 *  the group id); it is `null` for 1:1 chats where the peer id identifies
 *  the writer. */
export interface TypingEvent {
  peer_id: string;
  is_typing: boolean;
  sender?: string | null;
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

/** Payload of the `group-removed` event emitted when the owner removes us from
 *  a group (online push). */
export interface GroupRemovedEvent {
  group_id: string;
}

/** Payload of the `friend-request` event emitted when a new incoming friend
 *  request arrives (UI toast + Requests section update). */
export interface FriendRequestEvent {
  peer_id: string;
  /** The requester's public display name; null when they have none. */
  display_name: string | null;
}

/** Payload of the `contact-added` event emitted when a peer becomes an
 *  accepted contact (my outgoing request was accepted, or I accepted
 *  someone's request). */
export interface ContactAddedEvent {
  peer_id: string;
  display_name?: string | null;
}

/** Payload of the `friend-request-declined` event emitted when my outgoing
 *  request was declined. */
export interface FriendRequestDeclinedEvent {
  peer_id: string;
}

/** Payload of the `contact-removed` event emitted when a contact relationship
 *  ends (either side removed it). */
export interface ContactRemovedEvent {
  peer_id: string;
}

/** One pending group invite as reported to the invitee. */
export interface GroupInviteInfo {
  group_id: string;
  group_name: string;
  inviter_peer_id: string;
}

/** Payload of the `group-invite-received` event (UI toast + Invites section). */
export interface GroupInviteReceivedEvent {
  group_id: string;
  group_name: string;
  inviter_peer_id: string;
}

/** Payload of the `group-invite-outcome` event (the inviter learns the result). */
export interface GroupInviteOutcomeEvent {
  group_id: string;
  peer_id: string;
  accepted: boolean;
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
  /** Whether closing the window hides it to the system tray instead of
   *  quitting (WhatsApp-style background chat). */
  minimize_to_tray?: boolean;
  /** Whether Enter sends a message. When off, Enter inserts a new line and
   *  Ctrl+Enter sends. */
  enter_to_send?: boolean;
  /** Message bubble font scale: "small", "normal" or "large". */
  message_font_scale?: string;
  /** Whether the app registers itself to launch at system startup. */
  autostart?: boolean;
  /** Whether automatic full-profile backups are enabled (written into
   *  `autobackup_dir` on a schedule). */
  autobackup_enabled?: boolean;
  /** Directory for automatic backups — typically a cloud-synced folder. */
  autobackup_dir?: string | null;
  /** How many recent automatic backups to keep before pruning. */
  autobackup_keep?: number;
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
