import type { ReactNode } from "react";
import { cx } from "../../lib/format";

/** A section heading with an accent icon, used to group settings sections. */
export function SectionHeading({
  id,
  icon,
  label,
}: {
  id: string;
  icon: ReactNode;
  label: string;
}) {
  return (
    <h3
      id={id}
      className="mb-3 flex items-center gap-1.5 text-xs font-semibold uppercase tracking-widest text-wp-faint"
    >
      <span className="text-wp-accent" aria-hidden="true">
        {icon}
      </span>
      {label}
    </h3>
  );
}

/** A labelled switch row with an accessible `role="switch"` control. */
export function ToggleRow({
  id,
  checked,
  onChange,
  title,
  description,
  icon,
  disabled,
}: {
  id: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  title: string;
  description: string;
  icon: ReactNode;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-start justify-between gap-4 rounded-xl border border-wp-line/10 bg-wp-panel-3 p-4">
      <div className="flex items-start gap-3">
        <span className="mt-0.5 text-wp-accent" aria-hidden="true">
          {icon}
        </span>
        <div>
          <p className="text-sm font-medium text-wp-text">{title}</p>
          <p className="mt-0.5 text-xs leading-snug text-wp-faint">
            {description}
          </p>
        </div>
      </div>
      <button
        type="button"
        role="switch"
        id={id}
        aria-checked={checked}
        aria-label={title}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={cx(
          "relative h-6 w-11 shrink-0 rounded-full transition-colors",
          checked ? "bg-wp-accent" : "bg-wp-panel-2 ring-1 ring-inset ring-wp-line/20",
          "disabled:cursor-not-allowed disabled:opacity-50"
        )}
      >
        <span
          aria-hidden="true"
          className={cx(
            "absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform",
            checked ? "translate-x-5" : "translate-x-0"
          )}
        />
      </button>
    </div>
  );
}
