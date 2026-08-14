import type { TFunction } from "../i18n/types";
import { DEFAULT_RELAY_URL } from "./relay";

/** Compose a class string from a set of possibly-empty fragments. */
export function cx(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(" ");
}

/** Shorten a peer ID for compact surfaces, keeping a readable fingerprint. */
export function shortPeerId(peerId: string, max = 12): string {
  if (peerId.length <= max) return peerId;
  return `${peerId.slice(0, Math.ceil(max / 2))}…${peerId.slice(-Math.floor(max / 2))}`;
}

/**
 * Whether an ID identifies a group rather than a peer. The relay assigns
 * UUIDs ("123e4567-e89b-...") as group IDs, which always contain a hyphen;
 * peer IDs are 24 lowercase hex characters with no separator.
 */
export function isGroupId(id: string): boolean {
  return id.includes("-");
}

/**
 * Turn a server avatar path ("/media/{hash}") into an absolute URL by deriving
 * the origin from the relay's ws:// endpoint. Already-absolute URLs pass
 * through unchanged; returns null when no URL can be derived.
 */
export function mediaUrl(relayUrl: string, avatarPath: string): string | null {
  if (/^https?:\/\//i.test(avatarPath)) return avatarPath;
  // Before the client has connected (and persisted the effective endpoint) the
  // relayUrl state is still empty; fall back to the built-in default relay so
  // avatars can load during that window instead of silently falling back to
  // the letter avatar.
  const base = relayUrl.trim() !== "" ? relayUrl : DEFAULT_RELAY_URL;
  try {
    const parsed = new URL(base.replace(/^ws/i, "http"));
    const path = avatarPath.startsWith("/") ? avatarPath : `/${avatarPath}`;
    return `${parsed.origin}${path}`;
  } catch {
    return null;
  }
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
 * relative string in the active UI language: "just now", "X minute(s) ago",
 * "X hour(s) ago" or "X day(s) ago". Future timestamps (clock skew) clamp to
 * "just now". The translation function also handles the Finnish partitive
 * plural forms ("1 minuutti sitten" vs "5 minuuttia sitten").
 */
export function formatLastSeen(lastSeen: number, t: TFunction): string {
  const diffSecs = Math.max(0, Math.floor(Date.now() / 1000) - lastSeen);
  if (diffSecs < 60) return t("time.just_now");
  const minutes = Math.floor(diffSecs / 60);
  if (minutes < 60) {
    return t("time.minutes_ago", { n: minutes });
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return t("time.hours_ago", { n: hours });
  }
  const days = Math.floor(hours / 24);
  return t("time.days_ago", { n: days });
}

/**
 * A local calendar key ("YYYY-MM-DD") for a timestamp. Messages sharing a key
 * belong to the same day, which is what the chat list's date pills split on.
 */
export function dayKey(timestamp: number): string {
  const date = new Date(timestamp);
  const month = `${date.getMonth() + 1}`.padStart(2, "0");
  const day = `${date.getDate()}`.padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

/**
 * WhatsApp/Signal-style centered date pill for a message timestamp: "Today",
 * "Yesterday" or a locale-aware full date for anything older.
 */
export function formatDaySeparator(
  timestamp: number,
  t: TFunction,
  language: string
): string {
  const today = dayKey(Date.now());
  const key = dayKey(timestamp);
  if (key === today) return t("chat.date_today");
  const yesterday = new Date();
  yesterday.setDate(yesterday.getDate() - 1);
  if (key === dayKey(yesterday.getTime())) return t("chat.date_yesterday");
  const locale = language === "fi" ? "fi-FI" : "en-US";
  return new Date(timestamp).toLocaleDateString(locale, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

/** One non-overlapping, case-insensitive occurrence of `query` in `text`. */
export interface TextMatch {
  start: number;
  end: number;
}

/** All non-overlapping case-insensitive occurrences of `query` in `text`. */
export function findMatches(text: string, query: string): TextMatch[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [];
  const haystack = text.toLowerCase();
  const matches: TextMatch[] = [];
  let from = 0;
  while (from < haystack.length) {
    const index = haystack.indexOf(needle, from);
    if (index === -1) break;
    matches.push({ start: index, end: index + needle.length });
    from = index + needle.length;
  }
  return matches;
}
