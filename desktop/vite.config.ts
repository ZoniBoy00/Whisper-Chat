import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

// Allow a second Whisper instance on the same machine: VITE_PORT=1421
// plus `--config src-tauri/tauri.dev2.json` gives the second window its
// own Vite dev server and dev URL (port 1420 stays the default).
const port = Number(process.env.VITE_PORT) || 1420;

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react()],

  // Tauri expects a fixed port and fails if it is not available.
  clearScreen: false,
  server: {
    port,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // Do not let Vite watch the Rust source tree.
      ignored: ["**/src-tauri/**"],
    },
  },
});
