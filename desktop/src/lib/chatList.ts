import type { ContactInfo, Conversation, GroupInfo, Message } from "../types";
import type { TFunction } from "../i18n/types";
import { isGroupId, shortPeerId } from "./format";

/** Build the conversation list from contacts + groups + message history,
 *  ordered by recency of the last message so the chat list behaves like
 *  Signal/WhatsApp: most recent activity first. Pinned chats (Telegram/Signal
 *  style) always sort above the rest, newest activity first within each group.
 *  Groups appear in the same list (keyed by their group ID with the group
 *  name as the display name); their letter avatar and member count
 *  distinguish them. */
export function buildConversations(
  contacts: ContactInfo[],
  groups: GroupInfo[],
  messages: Record<string, Message[]>,
  pinnedIds: string[] = []
): Conversation[] {
  const pinned = new Set(pinnedIds);
  const groupById = new Map(groups.map((g) => [g.group_id, g]));
  return contacts
    .map((contact) => {
      const isGroup = isGroupId(contact.peer_id);
      const group = isGroup ? groupById.get(contact.peer_id) : undefined;
      return {
        id: contact.peer_id,
        name: isGroup
          ? group?.name ?? contact.display_name ?? shortPeerId(contact.peer_id)
          : contact.display_name ?? shortPeerId(contact.peer_id),
        displayName: isGroup ? null : contact.display_name,
        peerId: contact.peer_id,
        username: isGroup ? undefined : contact.username,
        avatarUrl: isGroup
          ? group?.avatar_url ?? null
          : contact.avatar_url ?? null,
        isGroup,
        // An empty roster means the member count is not known yet (the group
        // was just hydrated or joined and `get_group_info` has not returned);
        // leave it undefined so the UI never renders a misleading "0".
        memberCount:
          group && group.members.length > 0 ? group.members.length : undefined,
        messages: messages[contact.peer_id] ?? [],
      };
    })
    .sort((a, b) => {
      const aPinned = pinned.has(a.peerId) ? 0 : 1;
      const bPinned = pinned.has(b.peerId) ? 0 : 1;
      if (aPinned !== bPinned) return aPinned - bPinned;
      return lastActivityAt(b) - lastActivityAt(a);
    });
}

/** Timestamp of a conversation's most recent message; 0 when it has none. */
export function lastActivityAt(conversation: Conversation): number {
  return conversation.messages[conversation.messages.length - 1]?.timestamp ?? 0;
}

/** The one-line preview under a conversation's name in the sidebar. */
export function conversationPreview(
  conversation: Conversation,
  t: TFunction
): string {
  const last = conversation.messages[conversation.messages.length - 1];
  if (last) return `${last.outgoing ? t("chatList.you_prefix") : ""}${last.text}`;
  if (conversation.isGroup === true) {
    // An undefined/zero member count means the roster hasn't been fetched yet;
    // never render a misleading "0 members".
    if (!conversation.memberCount) return "—";
    return t("common.members_count", { n: conversation.memberCount });
  }
  if (conversation.displayName) return shortPeerId(conversation.peerId);
  return "";
}
