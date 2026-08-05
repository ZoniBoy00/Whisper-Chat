import type { ContactInfo, Conversation, GroupInfo, Message } from "../types";
import { isGroupId, shortPeerId } from "./format";

/** Build the conversation list from contacts + groups + message history,
 *  ordered by recency of the last message so the chat list behaves like
 *  Signal/WhatsApp: most recent activity first. Groups appear in the same
 *  list (keyed by their group ID with the group name as the display name);
 *  their letter avatar and member count distinguish them. */
export function buildConversations(
  contacts: ContactInfo[],
  groups: GroupInfo[],
  messages: Record<string, Message[]>
): Conversation[] {
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
        avatarUrl: isGroup ? undefined : contact.avatar_url,
        isGroup,
        // An empty roster means the member count is not known yet (the group
        // was just hydrated or joined and `get_group_info` has not returned);
        // leave it undefined so the UI never renders a misleading "0".
        memberCount:
          group && group.members.length > 0 ? group.members.length : undefined,
        messages: messages[contact.peer_id] ?? [],
      };
    })
    .sort((a, b) => lastActivityAt(b) - lastActivityAt(a));
}

/** Timestamp of a conversation's most recent message; 0 when it has none. */
export function lastActivityAt(conversation: Conversation): number {
  return conversation.messages[conversation.messages.length - 1]?.timestamp ?? 0;
}

/** The one-line preview under a conversation's name in the sidebar. */
export function conversationPreview(conversation: Conversation): string {
  const last = conversation.messages[conversation.messages.length - 1];
  if (last) return `${last.outgoing ? "You: " : ""}${last.text}`;
  if (conversation.isGroup === true) {
    // An undefined/zero member count means the roster hasn't been fetched yet;
    // never render a misleading "0 members".
    if (!conversation.memberCount) return "—";
    return `${conversation.memberCount} member${conversation.memberCount === 1 ? "" : "s"}`;
  }
  if (conversation.displayName) return shortPeerId(conversation.peerId);
  return "";
}
