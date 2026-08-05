import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSettings,
  ChatMessageEvent,
  ChatState,
  ContactUpdatedEvent,
  GroupInfo,
  LogEntry,
  MessageStatusEvent,
  PresenceEvent,
  PresenceInfo,
  ProfileInfo,
  ReconnectingEvent,
  RelayStatusEvent,
  TypingEvent,
} from "../types";

/** Connect to the relay and authenticate with the stored identity. */
export function connectRelay(): Promise<void> {
  return invoke("connect_relay");
}

/** The most recent client log lines from the in-process ring buffer. `limit`
 *  caps the number of lines; the newest lines are returned. */
export function getClientLogs(limit?: number): Promise<LogEntry[]> {
  return invoke("get_client_logs", { limit });
}

/** Append a log line forwarded from the webview (e.g. an uncaught JS error)
 *  so the Logs settings tab shows frontend failures next to the Rust logs. */
export function appendClientLog(level: string, message: string): Promise<void> {
  return invoke("append_client_log", { level, message });
}

/** Generate and publish a fresh batch of one-time pre-keys. */
export function publishPrekeys(): Promise<void> {
  return invoke("publish_prekeys");
}

/** Establish an encrypted session with a peer and send the first message. */
export function startChat(peerId: string): Promise<void> {
  return invoke("start_chat", { peerId });
}

/**
 * Encrypt and send a message. `clientId` is echoed back inside the
 * `chat-message` event so the UI can deduplicate its optimistic insertion.
 */
export function sendMessage(
  peerId: string,
  text: string,
  clientId: string
): Promise<void> {
  return invoke("send_message", { peerId, text, clientId });
}

/** Snapshot of identity, connection, contacts and messages. */
export function getChatState(): Promise<ChatState> {
  return invoke("get_chat_state");
}

/** Persisted app settings (relay URL, theme). */
export function getSettings(): Promise<AppSettings> {
  return invoke("get_settings");
}

/** Persist a relay URL. Reconnect afterwards for it to take effect. */
export function setRelayUrl(url: string): Promise<void> {
  return invoke("set_relay_url", { url });
}

/** Persist the theme choice. */
export function setTheme(theme: "dark" | "light"): Promise<void> {
  return invoke("set_theme", { theme });
}

/** Toggle whether our online status and last-seen are shown to other peers.
 *  Persisted locally and pushed to the relay so it takes effect immediately. */
export function setPrivacy(presenceVisible: boolean): Promise<void> {
  return invoke("set_privacy", { presenceVisible });
}

/** Boolean preferences a settings tab can toggle without a dedicated command
 *  each: read receipts, typing indicator and desktop notification options.
 *  `language` carries the UI language (string, not boolean). */
export interface SettingsPatch {
  read_receipts?: boolean;
  typing_indicator?: boolean;
  notifications_enabled?: boolean;
  notification_preview?: boolean;
  notification_sound?: boolean;
  language?: string;
}

/** Persist a partial update of boolean preferences. */
export function updateSettings(patch: SettingsPatch): Promise<void> {
  return invoke("update_settings", { patch });
}

/** Remove a contact and its message history on this device (client-local). */
export function removeContact(peerId: string): Promise<void> {
  return invoke("remove_contact", { peerId });
}

/** Delete one message locally ("delete for me"): history + encrypted store on
 *  this device only. The peer's copy and any relay-queued envelopes are
 *  untouched. */
export function deleteMessage(peerId: string, messageId: string): Promise<void> {
  return invoke("delete_message", { peerId, messageId });
}

/** Persist our own display name and announce it to the relay. */
export function setDisplayName(name: string): Promise<void> {
  return invoke("set_display_name", { name });
}

/**
 * Register a public username for our identity. The Ed25519 signature over the
 * username is produced on the Rust side and attached to the request.
 * Resolves with the registered username on success.
 */
export function registerProfile(username: string): Promise<string> {
  return invoke<string>("register_profile", { username });
}

/**
 * Search the directory for users by username or Whisper ID. Returns a list of
 * public profiles; empty when nothing matches.
 */
export function searchUsers(
  query: string,
  limit = 10
): Promise<ProfileInfo[]> {
  return invoke("search_users", { query, limit });
}

/** Fetch a peer's public profile by Whisper ID. Rejects with `no_profile`
 *  when the peer has not registered one. */
export function getProfile(peerId: string): Promise<ProfileInfo> {
  return invoke("get_profile", { peerId });
}

/**
 * Upload our avatar image. `username` must already be registered (the relay
 * re-registers the profile with the avatar). `avatarBase64` is raw base64
 * WITHOUT the "data:image/...;base64," prefix — the backend stores the bytes
 * under /media/{hash} and the profile's avatar_url starts pointing there.
 */
