import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";
import { ToastViewport } from "../components/Toast";

/** The kind of feedback a toast carries; drives color and announcement. */
export type ToastType = "success" | "error" | "info";

/** One queued toast. `id` is a monotonically increasing local key. */
export interface ToastItem {
  id: number;
  type: ToastType;
  message: string;
}

interface ToastContextValue {
  /** Queue a toast; `type` defaults to `info`. */
  toast: (message: string, type?: ToastType) => void;
  /** Remove a toast immediately (used by the close button). */
  dismiss: (id: number) => void;
  /** The live queue, for rendering an in-dialog toast stack (dialogs live in
   *  the browser top layer, above every fixed element, so a dialog that wants
   *  toasts on top of itself renders its own viewport). */
  toasts: ToastItem[];
}

const ToastContext = createContext<ToastContextValue | null>(null);

/**
 * Auto-dismissal windows: errors stay readable longer, transient feedback
 * (success/info) leaves quickly so it never obstructs the UI.
 */
const DURATIONS: Record<ToastType, number> = {
  success: 3500,
  info: 3500,
  error: 6000,
};

/** Keep the stack bounded so a burst of errors cannot cover the screen. */
const MAX_VISIBLE_TOASTS = 5;

/**
 * Lightweight, dependency-free in-app toast system: a context that holds the
 * toast queue and renders them through `ToastViewport`. Consumers call
 * `useToast().toast(message, "success")` after an action settles. Toasts
 * auto-dismiss, stack top-right and announce themselves to assistive tech via
 * `role="status"` / `role="alert"` (see `Toast.tsx`).
 */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const nextId = useRef(1);

  const dismiss = useCallback((id: number) => {
    setToasts((prev) => prev.filter((item) => item.id !== id));
  }, []);

  const toast = useCallback(
    (message: string, type: ToastType = "info") => {
      const id = nextId.current++;
      setToasts((prev) => [...prev.slice(-(MAX_VISIBLE_TOASTS - 1)), { id, type, message }]);
      window.setTimeout(() => dismiss(id), DURATIONS[type]);
    },
    [dismiss]
  );

  const value = useMemo<ToastContextValue>(
    () => ({ toast, dismiss, toasts }),
    [toast, dismiss, toasts]
  );

  return (
    <ToastContext.Provider value={value}>
      {children}
      <ToastViewport toasts={toasts} onDismiss={dismiss} />
    </ToastContext.Provider>
  );
}

/** Access the toast queue; must be used within a `ToastProvider`. */
export function useToast(): ToastContextValue {
  const context = useContext(ToastContext);
  if (!context) {
    throw new Error("useToast must be used within a ToastProvider");
  }
  return context;
}
