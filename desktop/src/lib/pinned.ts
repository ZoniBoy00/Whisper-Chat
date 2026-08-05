/**
 * Client-side pinning of chats (Signal/Telegram style): a small, per-identity
 * list of conversation IDs stored in localStorage. Pins are a UI convenience,
 * so every read/write is best-effort and never allowed to throw.
 */
const STORAGE_PREFIX = "whisper.pinnedChats:";

/** Load the pinned conversation IDs for an identity. */
export function loadPinnedChats(ownerId: string): string[] {
  try {
    const raw = localStorage.getItem(STORAGE_PREFIX + ownerId);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter((entry): entry is string => typeof entry === "string")
      : [];
  } catch {
    return [];
  }
}

/** Persist the pinned conversation IDs for an identity (best-effort). */
export function persistPinnedChats(ownerId: string, ids: string[]): void {
  try {
    localStorage.setItem(STORAGE_PREFIX + ownerId, JSON.stringify(ids));
  } catch {
    // Pins are cosmetic; a full storage failure must never break the app.
  }
}
