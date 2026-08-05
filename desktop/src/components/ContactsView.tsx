import { UserX, Users } from "lucide-react";
import type { Conversation, PresenceInfo } from "../types";
import { formatLastSeen, mediaUrl } from "../lib/format";
import { useI18n } from "../i18n/I18nContext";
import { Avatar } from "./Avatar";

interface ContactsViewProps {
  /** 1:1 conversations only (groups are filtered out by the caller). */
  contacts: Conversation[];
  /** Latest known presence per peer (pushes + 30s poll). */
  presence: Record<string, PresenceInfo>;
  /** Relay endpoint; used to resolve `/media/{hash}` avatar paths. */
  relayUrl: string;
  activeId: string | null;
  /** Open the chat for a contact. */
  onSelect: (id: string) => void;
  /** Remove a contact locally and on the relay (context menu behavior). */
  onRemoveContact: (peerId: string) => void;
}

/**
 * WhatsApp/Signal-style "Contacts" tab: every accepted friend in one list
 * with a live Online / Last seen status line and a remove button, instead of
 * hiding contacts behind the recency-sorted conversation list.
 */
export function ContactsView({
  contacts,
  presence,
  relayUrl,
  activeId,
  onSelect,
  onRemoveContact,
}: ContactsViewProps) {
  const { t } = useI18n();

  if (contacts.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
        <div className="rounded-full bg-wp-panel-2 p-3 text-wp-faint">
          <Users className="h-5 w-5" aria-hidden="true" />
        </div>
        <p className="text-sm font-medium text-wp-dim">{t("sidebar.no_contacts")}</p>
        <p className="mt-1 text-sm leading-relaxed text-wp-faint">
          {t("sidebar.no_contacts_hint")}
        </p>
      </div>
    );
  }

  return (
    <ul className="flex flex-col gap-0.5">
      {contacts.map((contact) => {
        const info = presence[contact.peerId];
        const online = info?.online === true;
        const selected = activeId === contact.peerId;
        return (
          <li key={contact.peerId}>
            <div
              className={`group flex items-center gap-3 rounded-xl px-3 py-2 transition-colors ${
                selected ? "bg-wp-panel-3" : "hover:bg-wp-panel-3/70"
              }`}
            >
              <button
                type="button"
                onClick={() => onSelect(contact.id)}
                className="flex min-w-0 flex-1 items-center gap-3 text-left"
              >
                <Avatar
                  name={contact.name}
                  src={contact.avatarUrl ? mediaUrl(relayUrl, contact.avatarUrl) : null}
                  size={44}
                />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-semibold text-wp-text">
                    {contact.name}
                  </p>
                  <p
                    className={`truncate text-xs ${
                      online ? "text-wp-online" : "text-wp-faint"
                    }`}
                  >
                    {online
                      ? t("common.online")
                      : info?.last_seen
                        ? `${t("chat.last_seen_prefix")}${formatLastSeen(info.last_seen, t)}`
                        : "—"}
                  </p>
                </div>
              </button>
              <button
                type="button"
                aria-label={t("common.remove_contact")}
                onClick={() => onRemoveContact(contact.peerId)}
                className="rounded-lg p-2 text-wp-faint transition hover:bg-wp-danger/15 hover:text-wp-danger"
              >
                <UserX className="h-4 w-4" aria-hidden="true" />
              </button>
            </div>
          </li>
        );
      })}
    </ul>
  );
}
