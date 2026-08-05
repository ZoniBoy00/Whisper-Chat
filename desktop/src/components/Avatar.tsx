import { useState } from "react";
import { User, Users } from "lucide-react";
import { cx } from "../lib/format";

interface AvatarProps {
  name?: string;
  size?: number;
  /** Absolute image URL; falls back to the letter avatar on error. */
  src?: string | null;
  /** Groups render a muted letter tile (the "letter group icon") instead of
   *  the teal peer gradient, so chats and groups read differently at a glance. */
  variant?: "peer" | "group";
}

export function Avatar({ name, size = 40, src, variant = "peer" }: AvatarProps) {
  const [imageFailed, setImageFailed] = useState(false);

  if (src && !imageFailed) {
    return (
      <img
        src={src}
        alt=""
        width={size}
        height={size}
        loading="lazy"
        onError={() => setImageFailed(true)}
        className="shrink-0 select-none rounded-full object-cover"
        style={{ width: size, height: size }}
      />
    );
  }

  if (variant === "group") {
    return (
      <div
        className="flex shrink-0 select-none items-center justify-center rounded-full bg-gradient-to-br from-wp-panel-3 to-wp-panel-2 text-wp-dim ring-1 ring-wp-line/10"
        style={{ width: size, height: size }}
      >
        {name ? (
          <span className="font-semibold" style={{ fontSize: size * 0.42 }}>
            {name.charAt(0).toUpperCase()}
          </span>
        ) : (
          <Users style={{ width: size * 0.5, height: size * 0.5 }} strokeWidth={2.2} />
        )}
      </div>
    );
  }

  return (
    <div
      className={cx(
        "flex shrink-0 select-none items-center justify-center rounded-full bg-gradient-to-br from-wp-accent/85 to-wp-accent-strong text-wp-accent-fg"
      )}
      style={{ width: size, height: size }}
    >
      {name ? (
        <span className="font-semibold" style={{ fontSize: size * 0.42 }}>
          {name.charAt(0).toUpperCase()}
        </span>
      ) : (
        <User style={{ width: size * 0.5, height: size * 0.5 }} strokeWidth={2.2} />
      )}
    </div>
  );
}
