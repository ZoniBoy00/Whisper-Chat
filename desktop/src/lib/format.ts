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
