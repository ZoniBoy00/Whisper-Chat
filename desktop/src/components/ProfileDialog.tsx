import { useCallback, useEffect, useRef, useState } from "react";
import {
  Crown,
  Lock,
  MessageCircle,
  ShieldCheck,
  Trash2,
  UserX,
  X,
} from "lucide-react";
import type { PresenceInfo, ProfileInfo } from "../types";
import { cx, formatLastSeen, mediaUrl, shortPeerId } from "../lib/format";
import { getPresence, getProfile } from "../lib/relay";
import type { TFunction } from "../i18n/types";
import { useI18n } from "../i18n/I18nContext";
import { Avatar } from "./Avatar";
import { CopyButton } from "./CopyButton";

/** Optional group member role badge (owner/admin); null outside groups. The
 *  group side of the app is built separately, so this stays optional. */
type MemberRole = "owner" | "admin";

interface ProfileDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The contact's peer ID (Whisper ID). */
  peerId: string;
  /** Relay endpoint used to resolve `/media/{hash}` avatar paths. */
  relayUrl: string;
  /** Display name known from the contact list (fallback until the profile
   *  fetch resolves). */
  fallbackDisplayName: string | null;
  /** Username known from the contact list (fallback). */
  fallbackUsername: string | null;
  /** Avatar path known from the contact list (fallback). */
  fallbackAvatarUrl: string | null;
  /** Latest known presence (from the 30s poll / pushes). */
  initialPresence: PresenceInfo | null;
  /** Optional group member role badge; absent outside group chats. */
  memberRole?: MemberRole | null;
  /** Opens the chat with this peer and closes the dialog. */
  onMessage: () => void;
  /** Removes the contact locally (contacts + messages on this device). */
  onRemoveContact: (peerId: string) => void;
}

/** How often to re-fetch presence + profile while the dialog is open. */
const PROFILE_POLL_MS = 30_000;

/** Render the presence line (WhatsApp-style). */
function PresenceLine({ presence, t }: { presence: PresenceInfo | null; t: TFunction }) {
  if (!presence) {
    return (
      <p className="text-xs text-wp-dim">
        <Lock className="mr-1 inline h-3 w-3 text-wp-accent" aria-hidden="true" />
        {t("common.end_to_end_encrypted")}
      </p>
    );
  }
  if (presence.online) {
    return (
      <p className="inline-flex items-center gap-1.5 text-sm font-semibold text-wp-online">
        <span className="h-2 w-2 rounded-full bg-wp-online" aria-hidden="true" />
        {t("common.online")}
      </p>
    );
  }
  return (
    <p className="text-xs font-medium text-wp-dim">
      {presence.last_seen != null
        ? `${t("chat.last_seen_prefix")}${formatLastSeen(presence.last_seen, t)}`
        : t("common.last_seen_unavailable")}
    </p>
  );
}

