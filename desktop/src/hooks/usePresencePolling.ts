import { useEffect } from "react";
import type { PresenceInfo } from "../types";
import { getPresence, watchPresence } from "../lib/relay";

/** How often to re-fetch the active peer's presence (pushes are real-time;
 *  the poll only guarantees freshness across reconnects). */
const PRESENCE_POLL_MS = 30_000;

interface UsePresencePollingParams {
  activePeerId: string | null;
  connected: boolean;
  /** Merge a fresh presence snapshot for a peer into the shared map. */
  onPresence: (peerId: string, info: PresenceInfo) => void;
}

/** Keep the active peer's presence current: a `watch_presence` subscription
 *  delivers real-time online/offline pushes, while a 30-second `get_presence`
 *  poll seeds the initial state and covers events missed across reconnects.
 *  Re-running on `connected` re-subscribes after every reconnect. */
export function usePresencePolling({
  activePeerId,
  connected,
  onPresence,
}: UsePresencePollingParams) {
  useEffect(() => {
    if (!activePeerId) return;
    let cancelled = false;
    const poll = async () => {
      try {
        const info = await getPresence(activePeerId);
        if (!cancelled) onPresence(activePeerId, info);
      } catch {
        // Best-effort: a transient failure (e.g. while disconnected) is
        // recovered by the next poll or by a presence push.
      }
    };
    if (connected) {
      void watchPresence(activePeerId).catch(() => {});
      void poll();
    }
    const timer = setInterval(poll, PRESENCE_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [activePeerId, connected, onPresence]);
}
