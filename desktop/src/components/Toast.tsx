import { useState } from "react";
import { AlertCircle, CheckCircle2, Info, X } from "lucide-react";
import { cx } from "../lib/format";
import { useI18n } from "../i18n/I18nContext";
import type { ToastItem, ToastType } from "../hooks/useToast";

/** Icon per toast type; the color is carried by the CSS modifier class. */
const ICONS: Record<ToastType, typeof Info> = {
  success: CheckCircle2,
  error: AlertCircle,
  info: Info,
};

interface ToastCardProps {
  item: ToastItem;
  onDismiss: (id: number) => void;
}

/**
 * One toast: a dark card with a type-colored left border and icon, a message
 * and a close button. Errors announce assertively (`role="alert"`); everything
 * else announces politely (`role="status"`), so a screen reader hears the
 * feedback without the whole stack being read out. Closing animates the card
 * out before the provider removes it.
 */
function ToastCard({ item, onDismiss }: ToastCardProps) {
  const { t } = useI18n();
  const [leaving, setLeaving] = useState(false);
  const Icon = ICONS[item.type];

  const dismiss = () => {
    if (leaving) return;
    setLeaving(true);
    window.setTimeout(() => onDismiss(item.id), 160);
  };

  return (
    <div
      role={item.type === "error" ? "alert" : "status"}
      className={cx("wp-toast", `wp-toast-${item.type}`, leaving && "wp-toast-leaving")}
    >
      <Icon className="wp-toast-icon" aria-hidden="true" />
      <p className="wp-toast-message">{item.message}</p>
      <button
        type="button"
        onClick={dismiss}
        aria-label={t("toast.dismiss")}
        className="wp-toast-close"
      >
        <X className="h-3.5 w-3.5" aria-hidden="true" />
      </button>
    </div>
  );
}

interface ToastViewportProps {
  toasts: ToastItem[];
  onDismiss: (id: number) => void;
}

/**
 * The fixed, top-right stack that holds every active toast. The container is
 * `aria-live="polite"` so newly added toasts are announced; individual error
 * cards opt into an assertive announcement via their own `role="alert"`.
 */
export function ToastViewport({ toasts, onDismiss }: ToastViewportProps) {
  return (
    <div className="wp-toast-viewport" aria-live="polite" aria-atomic="false">
      {toasts.map((item) => (
        <ToastCard key={item.id} item={item} onDismiss={onDismiss} />
      ))}
    </div>
  );
}
