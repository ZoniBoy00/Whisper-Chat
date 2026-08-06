import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useI18n } from "../i18n/I18nContext";

/** The emoji palette offered by the reaction picker (Signal-style presets). */
export const REACTION_EMOJIS = ["👍", "❤️", "😂", "😮", "😢", "🙏", "🔥", "🎉"];

interface ReactionPickerProps {
  /** Viewport coordinates to open at (the click point). */
  x: number;
  y: number;
  /** Called with the chosen emoji; the caller decides react/un-react. */
  onPick: (emoji: string) => void;
  onClose: () => void;
}

/** Horizontal emoji picker used by BOTH the message context menu ("Add
 *  reaction") and the quick-react button on every bubble, so the behaviour is
 *  identical: it renders fixed at the click point (clamped to the viewport so
 *  the chat list's overflow can never clip it) and dismisses on an outside
 *  press or Escape. */
export function ReactionPicker({ x, y, onPick, onClose }: ReactionPickerProps) {
  const { t } = useI18n();
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });

  // The picker is measured hidden at (x, y), then snapped above the click
  // point and clamped so it never overflows the window edges.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const clampedX = Math.max(8, Math.min(x, window.innerWidth - rect.width - 8));
    const clampedY = Math.max(
      8,
      Math.min(y - rect.height - 8, window.innerHeight - rect.height - 8)
    );
    setPos({ x: clampedX, y: clampedY });
  }, [x, y]);

  // Dismiss on outside pointer press or Escape.
  useEffect(() => {
    const handlePointerDown = (event: PointerEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) {
        onClose();
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      role="menu"
      aria-label={t("chat.react_to_message")}
      style={{ left: pos.x, top: pos.y }}
      className="fixed z-50 flex animate-menu-in items-center gap-0.5 rounded-full border border-wp-line/10 bg-wp-panel-2 p-1.5 shadow-2xl shadow-black/50"
    >
      {REACTION_EMOJIS.map((emoji) => (
        <button
          key={emoji}
          type="button"
          role="menuitem"
          onClick={() => onPick(emoji)}
          className="rounded-full px-1 py-0.5 text-xl leading-none transition hover:bg-wp-panel-3 active:scale-90"
        >
          {emoji}
        </button>
      ))}
    </div>
  );
}
