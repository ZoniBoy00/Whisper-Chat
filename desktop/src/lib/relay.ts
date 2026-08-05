import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSettings,
  ChatMessageEvent,
  ChatState,
  MessageStatusEvent,
  RelayStatusEvent,
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
