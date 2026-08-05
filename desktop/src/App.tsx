import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Loader2 } from "lucide-react";
import { Onboarding } from "./components/Onboarding";
import { MainView } from "./components/MainView";

export interface IdentityInfo {
  peer_id: string;
  exists: boolean;
}

function FullScreenLoader() {
  return (
    <div className="flex h-screen items-center justify-center bg-[#0a0e14]">
      <Loader2 className="h-6 w-6 animate-spin text-slate-500" />
    </div>
  );
}

export default function App() {
  const [loading, setLoading] = useState(true);
  const [identity, setIdentity] = useState<IdentityInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const loadIdentity = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const info = await invoke<IdentityInfo>("get_identity");
      setIdentity(info);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadIdentity();
  }, [loadIdentity]);

  if (loading) {
    return <FullScreenLoader />;
  }

  if (error) {
    return (
      <div className="flex h-screen flex-col items-center justify-center gap-4 bg-[#0a0e14] text-slate-300">
        <p className="text-sm">Could not load your identity.</p>
        <p className="max-w-md truncate text-xs text-slate-500">{error}</p>
        <button
          type="button"
          onClick={() => void loadIdentity()}
          className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white transition hover:bg-violet-500"
        >
          Retry
        </button>
      </div>
    );
  }

  if (!identity?.exists) {
    return <Onboarding onCreated={(peerId) => setIdentity({ peer_id: peerId, exists: true })} />;
  }

  return <MainView peerId={identity.peer_id} />;
}
