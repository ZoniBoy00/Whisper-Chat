/** Compose a class string from a set of possibly-empty fragments. */
export function cx(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(" ");
}

/** Shorten a peer ID for compact surfaces, keeping a readable fingerprint. */
export function shortPeerId(peerId: string, max = 12): string {
  if (peerId.length <= max) return peerId;
  return `${peerId.slice(0, Math.ceil(max / 2))}…${peerId.slice(-Math.floor(max / 2))}`;
}

/** Render a timestamp as a compact local clock time, e.g. "14:05". */
export function formatTime(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

/**
 * Render a peer's last-seen unix-seconds timestamp as a WhatsApp-style
 * relative string: "just now", "X minute(s) ago", "X hour(s) ago" or
 * "X day(s) ago". Future timestamps (clock skew) clamp to "just now".
 */
export function formatLastSeen(lastSeen: number): string {
  const diffSecs = Math.max(0, Math.floor(Date.now() / 1000) - lastSeen);
  if (diffSecs < 60) return "just now";
  const minutes = Math.floor(diffSecs / 60);
  if (minutes < 60) {
    return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  }
  const days = Math.floor(hours / 24);
  return `${days} day${days === 1 ? "" : "s"} ago`;
}
