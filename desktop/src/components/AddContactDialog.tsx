import { useEffect, useRef, useState } from "react";
import { Check, Loader2, UserPlus, X } from "lucide-react";
import { cx } from "../lib/format";
import { relayErrorCode } from "../lib/relay";
import { useI18n } from "../i18n/I18nContext";

interface AddContactDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Send a friend request to the entered peer ID. Rejects with a relay error
   *  code (`already_pending`, `already_contacts`, `cannot_add_self`,
   *  `not_found`, `rate_limited`) on failure. */
  onAdd: (peerId: string) => Promise<void>;
  /** Our own peer ID, rejected client-side with a clean message. */
  myPeerId: string;
  /** Pre-fill the input when the dialog opens (e.g. from a pasted invite
   *  link or a `whisper://` deep link). */
  initialValue?: string;
}

/** Whisper IDs are 24 lowercase hex characters. */
const PEER_ID_PATTERN = /^[0-9a-f]{24}$/i;

export function AddContactDialog({
  open,
  onOpenChange,
  onAdd,
  myPeerId,
  initialValue = "",
}: AddContactDialogProps) {
  const { t } = useI18n();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const [value, setValue] = useState(initialValue);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  // The peer a request was successfully sent to; the dialog then shows a
  // "Request sent" confirmation instead of the form.
  const [sentTo, setSentTo] = useState<string | null>(null);
  // When true the dialog plays its fade-out before `onOpenChange(false)` is
  // signalled, so the close feels as polished as the open.
  const [closing, setClosing] = useState(false);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      setClosing(false);
      setSentTo(null);
      setValue(initialValue);
      dialog.showModal();
      inputRef.current?.focus();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open, initialValue]);

  const requestClose = () => {
    if (closing) return;
    setClosing(true);
    window.setTimeout(() => {
      setClosing(false);
      setValue("");
      setError(null);
      setSentTo(null);
      onOpenChange(false);
    }, 160);
  };

  const close = () => {
    if (adding) return;
    requestClose();
  };

  const submit = async () => {
    const raw = value.trim();
    // Accept either a bare Whisper ID or a `whisper://invite?peer=..` link
    // (shared from another device via "Share invite"). The browser-normalized
    // `whisper://invite/?peer=..` form (trailing slash) is accepted too.
    const invite = raw.match(/^whisper:\/\/invite\/?\?[^]*\bpeer=([0-9a-f]{24})\b/i);
    const peerId = (invite ? invite[1] : raw).toLowerCase();
    if (!PEER_ID_PATTERN.test(peerId)) {
      setError(t("addContact.invalid_peer_id"));
      return;
    }
    if (peerId === myPeerId) {
      setError(t("contacts.cannot_add_self"));
      return;
    }
    setAdding(true);
    setError(null);
    try {
      await onAdd(peerId);
      setSentTo(peerId);
      setValue("");
    } catch (err) {
      // Translate the well-known relay error codes; anything else shows raw.
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

  return (
    <dialog
      ref={dialogRef}
      className={cx("wp-dialog", closing && "wp-dialog-closing")}
      aria-labelledby="add-contact-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
    >
      <div className="w-[min(92vw,24rem)] rounded-2xl bg-wp-panel-2 p-5">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2
              id="add-contact-title"
              className="font-display text-lg font-semibold tracking-tight text-wp-text"
            >
              {t("addContact.title")}
            </h2>
            <p className="mt-1 text-sm leading-relaxed text-wp-dim">
              {t("addContact.hint")}
            </p>
          </div>
          <button
            type="button"
            onClick={close}
            aria-label={t("common.close_dialog")}
            className="rounded-lg p-2 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text active:scale-90"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {sentTo ? (
          <div
            role="status"
            className="mt-5 flex flex-col items-center gap-3 rounded-xl bg-wp-panel-3 px-4 py-6 text-center"
          >
            <span className="inline-flex h-10 w-10 items-center justify-center rounded-full bg-wp-accent/15 text-wp-accent">
              <Check className="h-5 w-5" aria-hidden="true" />
            </span>
            <div>
              <p className="text-sm font-semibold text-wp-text">
                {t("addContact.request_sent_title")}
              </p>
              <p className="mt-1 text-xs leading-relaxed text-wp-dim">
                {t("addContact.request_sent_hint", { peerId: sentTo })}
              </p>
            </div>
            <button
              type="button"
              onClick={requestClose}
              className="inline-flex items-center justify-center gap-2 rounded-xl bg-wp-accent px-4 py-2.5 text-sm font-semibold text-wp-accent-fg transition hover:bg-wp-accent-strong active:scale-[0.98]"
            >
              {t("addContact.done")}
            </button>
          </div>
        ) : (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void submit();
            }}
            className="mt-5 flex flex-col gap-3"
          >
            <label htmlFor="add-contact-peer-id" className="sr-only">
              {t("common.whisper_id")}
            </label>
            <input
              id="add-contact-peer-id"
              ref={inputRef}
              type="text"
              value={value}
              onChange={(e) => setValue(e.target.value)}
              placeholder="whisper://invite?peer=… or 3f2a91c07b44d8e5a1b2c3d4"
              autoComplete="off"
              spellCheck={false}
              aria-invalid={error ? true : undefined}
              aria-describedby={error ? "add-contact-error" : undefined}
              className="rounded-xl bg-wp-panel-3 px-3.5 py-2.5 font-mono text-sm text-wp-text placeholder-wp-faint outline-none transition focus:ring-1 focus:ring-wp-accent/60"
            />
            {error ? (
              <p
                id="add-contact-error"
                role="alert"
                className="text-xs leading-snug text-wp-danger"
              >
                {error}
              </p>
            ) : null}
            <button
              type="submit"
              disabled={adding || !value.trim()}
              className={cx(
                "inline-flex items-center justify-center gap-2 rounded-xl px-4 py-2.5 text-sm font-semibold transition active:scale-[0.98]",
                "bg-wp-accent text-wp-accent-fg hover:bg-wp-accent-strong",
                "disabled:cursor-not-allowed disabled:opacity-50"
              )}
            >
              {adding ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <UserPlus className="h-4 w-4" />
              )}
              {adding ? t("addContact.sending") : t("addContact.send_request")}
            </button>
          </form>
        )}
      </div>
    </dialog>
  );
}
