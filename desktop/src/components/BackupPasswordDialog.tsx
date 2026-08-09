import { useCallback, useEffect, useRef, useState } from "react";
import { Eye, EyeOff, KeyRound, Loader2, X } from "lucide-react";
import { cx } from "../lib/format";
import { useI18n } from "../i18n/I18nContext";
import { useToast } from "../hooks/useToast";
import { ToastViewport } from "./Toast";

interface BackupPasswordDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** `export` asks for a NEW password (+ confirmation) to seal a backup;
   *  `import` asks for the password that unlocks an existing backup;
   *  `set` asks for a new automatic-backup password (+ confirmation). */
  mode: "export" | "import" | "set";
  /** Runs once the password passes validation. Errors thrown here are shown
   *  inside the dialog. The caller is responsible for closing on success. */
  onSubmit: (password: string) => Promise<void>;
}

/** Minimum backup-password length; mirrors the Rust-side guard. */
export const MIN_BACKUP_PASSWORD_LEN = 8;

/** Password prompt for creating / unlocking encrypted backups and for setting
 *  the automatic-backup password. The password never leaves the device except
 *  as an Argon2id→AES-256-GCM key in the Rust layer. */
export function BackupPasswordDialog({
  open,
  onOpenChange,
  mode,
  onSubmit,
}: BackupPasswordDialogProps) {
  const { t } = useI18n();
  const { toasts, dismiss } = useToast();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      setError(null);
      setBusy(false);
      setPassword("");
      setConfirm("");
      setShowPassword(false);
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open]);

  const close = useCallback(() => {
    if (busy) return;
    onOpenChange(false);
  }, [busy, onOpenChange]);

  const needsConfirmation = mode !== "import";

  const handleSubmit = async () => {
    if (password.length < MIN_BACKUP_PASSWORD_LEN) {
      setError(
        t("backup.password_too_short", { min: String(MIN_BACKUP_PASSWORD_LEN) })
      );
      return;
    }
    if (needsConfirmation && password !== confirm) {
      setError(t("backup.password_mismatch"));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await onSubmit(password);
      onOpenChange(false);
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/, ""));
    } finally {
      setBusy(false);
    }
  };

  const titleKey =
    mode === "import"
      ? "backup.unlock_title"
      : mode === "set"
        ? "backup.set_password_title"
        : "backup.export_title";
  const hintKey =
    mode === "import"
      ? "backup.unlock_hint"
      : mode === "set"
        ? "backup.set_password_hint"
        : "backup.export_hint";

  return (
    <dialog
      ref={dialogRef}
      className="wp-dialog"
      aria-labelledby="backup-password-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
    >
      <div className="w-[min(92vw,22rem)] rounded-2xl bg-wp-panel-2">
        <div className="relative flex flex-col items-center px-5 pb-4 pt-6">
          <button
            type="button"
            onClick={close}
            aria-label={t("common.close_dialog")}
            className="absolute right-3 top-3 rounded-lg p-2 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
          <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-wp-accent/15 text-wp-accent">
            <KeyRound className="h-6 w-6" aria-hidden="true" />
          </div>
          <h2
            id="backup-password-title"
            className="mt-3 text-center font-display text-lg font-semibold tracking-tight text-wp-text"
          >
            {t(titleKey)}
          </h2>
          <p className="mt-1.5 max-w-[16rem] text-center text-xs leading-relaxed text-wp-dim">
            {t(hintKey)}
          </p>
        </div>

        <form
          className="space-y-3 border-t border-wp-line/10 px-5 py-5"
          onSubmit={(e) => {
            e.preventDefault();
            void handleSubmit();
          }}
        >
          <label className="block">
            <span className="mb-1 block text-xs font-medium text-wp-dim">
              {t("backup.password")}
            </span>
            <div className="relative">
              <input
                type={showPassword ? "text" : "password"}
                autoFocus
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder="••••••••"
                autoComplete="off"
                spellCheck={false}
                className="w-full rounded-xl border border-wp-line/20 bg-wp-panel-3 py-2.5 pl-3.5 pr-10 text-sm text-wp-text outline-none transition placeholder:text-wp-faint focus:border-wp-accent/50"
              />
              <button
                type="button"
                onClick={() => setShowPassword((show) => !show)}
                aria-label={
                  showPassword
                    ? t("backup.hide_password")
                    : t("backup.show_password")
                }
                title={
                  showPassword
                    ? t("backup.hide_password")
                    : t("backup.show_password")
                }
                className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
              >
                {showPassword ? (
                  <EyeOff className="h-4 w-4" aria-hidden="true" />
                ) : (
                  <Eye className="h-4 w-4" aria-hidden="true" />
                )}
              </button>
            </div>
          </label>
          {needsConfirmation ? (
            <label className="block">
              <span className="mb-1 block text-xs font-medium text-wp-dim">
                {t("backup.confirm_password")}
              </span>
              <div className="relative">
                <input
                  type={showPassword ? "text" : "password"}
                  value={confirm}
                  onChange={(e) => setConfirm(e.target.value)}
                  placeholder="••••••••"
                  autoComplete="off"
                  spellCheck={false}
                  className="w-full rounded-xl border border-wp-line/20 bg-wp-panel-3 py-2.5 pl-3.5 pr-10 text-sm text-wp-text outline-none transition placeholder:text-wp-faint focus:border-wp-accent/50"
                />
                <button
                  type="button"
                  onClick={() => setShowPassword((show) => !show)}
                  aria-label={
                    showPassword
                      ? t("backup.hide_password")
                      : t("backup.show_password")
                  }
                  title={
                    showPassword
                      ? t("backup.hide_password")
                      : t("backup.show_password")
                  }
                  className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
                >
                  {showPassword ? (
                    <EyeOff className="h-4 w-4" aria-hidden="true" />
                  ) : (
                    <Eye className="h-4 w-4" aria-hidden="true" />
                  )}
                </button>
              </div>
            </label>
          ) : null}
          {error ? (
            <p role="alert" className="text-xs leading-snug text-wp-danger">
              {error}
            </p>
          ) : null}
          <button
            type="submit"
            disabled={busy || password.length === 0}
            className={cx(
              "inline-flex w-full items-center justify-center gap-2 rounded-xl px-4 py-2.5 text-sm font-semibold transition active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-50",
              "bg-wp-accent text-wp-accent-fg hover:bg-wp-accent-strong"
            )}
          >
            {busy ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <KeyRound className="h-4 w-4" />
            )}
            {busy
              ? t("backup.working")
              : mode === "import"
                ? t("backup.unlock")
                : t("backup.continue")}
          </button>
        </form>
      </div>
      <ToastViewport toasts={toasts} onDismiss={dismiss} />
    </dialog>
  );
}
