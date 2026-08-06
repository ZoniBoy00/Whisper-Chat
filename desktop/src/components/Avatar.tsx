import { useEffect, useState } from "react";
import { User, Users } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
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
  // Bounded retry counter: a transient failure (relay still starting, a profile
  // blob not yet replicated) is retried a few times before the letter fallback
  // takes over, so an avatar self-heals instead of staying blank.
  const [attempt, setAttempt] = useState(0);

  // When the relay (re)connects, every avatar re-arms and retries: a client
  // started before the server came up would otherwise pin the letter fallback
  // forever, because the image element never re-attempts a failed load.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ connected: boolean }>("relay-status", (event) => {
      if (event.payload.connected) {
        setImageFailed(false);
        setAttempt(0);
      }
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  // A new src (a freshly uploaded avatar, or a relay URL that only resolved
  // after connect) must re-arm the image path — otherwise one transient load
  // failure pins the letter fallback for the life of the component.
  useEffect(() => {
    setImageFailed(false);
  }, [src, attempt]);

  if (src && !imageFailed) {
    return (
      <img
        src={src}
        alt=""
        width={size}
        height={size}
        loading="lazy"
        onError={() => {
          // Debug aid: surface the exact URL that failed so avatar problems
          // are diagnosable from the devtools console.
          console.warn(`[whisper] avatar failed to load: ${src}`);
          if (attempt < 3) {
            window.setTimeout(() => setAttempt((count) => count + 1), 1500);
          } else {
            setImageFailed(true);
          }
        }}
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
