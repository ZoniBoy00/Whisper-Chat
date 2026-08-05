import { useCallback, useEffect, useState } from "react";
import type { ProfileInfo } from "../types";
import { getChatState, getProfile } from "../lib/relay";

/**
 * Our own public profile (username + avatar). The relay is the source of
 * truth when reachable; while it is not (still connecting, offline, or the
 * profile was never published) the locally persisted `my_username` /
 * `my_avatar_url` snapshot from `get_chat_state` is returned instead, so the
 * UI shows the registered state across restarts without re-registering.
 */
export function useOwnProfile(peerId: string, connected: boolean) {
  const [myProfile, setMyProfile] = useState<ProfileInfo | null>(null);

  const refreshOwnProfile = useCallback(async () => {
    try {
      const profile = await getProfile(peerId);
      setMyProfile(profile);
      return;
    } catch {
      // `no_profile` (unregistered yet), the command isn't wired, or the relay
      // is unreachable — fall through to the persisted local snapshot.
    }
    try {
      const state = await getChatState();
      if (state.my_username || state.my_avatar_url) {
        setMyProfile({
          username: state.my_username,
          peer_id: peerId,
          display_name: state.my_display_name,
          avatar_url: state.my_avatar_url,
        });
      } else {
        setMyProfile(null);
      }
    } catch {
      setMyProfile(null);
    }
  }, [peerId]);

  useEffect(() => {
    void refreshOwnProfile();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshOwnProfile, connected]);

  return { myProfile, refreshOwnProfile };
}
