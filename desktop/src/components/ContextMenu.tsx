import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { cx } from "../lib/format";

/** One selectable row of a context menu. */
export interface ContextMenuItem {
  id: string;
  label: string;
  /** Destructive actions get the danger tint (e.g. "Delete", "Remove"). */
  danger?: boolean;
  disabled?: boolean;
  icon?: ReactNode;
  onSelect: () => void;
}

interface ContextMenuProps {
  /** Viewport coordinates the menu should open at. */
  x: number;
  y: number;
  /** Accessible name for the menu (read by screen readers). */
  label: string;
  items: ContextMenuItem[];
  onClose: () => void;
}

/** Margin from the viewport edge so the clamped menu never touches it. */
const EDGE_MARGIN = 8;

/**
 * A generic, keyboard-accessible context menu rendered at a fixed position.
 *
 * Opens with keyboard focus on the first enabled item. Arrow keys move the
 * focus through the items, Enter/Space activate, Escape (or Tab) closes, and
 * any pointer press outside the menu — or a scroll/resize — dismisses it. The
 * menu is clamped to the viewport so it never overflows the window edges.
 */
export function ContextMenu({ x, y, label, items, onClose }: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const itemRefs = useRef<(HTMLButtonElement | null)[]>([]);
  // The menu renders hidden (still measurable) at (x, y), then snaps to the
  // clamped position once measured so it never flashes off-screen.
  const [pos, setPos] = useState({ x, y });
  const [ready, setReady] = useState(false);

  useLayoutEffect(() => {
    const el = menuRef.current;
    if (!el) return;
    const { innerWidth, innerHeight } = window;
    const rect = el.getBoundingClientRect();
    const clampedX = Math.max(
      EDGE_MARGIN,
      Math.min(x, innerWidth - rect.width - EDGE_MARGIN)
    );
    const clampedY = Math.max(
      EDGE_MARGIN,
      Math.min(y, innerHeight - rect.height - EDGE_MARGIN)
    );
    setPos({ x: clampedX, y: clampedY });
    setReady(true);
  }, [x, y]);

  // Dismiss on outside pointer press, scroll or resize.
  useEffect(() => {
    const handlePointerDown = (event: PointerEvent) => {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
        onClose();
      }
    };
    const handleScroll = () => onClose();
    const handleResize = () => onClose();
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    // `pointerdown` must be seen before the item's `click` so clicking an item
    // (which lives inside the menu) is not treated as an outside press.
    document.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("scroll", handleScroll, true);
    window.addEventListener("resize", handleResize);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("scroll", handleScroll, true);
      window.removeEventListener("resize", handleResize);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose]);

  const enabledIndexes = items
    .map((item, index) => (item.disabled ? -1 : index))
    .filter((index) => index >= 0);

  const moveFocus = (direction: 1 | -1) => {
    const enabled = enabledIndexes;
    if (enabled.length === 0) return;
    const current = enabled.indexOf(
      itemRefs.current.findIndex((el) => el === document.activeElement)
    );
    const nextIndex =
      enabled[(current + direction + enabled.length) % enabled.length];
    itemRefs.current[nextIndex]?.focus();
  };

  return (
    <div
      ref={menuRef}
      role="menu"
      aria-label={label}
      style={{
        left: pos.x,
        top: pos.y,
        visibility: ready ? "visible" : "hidden",
      }}
      className="fixed z-50 w-56 animate-menu-in rounded-xl border border-wp-line/10 bg-wp-panel-2 p-1.5 shadow-2xl shadow-black/50"
      onKeyDown={(event) => {
        if (event.key === "ArrowDown") {
          event.preventDefault();
          moveFocus(1);
        } else if (event.key === "ArrowUp") {
          event.preventDefault();
          moveFocus(-1);
        } else if (event.key === "Tab") {
          onClose();
        }
      }}
    >
      {items.map((item, index) => (
        <button
          key={item.id}
          ref={(el) => {
            itemRefs.current[index] = el;
          }}
          type="button"
          role="menuitem"
          disabled={item.disabled}
          autoFocus={!item.disabled && enabledIndexes[0] === index}
          onClick={() => {
            onClose();
            item.onSelect();
          }}
          className={cx(
            "flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-sm font-medium transition-colors",
            item.disabled
              ? "cursor-not-allowed text-wp-faint opacity-50"
              : item.danger
                ? "text-wp-danger hover:bg-wp-danger/10"
                : "text-wp-text hover:bg-wp-panel-3"
          )}
        >
          {item.icon ? (
            <span className="shrink-0" aria-hidden="true">
              {item.icon}
            </span>
          ) : null}
          <span className="truncate">{item.label}</span>
        </button>
      ))}
    </div>
  );
}
