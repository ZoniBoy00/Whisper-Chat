import { useState } from "react";
import { User } from "lucide-react";

interface AvatarProps {
  name?: string;
  size?: number;
  /** Absolute image URL; falls back to the letter avatar on error. */
  src?: string | null;
}

export function Avatar({ name, size = 40, src }: AvatarProps) {
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

  return (
    <div
      className="flex shrink-0 select-none items-center justify-center rounded-full bg-gradient-to-br from-wp-accent/85 to-wp-accent-strong text-wp-accent-fg"
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
