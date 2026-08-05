import { useEffect, useRef, useState } from "react";
import { Loader2, UserPlus, X } from "lucide-react";
import { cx } from "../lib/format";
import { useI18n } from "../i18n/I18nContext";

interface AddContactDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAdd: (peerId: string) => Promise<void>;
}

/** Whisper IDs are 16 lowercase hex characters. */
const PEER_ID_PATTERN = /^[0-9a-f]{24}$/i;

export function AddContactDialog({
  open,
  onOpenChange,
  onAdd,
}: AddContactDialogProps) {
  const { t } = useI18n();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  // When true the dialog plays its fade-out before `onOpenChange(false)` is
  // signalled, so the close feels as polished as the open.
  const [closing, setClosing] = useState(false);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      setClosing(false);
      dialog.showModal();
      inputRef.current?.focus();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open]);

  const requestClose = () => {
    if (closing) return;
    setClosing(true);
    window.setTimeout(() => {
      setClosing(false);
      setValue("");
      setError(null);
      onOpenChange(false);
    }, 160);
  };

  const close = () => {
    if (adding) return;
    requestClose();
  };

  const submit = async () => {
    const peerId = value.trim().toLowerCase();
    if (!PEER_ID_PATTERN.test(peerId)) {
      setError(t("addContact.invalid_peer_id"));
      return;
    }
    setAdding(true);
    setError(null);
    try {
      await onAdd(peerId);
      requestClose();
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/, ""));
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
              {t("sidebar.start_new_chat")}
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
            placeholder="e.g. 3f2a91c07b44d8e5"
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
            {adding ? t("addContact.starting_session") : t("addContact.start_chat")}
          </button>
        </form>
      </div>
    </dialog>
  );
}