export function ProfileDialog({
  open,
  onOpenChange,
  peerId,
  relayUrl,
  fallbackDisplayName,
  fallbackUsername,
  fallbackAvatarUrl,
  initialPresence,
  memberRole,
  onMessage,
  onRemoveContact,
}: ProfileDialogProps) {
  const { t } = useI18n();
  const dialogRef = useRef<HTMLDialogElement>(null);
  // Profile + presence fetched on open and refreshed on a 30s poll so the
  // dialog never shows a stale name or status while it stays open.
  const [profile, setProfile] = useState<ProfileInfo | null>(null);
  const [presence, setPresence] = useState<PresenceInfo | null>(null);
  const [confirmingRemove, setConfirmingRemove] = useState(false);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      setProfile(null);
      setPresence(null);
      setConfirmingRemove(false);
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    const load = async () => {
      try {
        const prof = await getProfile(peerId);
        if (!cancelled) setProfile(prof);
      } catch {
        // `no_profile` (unregistered) or a lookup failure — the fallback props
        // stand in, and the next poll retries.
      }
      try {
        const pres = await getPresence(peerId);
        if (!cancelled) setPresence(pres);
      } catch {
        // Best-effort; the poll retries shortly.
      }
    };
    void load();
    const timer = window.setInterval(load, PROFILE_POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [open, peerId]);

  const close = useCallback(() => {
    if (confirmingRemove) return;
    onOpenChange(false);
  }, [confirmingRemove, onOpenChange]);

  const displayName = profile?.display_name ?? fallbackDisplayName;
  const username = profile?.username ?? fallbackUsername;
  const avatarUrl = profile?.avatar_url ?? fallbackAvatarUrl;
  const resolvedPresence = presence ?? initialPresence;
  const name = displayName ?? shortPeerId(peerId, 16);

  return (
    <dialog
      ref={dialogRef}
      className="wp-dialog"
      aria-labelledby="profile-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
    >
      <div className="w-[min(92vw,22rem)] rounded-2xl bg-wp-panel-2">
        <div className="relative flex flex-col items-center px-5 pb-5 pt-7">
          <button
            type="button"
            onClick={close}
            aria-label={t("profile.close_profile")}
            className="absolute right-3 top-3 rounded-lg p-2 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>

          <Avatar
            name={displayName ?? undefined}
            size={92}
            src={avatarUrl ? mediaUrl(relayUrl, avatarUrl) : null}
          />

          <h2
            id="profile-title"
            className="mt-4 max-w-full truncate font-display text-xl font-semibold tracking-tight text-wp-text"
          >
            {name}
          </h2>

          {username ? (
            <p className="mt-0.5 font-mono text-sm text-wp-dim">@{username}</p>
          ) : null}

          {memberRole ? (
            <p className="mt-2 inline-flex items-center gap-1 rounded-full border border-wp-accent/30 bg-wp-accent/10 px-2.5 py-0.5 text-xs font-semibold text-wp-accent">
              {memberRole === "owner" ? (
                <Crown className="h-3 w-3" aria-hidden="true" />
              ) : (
                <ShieldCheck className="h-3 w-3" aria-hidden="true" />
              )}
              {memberRole === "owner" ? t("common.owner") : t("common.admin")}
            </p>
          ) : null}

          <div className="mt-3">
            <PresenceLine presence={resolvedPresence} t={t} />
          </div>
        </div>

        <div className="space-y-4 border-t border-wp-line/10 px-5 py-5">
          {/* Whisper ID + copy */}
          <div>
            <p className="text-xs font-medium text-wp-dim">{t("common.whisper_id")}</p>
            <div className="mt-1.5 flex items-center gap-2">
              <p className="min-w-0 flex-1 select-all break-all font-mono text-xs leading-relaxed text-wp-text">
                {peerId}
              </p>
              <CopyButton value={peerId} label={t("common.copy")} />
            </div>
          </div>

          {/* Actions */}
          <div className="flex gap-2">
            <button
              type="button"
              onClick={onMessage}
              className={cx(
                "inline-flex flex-1 items-center justify-center gap-2 rounded-xl px-4 py-2.5 text-sm font-semibold transition",
                "bg-wp-accent text-wp-accent-fg hover:bg-wp-accent-strong"
              )}
            >
              <MessageCircle className="h-4 w-4" aria-hidden="true" />
              {t("common.message")}
            </button>
            <button
              type="button"
              onClick={() => {
                if (confirmingRemove) {
                  setConfirmingRemove(false);
                  onOpenChange(false);
                  onRemoveContact(peerId);
                } else {
                  setConfirmingRemove(true);
                }
              }}
              className={cx(
                "inline-flex flex-1 items-center justify-center gap-2 rounded-xl px-4 py-2.5 text-xs font-semibold transition",
                confirmingRemove
                  ? "bg-wp-danger/15 text-wp-danger"
                  : "border border-wp-line/10 text-wp-danger hover:bg-wp-danger/10"
              )}
            >
              {confirmingRemove ? (
                <Trash2 className="h-4 w-4" aria-hidden="true" />
              ) : (
                <UserX className="h-4 w-4" aria-hidden="true" />
              )}
              {confirmingRemove ? t("profile.confirm_remove") : t("common.remove_contact")}
            </button>
          </div>

          {confirmingRemove ? (
            <p className="text-xs leading-snug text-wp-faint">
              {t("profile.remove_contact_hint")}
            </p>
          ) : null}
        </div>
      </div>
    </dialog>
  );
}
