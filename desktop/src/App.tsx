import { useCallback, useEffect, useState } from "react";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Loader2 } from "lucide-react";
import { cx } from "./lib/format";
import { I18nProvider, useI18n } from "./i18n/I18nContext";
import { ToastProvider } from "./hooks/useToast";
import { Onboarding } from "./components/Onboarding";
import { MainView } from "./components/MainView";
import { Splash } from "./components/Splash";

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

/**
 * Both windows (splash and main) load the same index.html — there is no router
 * to give them separate URLs. The window label, read synchronously from the
 * Tauri internals, decides which view to render: the splash screen on the
 * "splash" window, the actual app everywhere else.
 */
function WindowRouter() {
  const [windowLabel, setWindowLabel] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    try {
      const label = getCurrentWindow().label;
      if (!disposed) setWindowLabel(label);
    } catch {
      // Not running inside a Tauri webview (e.g. plain `vite dev` in a
      // browser): fall back to the main app view.
      if (!disposed) setWindowLabel("main");
    }
    return () => {
      disposed = true;
    };
  }, []);

  if (windowLabel === null) {
    // Dark, empty first paint so the splash handoff never flashes white.
    return <div className="h-screen w-screen bg-wp-bg" />;
  }

  if (windowLabel === "splash") {
    return <Splash />;
  }

  return <MainApp />;
}

function MainApp() {
  const { t } = useI18n();
  const [loading, setLoading] = useState(true);
  const [identity, setIdentity] = useState<IdentityInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The main window boots hidden behind the splash and is revealed by the Rust
  // side once `splash-done` fires. `entered` gates a soft fade-in so the app
  // does not flash onto the screen — it appears the moment the window shows.
  const [entered, setEntered] = useState(false);

  useEffect(() => {
    let disposed = false;
    let timer: number | undefined;
    const unlisteners: UnlistenFn[] = [];
    void (async () => {
      try {
        // Throws when not running inside a Tauri webview (plain `vite dev`).
        getCurrentWindow();
        const unlisten = await listen("splash-done", () => {
          if (!disposed) setEntered(true);
        });
        if (disposed) {
          unlisten();
          return;
        }
        unlisteners.push(unlisten);
        // Fallback mirroring the Rust-side timeout: if the splash signal is
        // missed (e.g. the splash webview failed to boot), reveal anyway.
        timer = window.setTimeout(() => {
          if (!disposed) setEntered(true);
        }, 2600);
      } catch {
        // Browser dev has no splash handoff — reveal immediately.
        if (!disposed) setEntered(true);
      }
    })();
    return () => {
      disposed = true;
      if (timer !== undefined) window.clearTimeout(timer);
      for (const unlisten of unlisteners) unlisten();
    };
  }, []);

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

  let content: ReactNode;
  if (loading) {
    content = <FullScreenLoader />;
  } else if (error) {
    content = (
      <div className="flex h-screen flex-col items-center justify-center gap-4 bg-wp-bg text-wp-dim">
        <p className="text-sm">{t("app.identity_load_failed")}</p>
        <p className="max-w-md truncate text-xs text-wp-faint">{error}</p>
        <button
          type="button"
          onClick={() => void loadIdentity()}
          className="rounded-xl bg-wp-accent px-4 py-2 text-sm font-semibold text-wp-accent-fg transition hover:bg-wp-accent-strong"
        >
          {t("app.retry")}
        </button>
      </div>
    );
  } else if (!identity?.exists) {
    content = (
      <Onboarding
        onCreated={(peerId) => setIdentity({ peer_id: peerId, exists: true })}
      />
    );
  } else {
    content = <MainView peerId={identity.peer_id} onReset={() => void handleReset()} />;
  }

  return (
    <div className={cx("h-screen", entered && "animate-app-in")}>{content}</div>
  );
}

export default function App() {
  return (
    <I18nProvider>
      <ToastProvider>
        <WindowRouter />
      </ToastProvider>
    </I18nProvider>
  );
}
