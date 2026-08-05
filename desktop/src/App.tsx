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
    <div className="flex h-screen items-center justify-center bg-wp-bg">
      <Loader2 className="h-6 w-6 animate-spin text-wp-faint" />
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

  const handleReset = useCallback(async () => {
    try {
      await invoke("delete_identity");
    } catch {
      // Ignore delete failures — reloading will just keep the current view.
    }
    await loadIdentity();
  }, [loadIdentity]);

  if (loading) {
    return <FullScreenLoader />;
  }

  if (error) {
    return (
      <div className="flex h-screen flex-col items-center justify-center gap-4 bg-wp-bg text-wp-dim">
        <p className="text-sm">Could not load your identity.</p>
        <p className="max-w-md truncate text-xs text-wp-faint">{error}</p>
        <button
          type="button"
          onClick={() => void loadIdentity()}
          className="rounded-xl bg-wp-accent px-4 py-2 text-sm font-semibold text-wp-deep transition hover:bg-wp-accent-strong"
        >
          Retry
        </button>
      </div>
    );
  }

  if (!identity?.exists) {
    return <Onboarding onCreated={(peerId) => setIdentity({ peer_id: peerId, exists: true })} />;
  }

  return <MainView peerId={identity.peer_id} onReset={() => void handleReset()} />;
}
