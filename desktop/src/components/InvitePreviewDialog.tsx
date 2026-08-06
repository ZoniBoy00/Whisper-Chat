import { useCallback, useEffect, useRef, useState } from "react";
import { Check, Loader2, UserPlus, X } from "lucide-react";
import type { ProfileInfo } from "../types";
import { cx, mediaUrl, shortPeerId } from "../lib/format";
import { getProfile, relayErrorCode } from "../lib/relay";
import { useI18n } from "../i18n/I18nContext";
import { Avatar } from "./Avatar";

interface InvitePreviewDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The raw whisper:// link that opened the dialog. */
  link: string;
  /** Relay endpoint used to resolve `/media/{hash}` avatar paths. */
  relayUrl: string;
  /** Send a friend request to the invite's peer. Rejects with a relay error
   *  code on failure. */
  onAdd: (peerId: string) => Promise<void>;
  /** Our own peer ID, rejected client-side. */
  myPeerId: string;
}

/** Extract the target peer ID from a whisper:// deep link. Accepts both the
 *  `whisper://invite?peer=..` form and the browser-normalized
 *  `whisper://invite/?peer=..` form. */
function extractPeerIdFromLink(url: string): string | null {
  const match = url.match(
    /^whisper:\/\/(?:invite|verify)\/?\?[^]*\bpeer=([0-9a-f]{24})\b/i
  );
  return match ? match[1] : null;
}

/** Popup shown when the app is opened (or brought to front) by a whisper://
 *  invite link: the peer's public profile with a one-click friend request. */
export function InvitePreviewDialog({
  open,
  onOpenChange,
  link,
  relayUrl,
  onAdd,
  myPeerId,
}: InvitePreviewDialogProps) {
  const { t } = useI18n();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [profile, setProfile] = useState<ProfileInfo | null>(null);
  const [adding, setAdding] = useState(false);
  const [sent, setSent] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const peerId = extractPeerIdFromLink(link);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      setProfile(null);
      setSent(false);
      setError(null);
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open]);

  // Fetch the invited peer's public profile (name, avatar, username).
  useEffect(() => {
    if (!open || !peerId) return;
    let cancelled = false;
    getProfile(peerId)
      .then((prof) => {
        if (!cancelled) setProfile(prof);
      })
      .catch(() => {
        // `no_profile` is fine — the peer just has not registered a username;
        // the request can still be sent.
      });
    return () => {
      cancelled = true;
    };
  }, [open, peerId]);

  const close = useCallback(() => {
    if (adding) return;
    onOpenChange(false);
  }, [adding, onOpenChange]);

  const submit = async () => {
    if (!peerId) return;
    if (peerId === myPeerId) {
      setError(t("contacts.cannot_add_self"));
      return;
    }
    setAdding(true);
    setError(null);
    try {
      await onAdd(peerId);
      setSent(true);
    } catch (err) {
      switch (relayErrorCode(err)) {
        case "already_contacts":
          setError(t("contacts.already_contacts"));
          break;
        case "already_pending":
          setError(t("contacts.already_pending"));
          break;
        case "cannot_add_self":
          setError(t("contacts.cannot_add_self"));
          break;
        case "not_found":
          setError(t("contacts.not_found"));
          break;
        case "rate_limited":
          setError(t("contacts.rate_limited"));
          break;
        default:
          setError(String(err).replace(/^Error:\s*/, ""));
      }
    } finally {
      setAdding(false);
    }
  };

  const displayName = profile?.display_name ?? null;
  const username = profile?.username ?? null;
  const avatarUrl = profile?.avatar_url ?? null;
  const name = displayName ?? (peerId ? shortPeerId(peerId, 16) : "");

  return (
    <dialog
      ref={dialogRef}
      className="wp-dialog"
      aria-labelledby="invite-preview-title"
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
            aria-label={t("common.close_dialog")}
            className="absolute right-3 top-3 rounded-lg p-2 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>

          <Avatar
            name={displayName ?? undefined}
            size={84}
            src={avatarUrl ? mediaUrl(relayUrl, avatarUrl) : null}
          />
          <h2
            id="invite-preview-title"
            className="mt-3 max-w-full truncate font-display text-lg font-semibold tracking-tight text-wp-text"
          >
            {name}
          </h2>
          {username ? (
            <p className="mt-0.5 font-mono text-sm text-wp-dim">@{username}</p>
          ) : (
            <p className="mt-0.5 font-mono text-xs text-wp-faint">
              {peerId}
            </p>
          )}
          <p className="mt-2 text-xs leading-relaxed text-wp-dim">
            {t("invite.hint")}
          </p>
        </div>

        <div className="space-y-3 border-t border-wp-line/10 px-5 py-5">
          {sent ? (
            <div
              role="status"
              className="flex flex-col items-center gap-2 rounded-xl bg-wp-panel-3 px-4 py-5 text-center"
            >
              <span className="inline-flex h-9 w-9 items-center justify-center rounded-full bg-wp-accent/15 text-wp-accent">
                <Check className="h-5 w-5" aria-hidden="true" />
              </span>
              <p className="text-sm font-semibold text-wp-text">
                {t("invite.request_sent")}
              </p>
              <button
                type="button"
                onClick={close}
                className="mt-1 inline-flex items-center justify-center rounded-xl bg-wp-accent px-4 py-2 text-sm font-semibold text-wp-accent-fg transition hover:bg-wp-accent-strong active:scale-[0.98]"
              >
                {t("addContact.done")}
              </button>
            </div>
          ) : (
            <>
              {error ? (
                <p
                  role="alert"
                  className="text-xs leading-snug text-wp-danger"
                >
                  {error}
                </p>
              ) : null}
              <button
                type="button"
                onClick={() => void submit()}
                disabled={adding || !peerId}
                className={cx(
                  "inline-flex w-full items-center justify-center gap-2 rounded-xl px-4 py-2.5 text-sm font-semibold transition active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-50",
                  "bg-wp-accent text-wp-accent-fg hover:bg-wp-accent-strong"
                )}
              >
                {adding ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <UserPlus className="h-4 w-4" />
                )}
                {adding
                  ? t("addContact.sending")
                  : t("invite.add_friend")}
              </button>
            </>
          )}
        </div>
      </div>
    </dialog>
  );
}
