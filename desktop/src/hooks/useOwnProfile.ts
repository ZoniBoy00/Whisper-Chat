import { useCallback, useEffect, useState } from "react";
import type { ProfileInfo } from "../types";
import { getProfile } from "../lib/relay";

/** Our own public profile (username + avatar) fetched from the directory.
 *  Rejects with `no_profile` while unregistered, or when the backend command
 *  isn't wired up yet — both fall back to the unregistered UI. */
export function useOwnProfile(peerId: string) {
  const [myProfile, setMyProfile] = useState<ProfileInfo | null>(null);

  const refreshOwnProfile = useCallback(async () => {
    try {
      const profile = await getProfile(peerId);
      setMyProfile(profile);
    } catch {
      // `no_profile` (unregistered yet) or the command isn't wired on the
      // backend — treat as unregistered. Also clears stale data after an
      // identity reset, since MainView persists across peerId changes.
      setMyProfile(null);
    }
  }, [peerId]);

  useEffect(() => {
    void refreshOwnProfile();
  }, [refreshOwnProfile]);

  return { myProfile, refreshOwnProfile };
}
