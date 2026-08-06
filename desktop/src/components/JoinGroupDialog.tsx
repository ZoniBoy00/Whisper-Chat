import { useCallback, useEffect, useRef, useState } from "react";
import { Loader2, Users, X } from "lucide-react";
import { cx, shortPeerId } from "../lib/format";
import { relayErrorCode } from "../lib/relay";
import { useI18n } from "../i18n/I18nContext";
import { useToast } from "../hooks/useToast";
import { Avatar } from "./Avatar";
import { ToastViewport } from "./Toast";

interface JoinGroupDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The raw whisper://join link that opened the dialog. */
  link: string;
  /** Join the group (relay call). */
  onJoin: (groupId: string, token: string) => Promise<void>;
}

/** Extract the group id + token (+ optional display name) from a
 *  `whisper://join?group=..&token=..&name=..` link. */
function parseJoinLink(url: string): {
  groupId: string;
  token: string;
  groupName: string | null;
} | null {
  const match = url.match(
    /^whisper:\/\/join\/?\?[^]*\bgroup=([0-9a-f-]+)\b[^]*\btoken=([0-9a-f-]+)\b/i
  );
  if (!match) return null;
  const nameMatch = url.match(/[?&]name=([^&]+)/i);
  const groupName = nameMatch
    ? decodeURIComponent(nameMatch[1].replace(/\+/g, " "))
    : null;
  return { groupId: match[1], token: match[2], groupName };
}

/** Popup shown when a `whisper://join` link is opened: confirm joining the
 *  group, then add us to the roster. */
export function JoinGroupDialog({
  open,
  onOpenChange,
  link,
  onJoin,
}: JoinGroupDialogProps) {
  const { t } = useI18n();
  const { toasts, dismiss } = useToast();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [joining, setJoining] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const parsed = parseJoinLink(link);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      setError(null);
      setJoining(false);
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open]);

  const close = useCallback(() => {
    if (joining) return;
    onOpenChange(false);
  }, [joining, onOpenChange]);

  const handleJoin = async () => {
    if (!parsed) return;
    setJoining(true);
    setError(null);
    try {
      await onJoin(parsed.groupId, parsed.token);
      onOpenChange(false);
    } catch (err) {
      switch (relayErrorCode(err)) {
        case "invalid_join_token":
          setError(t("join.invalid_link"));
          break;
        case "already_member":
          setError(t("join.already_member"));
          break;
        case "group_not_found":
          setError(t("join.group_not_found"));
          break;
        default:
          setError(String(err).replace(/^Error:\s*/, ""));
      }
    } finally {
      setJoining(false);
    }
  };

  const name = parsed?.groupName ?? (parsed ? shortPeerId(parsed.groupId, 16) : "");

  return (
    <dialog
      ref={dialogRef}
      className="wp-dialog"
      aria-labelledby="join-group-title"
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

          <Avatar name={name} size={84} variant="group" />
          <h2
            id="join-group-title"
            className="mt-3 max-w-full truncate font-display text-lg font-semibold tracking-tight text-wp-text"
          >
            {parsed?.groupName ?? t("join.title")}
          </h2>
          {parsed ? (
            <p className="mt-1 font-mono text-xs text-wp-faint">
              {parsed.groupId}
            </p>
          ) : null}
          <p className="mt-2 text-center text-xs leading-relaxed text-wp-dim">
            {t("join.hint")}
          </p>
        </div>

        <div className="space-y-3 border-t border-wp-line/10 px-5 py-5">
          {error ? (
            <p role="alert" className="text-xs leading-snug text-wp-danger">
              {error}
            </p>
          ) : null}
          <button
            type="button"
            onClick={() => void handleJoin()}
            disabled={joining || !parsed}
            className={cx(
              "inline-flex w-full items-center justify-center gap-2 rounded-xl px-4 py-2.5 text-sm font-semibold transition active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-50",
              "bg-wp-accent text-wp-accent-fg hover:bg-wp-accent-strong"
            )}
          >
            {joining ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Users className="h-4 w-4" />
            )}
            {joining ? t("join.joining") : t("join.join")}
          </button>
        </div>
      </div>
      <ToastViewport toasts={toasts} onDismiss={dismiss} />
    </dialog>
  );
}
