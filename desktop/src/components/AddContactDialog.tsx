import { useEffect, useRef, useState } from "react";
import { Loader2, UserPlus, X } from "lucide-react";
import { cx } from "../lib/format";

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
  const dialogRef = useRef<HTMLDialogElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      dialog.showModal();
      inputRef.current?.focus();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open]);

  const close = () => {
    if (adding) return;
    setValue("");
    setError(null);
    onOpenChange(false);
  };

  const submit = async () => {
    const peerId = value.trim().toLowerCase();
    if (!PEER_ID_PATTERN.test(peerId)) {
      setError("Enter a valid 16-character Whisper ID (hex digits only).");
      return;
    }
    setAdding(true);
    setError(null);
    try {
      await onAdd(peerId);
      setValue("");
      onOpenChange(false);
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/, ""));
    } finally {
      setAdding(false);
    }
  };

  return (
    <dialog
      ref={dialogRef}
      className="wp-dialog"
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
              Start a new chat
            </h2>
            <p className="mt-1 text-xs leading-relaxed text-wp-dim">
              Paste a friend&apos;s Whisper ID. The session is established with
              their published pre-keys and every message is end-to-end
              encrypted.
            </p>
          </div>
          <button
            type="button"
            onClick={close}
            aria-label="Close dialog"
            className="rounded-lg p-2 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
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
            Whisper ID
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
              "inline-flex items-center justify-center gap-2 rounded-xl px-4 py-2.5 text-sm font-semibold transition",
              "bg-wp-accent text-wp-accent-fg hover:bg-wp-accent-strong",
              "disabled:cursor-not-allowed disabled:opacity-50"
            )}
          >
            {adding ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <UserPlus className="h-4 w-4" />
            )}
            {adding ? "Starting session…" : "Start chat"}
          </button>
        </form>
      </div>
    </dialog>
  );
}
