import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSettings,
  ChatMessageEvent,
  ChatState,
  ContactAddedEvent,
  ContactRemovedEvent,
  ContactUpdatedEvent,
  FriendRequestDeclinedEvent,
  FriendRequestEvent,
  FriendRequests,
  GroupInfo,
  GroupInviteInfo,
  GroupInviteOutcomeEvent,
  GroupInviteReceivedEvent,
  GroupRemovedEvent,
  LogEntry,
  MessageStatusEvent,
  PresenceEvent,
  PresenceInfo,
  ProfileInfo,
  QuoteInfo,
  ReactionEvent,
  ReconnectingEvent,
  RelayStatusEvent,
  SafetyNumberInfo,
  TypingEvent,
} from "../types";

/**
 * Default relay endpoint, used when no custom URL is persisted and
 * `WHISPER_RELAY_URL` is unset. Currently points at the test relay
 * (whisper-test.homelab.cfd) while the VPS E2EE test is running;
 * replace with the production relay before public release.
 */
export const DEFAULT_RELAY_URL = "wss://whisper-test.homelab.cfd/ws";

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

/** Open the daily client log folder in the OS file manager. */
export function openLogsFolder(): Promise<void> {
  return invoke("open_logs_folder");
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
 * `quote` turns the message into a quoted reply (the snapshot travels inside
 * the encrypted payload).
 */
export function sendMessage(
  peerId: string,
  text: string,
  clientId: string,
  quote?: QuoteInfo | null
): Promise<void> {
  return invoke("send_message", { peerId, text, clientId, quote });
}

export function sendMedia(peerId: string, path: string, clientId: string): Promise<void> {
  return invoke("send_media", { peerId, path, clientId });
}

export function pickMedia(): Promise<string | null> {
  return invoke("pick_media");
}

export function openMedia(path: string): Promise<void> {
  return invoke("open_media", { path });
}

/**
 * React to a message with an emoji. `active` is the sender's freshly computed
 * absolute state (true = react, false = unreact) and travels inside the
 * encrypted payload — no server changes involved.
 */
export function sendReaction(
  peerId: string,
  messageId: string,
  emoji: string,
  active: boolean
): Promise<void> {
  return invoke("send_reaction", { peerId, messageId, emoji, active });
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
  minimize_to_tray?: boolean;
  enter_to_send?: boolean;
  message_font_scale?: string;
  autostart?: boolean;
  autobackup_enabled?: boolean;
  /** `null` clears the backup folder; a string sets it. */
  autobackup_dir?: string | null;
  autobackup_keep?: number;
  /** Sets the backup password; an empty string clears it. Write-only: the
   *  password is never returned by `getSettings`. */
  autobackup_password?: string;
}

/** Persist a partial update of boolean preferences. */
export function updateSettings(patch: SettingsPatch): Promise<void> {
  return invoke("update_settings", { patch });
}

/** Remove the accepted contact relationship with `peerId` on both sides. The
 *  relay broadcasts a `contact_removed` push to both peers; the local contact
 *  row, history and presence are dropped immediately. */
export function removeContact(peerId: string): Promise<void> {
  return invoke("remove_contact", { peerId });
}

/** Send a friend request to `peerId`. The peer becomes an accepted contact
 *  once they accept. Rejects with a relay error code on failure
 *  (`already_pending`, `already_contacts`, `cannot_add_self`, `not_found`,
 *  `rate_limited`). */
export function sendFriendRequest(peerId: string): Promise<void> {
  return invoke("send_friend_request", { peerId });
}

/** Accept a pending incoming friend request from `peerId`. Both sides become
 *  accepted contacts and the requester receives a `friend_request_accepted`
 *  push. */
export function acceptFriendRequest(peerId: string): Promise<void> {
  return invoke("accept_friend_request", { peerId });
}

/** Decline a pending incoming friend request from `peerId`. */
export function declineFriendRequest(peerId: string): Promise<void> {
  return invoke("decline_friend_request", { peerId });
}

/** Fetch the pending friend-request snapshot (incoming + outgoing). */
export function getFriendRequests(): Promise<FriendRequests> {
  return invoke<FriendRequests>("get_friend_requests");
}

/**
 * Extract the relay error code from a Tauri invoke rejection. The Rust side
 * maps relay error codes to `"relay error: <code>"` strings (occasionally
 * prefixed with `Error: `), so this picks the `<code>` back out. Returns null
 * when the error is not a relay code.
 */
export function relayErrorCode(err: unknown): string | null {
  const match = String(err).match(/(?:Error:\s*)?relay error: ([a-z_]+)/i);
  if (!match) return null;
  const code = match[1].toLowerCase();
  return [
    "not_contacts",
    "already_pending",
    "already_contacts",
    "cannot_add_self",
    "not_found",
    "rate_limited",
  ].includes(code)
    ? code
    : null;
}

/** Delete one message locally ("delete for me"): history + encrypted store on
 *  this device only. The peer's copy and any relay-queued envelopes are
 *  untouched. */
export function deleteMessage(peerId: string, messageId: string): Promise<void> {
  return invoke("delete_message", { peerId, messageId });
}

/** Wipe the entire message history on this device. Contacts, sessions, groups
 *  and settings are kept — only the message history is cleared. */
export function clearChatHistory(): Promise<void> {
  return invoke("clear_chat_history");
}

/** Open a native save dialog and copy the identity file to the chosen
 *  location. Resolves with the destination path on success. */
export function exportIdentity(): Promise<string> {
  return invoke("export_identity");
}

/** Open a native pick dialog, validate and import an identity file over the
 *  current one. `password` is required when the picked file is a full
 *  encrypted backup; it is ignored for a bare identity file. The frontend
 *  must then call `reloadIdentity` and reload the webview for the restored
 *  identity to take effect. */
export function importIdentity(password?: string): Promise<string> {
  return invoke("import_identity", { password: password ?? null });
}

/** Export EVERYTHING — identity + the encrypted local database (history,
 *  sessions, contacts, settings) — as one password-encrypted JSON backup
 *  file (Argon2id → AES-256-GCM). When `password` is omitted the stored
 *  backup password is reused, so the user only ever enters it once. Resolves
 *  with the destination path. */
export function exportEverything(password?: string): Promise<string> {
  return invoke("export_everything", { password: password ?? null });
}

/** Import a Whisper backup from `exportEverything`; `password` unlocks the
 *  sealed package (a wrong password fails cleanly). The frontend must reload
 *  the webview afterwards for the restored profile to take effect. */
export function importEverything(password: string): Promise<string> {
  return invoke("import_everything", { password });
}

/** Open a native folder picker for the automatic-backup destination.
 *  Resolves with the chosen path. */
export function pickAutobackupDir(): Promise<string> {
  return invoke("pick_autobackup_dir");
}

/** Run an automatic backup right now using the persisted settings. Resolves
 *  with the written backup path. */
export function runAutobackupNow(): Promise<string> {
  return invoke("run_autobackup_now");
}

/** Drop the cached identity so the next connect reloads it from disk. */
export function reloadIdentity(): Promise<void> {
  return invoke("reload_identity");
}

/** Enable or disable launching Whisper at system startup (OS-level). The
 *  preference itself is persisted through `updateSettings`. */
export function setAutostart(enabled: boolean): Promise<void> {
  return invoke("set_autostart", { enabled });
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

/** Mark a conversation as read end-to-end once its messages are visible on
 *  screen: a 1:1 receipt, or a group read receipt for `messageId`. */
export function sendReadReceipt(
  peerId: string,
  messageId?: string | null
): Promise<void> {
  return invoke("send_read_receipt", { peerId, messageId });
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

/** Add a peer to a group's roster after creation (owner or admin only). Every
 *  existing member shares its own Megolm session key to the new member over a
 *  1:1 encrypted channel. */
export function addGroupMember(groupId: string, peerId: string): Promise<void> {
  return invoke("add_group_member", { groupId, peerId });
}

/**
 * Set a group's avatar image. `avatarBase64` is raw base64 WITHOUT the
 * `data:image/...;base64,` prefix — the backend stores the bytes under
 * /media/{hash} and the group's avatar_url starts pointing there. Owner or
 * admin only.
 */
export function setGroupAvatar(
  groupId: string,
  avatarBase64: string
): Promise<void> {
  return invoke("set_group_avatar", { groupId, avatar: avatarBase64 });
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

/** Rename a group (owner/admin only). */
export function renameGroup(groupId: string, name: string): Promise<void> {
  return invoke("rename_group", { groupId, name });
}

/** Remove the caller from a group's roster. */
export function leaveGroup(groupId: string): Promise<void> {
  return invoke("leave_group", { groupId });
}

/** Invite `peerId` to `groupId` (owner/admin only). The invitee accepts or
 *  declines; they join the roster only on accept. */
export function sendGroupInvite(
  groupId: string,
  peerId: string
): Promise<void> {
  return invoke("send_group_invite", { groupId, peerId });
}

/** Accept a pending invite to `groupId`. */
export function acceptGroupInvite(groupId: string): Promise<void> {
  return invoke("accept_group_invite", { groupId });
}

/** Decline a pending invite to `groupId`. */
export function declineGroupInvite(groupId: string): Promise<void> {
  return invoke("decline_group_invite", { groupId });
}

/** Fetch the pending group invites for this identity. */
export function getGroupInvites(): Promise<GroupInviteInfo[]> {
  return invoke<GroupInviteInfo[]>("get_group_invites");
}

/** Get (or create) the group's shareable join link (`whisper://join?..`).
 *  Any member may ask; anyone with the link can join. */
export function getGroupJoinLink(groupId: string): Promise<string> {
  return invoke<string>("get_group_join_link", { groupId });
}

/** Join a group via its shareable join link. */
export function joinGroupByLink(
  groupId: string,
  token: string
): Promise<void> {
  return invoke("join_group_by_link", { groupId, token });
}

/** Subscribe to new group invites. Returns an unlisten function. */
export function onGroupInviteReceived(
  handler: (event: GroupInviteReceivedEvent) => void
): Promise<UnlistenFn> {
  return listen<GroupInviteReceivedEvent>("group-invite-received", (event) =>
    handler(event.payload)
  );
}

/** Subscribe to invite outcomes (the inviter learns accept/decline). */
export function onGroupInviteOutcome(
  handler: (event: GroupInviteOutcomeEvent) => void
): Promise<UnlistenFn> {
  return listen<GroupInviteOutcomeEvent>("group-invite-outcome", (event) =>
    handler(event.payload)
  );
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

/** Payload of the `chat-message-deleted` event (disappearing messages that
 *  expired and were purged by the backend). */
export interface ChatMessageDeletedEvent {
  peer_id: string;
  message_ids: string[];
}

/** Subscribe to disappearing-message expirations. Returns an unlisten fn. */
export function onMessageDeleted(
  handler: (event: ChatMessageDeletedEvent) => void
): Promise<UnlistenFn> {
  return listen<ChatMessageDeletedEvent>("chat-message-deleted", (event) =>
    handler(event.payload)
  );
}

/** Payload of the `chat-message-edited` event (a message's text was replaced
 *  on every device). */
export interface ChatMessageEditedEvent {
  peer_id: string;
  message_id: string;
  text: string;
}

/** Subscribe to message edits. Returns an unlisten function. */
export function onMessageEdited(
  handler: (event: ChatMessageEditedEvent) => void
): Promise<UnlistenFn> {
  return listen<ChatMessageEditedEvent>("chat-message-edited", (event) =>
    handler(event.payload)
  );
}

/** Payload of the `contacts-rehydrated` event: the contact list was merged
 *  back from the relay (e.g. after a database reset). */
export interface ContactsRehydratedEvent {
  peer_ids: string[];
}

/** Subscribe to contact-list rehydration. Returns an unlisten function. */
export function onContactsRehydrated(
  handler: (event: ContactsRehydratedEvent) => void
): Promise<UnlistenFn> {
  return listen<ContactsRehydratedEvent>("contacts-rehydrated", (event) =>
    handler(event.payload)
  );
}

/** Edit one of our own messages: replace its text on every device. */
export function editMessageCommand(
  peerId: string,
  messageId: string,
  newText: string
): Promise<void> {
  return invoke("edit_message", { peerId, messageId, newText });
}

/** Delete one of our own messages on every device. */
export function deleteForEveryoneCommand(
  peerId: string,
  messageId: string
): Promise<void> {
  return invoke("delete_for_everyone", { peerId, messageId });
}

/** Set (or clear, with 0) the disappearing-message timer for a chat. */
export function setChatExpirationCommand(
  peerId: string,
  seconds: number
): Promise<void> {
  return invoke("set_chat_expiration", { peerId, seconds });
}

/** Payload of the `message-read-by` event (a member read our group message). */
export interface MessageReadByEvent {
  group_id: string;
  message_id: string;
  read_by_count: number;
}

/** Subscribe to group read receipts. Returns an unlisten function. */
export function onMessageReadBy(
  handler: (event: MessageReadByEvent) => void
): Promise<UnlistenFn> {
  return listen<MessageReadByEvent>("message-read-by", (event) =>
    handler(event.payload)
  );
}

/** Subscribe to emoji reaction updates. Returns an unlisten function. */
export function onMessageReaction(
  handler: (event: ReactionEvent) => void
): Promise<UnlistenFn> {
  return listen<ReactionEvent>("message-reaction", (event) =>
    handler(event.payload)
  );
}

/** Build a `whisper://invite` link for our own identity (with profile hints
 *  when a display name / username is registered). */
export function getInviteLink(): Promise<string> {
  return invoke<string>("get_invite_link");
}

/** Compute the safety number shared with `peerId` plus our verification
 *  state. Rejects until the peer's identity key has been learned. */
export function getSafetyNumber(peerId: string): Promise<SafetyNumberInfo> {
  return invoke<SafetyNumberInfo>("get_safety_number", { peerId });
}

/** Set (or clear) the locally-stored verified flag for a contact. */
export function markContactVerified(
  peerId: string,
  verified: boolean
): Promise<void> {
  return invoke("mark_contact_verified", { peerId, verified });
}

/** Drain deep links that arrived before the webview was ready (app launched
 *  by clicking a `whisper://` link). Call once on startup. */
export function getPendingDeepLink(): Promise<string[]> {
  return invoke<string[]>("take_pending_deep_link");
}

/** Subscribe to live `whisper://` deep links (second instance or OS open
 *  while running). Returns an unlisten function. */
export function onDeepLink(
  handler: (url: string) => void
): Promise<UnlistenFn> {
  return listen<string>("deep-link", (event) => handler(event.payload));
}

/** Subscribe to contact display-name updates. Returns an unlisten fn. */
export function onContactUpdated(
  handler: (event: ContactUpdatedEvent) => void
): Promise<UnlistenFn> {
  return listen<ContactUpdatedEvent>("contact-updated", (event) =>
    handler(event.payload)
  );
}

/** Subscribe to `group-removed` pushes (the owner removed us from a group).
 *  Returns an unlisten function. */
export function onGroupRemoved(
  handler: (event: GroupRemovedEvent) => void
): Promise<UnlistenFn> {
  return listen<GroupRemovedEvent>("group-removed", (event) =>
    handler(event.payload)
  );
}

/** Subscribe to group roster changes (member count/roles) so the chat list
 *  stays current without a full refresh. Returns an unlisten function. */
export function onGroupUpdated(
  handler: (event: { group_id: string }) => void
): Promise<UnlistenFn> {
  return listen<{ group_id: string }>("group-updated", (event) =>
    handler(event.payload)
  );
}

/** Subscribe to new incoming friend requests. Returns an unlisten function. */
export function onFriendRequest(
  handler: (event: FriendRequestEvent) => void
): Promise<UnlistenFn> {
  return listen<FriendRequestEvent>("friend-request", (event) =>
    handler(event.payload)
  );
}

/** Subscribe to `contact-added` pushes (a peer became an accepted contact).
 *  Returns an unlisten function. */
export function onContactAdded(
  handler: (event: ContactAddedEvent) => void
): Promise<UnlistenFn> {
  return listen<ContactAddedEvent>("contact-added", (event) =>
    handler(event.payload)
  );
}

/** Subscribe to `friend-request-declined` pushes (my outgoing request was
 *  declined). Returns an unlisten function. */
export function onFriendRequestDeclined(
  handler: (event: FriendRequestDeclinedEvent) => void
): Promise<UnlistenFn> {
  return listen<FriendRequestDeclinedEvent>("friend-request-declined", (event) =>
    handler(event.payload)
  );
}

/** Subscribe to `contact-removed` pushes (a contact relationship ended on
 *  either side). Returns an unlisten function. */
export function onContactRemoved(
  handler: (event: ContactRemovedEvent) => void
): Promise<UnlistenFn> {
  return listen<ContactRemovedEvent>("contact-removed", (event) =>
    handler(event.payload)
  );
}