export function setAvatar(username: string, avatarBase64: string): Promise<void> {
  return invoke("set_avatar", { username, avatar: avatarBase64 });
}

/** Send an end-to-end typing indicator to a peer (encrypted in-session). */
export function sendTyping(peerId: string, isTyping: boolean): Promise<void> {
  return invoke("send_typing", { peerId, isTyping });
}

/** Fetch a peer's current presence (online status + last-seen timestamp). */
export function getPresence(peerId: string): Promise<PresenceInfo> {
  return invoke("get_presence", { peerId });
}

/** Subscribe to presence pushes for a peer. Call again after reconnecting. */
export function watchPresence(peerId: string): Promise<void> {
  return invoke("watch_presence", { peerId });
}

/** Subscribe to presence updates (pushes and get_presence replies). */
export function onPresence(
  handler: (event: PresenceEvent) => void
): Promise<UnlistenFn> {
  return listen<PresenceEvent>("presence", (event) => handler(event.payload));
}

/**
 * Create a group: registers it on the relay, adds `memberIds` to the roster,
 * builds the Megolm outbound session and shares its session key to every
 * member over the existing 1:1 encrypted sessions. Resolves with the
 * relay-assigned group ID.
 */
export function createGroup(name: string, memberIds: string[]): Promise<string> {
  return invoke<string>("create_group", { name, memberIds });
}

/** Fetch a group's public metadata and member roster (with roles). */
export function getGroupInfo(groupId: string): Promise<GroupInfo> {
  return invoke("get_group_info", { groupId });
}

/** Promote a member to group admin (owner or admin only). */
export function promoteMember(groupId: string, peerId: string): Promise<void> {
  return invoke("promote_member", { groupId, peerId });
}

/** Demote an admin back to a regular member (owner only). */
export function demoteMember(groupId: string, peerId: string): Promise<void> {
  return invoke("demote_member", { groupId, peerId });
}

/** Remove a member from a group (owner only). */
export function removeMember(groupId: string, peerId: string): Promise<void> {
  return invoke("remove_member", { groupId, peerId });
}

/** Transfer group ownership to `peerId` (owner only). The old owner becomes
 *  an admin; `peerId` takes over the owner role. */
export function transferOwnership(groupId: string, peerId: string): Promise<void> {
  return invoke("transfer_ownership", { groupId, peerId });
}

/** Remove the caller from a group's roster. */
export function leaveGroup(groupId: string): Promise<void> {
  return invoke("leave_group", { groupId });
}

/** Close the relay connection (used when resetting the identity). */
export function disconnectRelay(): Promise<void> {
  return invoke("disconnect_relay");
}

/** Close the connection and wipe all in-memory chat state. */
export function resetRelay(): Promise<void> {
  return invoke("reset_relay");
}

/** Subscribe to newly decrypted messages. Returns an unlisten function. */
export function onChatMessage(
  handler: (event: ChatMessageEvent) => void
): Promise<UnlistenFn> {
  return listen<ChatMessageEvent>("chat-message", (event) =>
    handler(event.payload)
  );
}

/** Subscribe to relay connection status changes. Returns an unlisten fn. */
export function onRelayStatus(
  handler: (event: RelayStatusEvent) => void
): Promise<UnlistenFn> {
  return listen<RelayStatusEvent>("relay-status", (event) =>
    handler(event.payload)
  );
}

/** Subscribe to auto-reconnect progress. Returns an unlisten function. */
export function onReconnecting(
  handler: (event: ReconnectingEvent) => void
): Promise<UnlistenFn> {
  return listen<ReconnectingEvent>("reconnecting", (event) =>
    handler(event.payload)
  );
}

/** Subscribe to delivery acknowledgements. Returns an unlisten function. */
export function onMessageStatus(
  handler: (event: MessageStatusEvent) => void
): Promise<UnlistenFn> {
  return listen<MessageStatusEvent>("message-status", (event) =>
    handler(event.payload)
  );
}

/** Subscribe to typing indicators. Returns an unlisten function. */
export function onTyping(
  handler: (event: TypingEvent) => void
): Promise<UnlistenFn> {
  return listen<TypingEvent>("typing", (event) => handler(event.payload));
}

/** Subscribe to contact display-name updates. Returns an unlisten fn. */
export function onContactUpdated(
  handler: (event: ContactUpdatedEvent) => void
): Promise<UnlistenFn> {
  return listen<ContactUpdatedEvent>("contact-updated", (event) =>
    handler(event.payload)
  );
}
