import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSettings,
  ChatMessageEvent,
  ChatState,
  ContactUpdatedEvent,
  MessageStatusEvent,
  PresenceEvent,
  PresenceInfo,
  RelayStatusEvent,
  TypingEvent,
} from "../types";

/** Connect to the relay and authenticate with the stored identity. */
export function connectRelay(): Promise<void> {
  return invoke("connect_relay");
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

/** Persist our own display name and announce it to the relay. */
export function setDisplayName(name: string): Promise<void> {
  return invoke("set_display_name", { name });
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
