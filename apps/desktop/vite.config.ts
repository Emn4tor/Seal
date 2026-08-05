import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
// Set by `scripts/tauri.mjs` (the `npm run tauri` wrapper) so a second
// `npm run tauri dev` instance on the same machine picks a free port
// instead of failing outright on the port below already being taken —
// see that script's own doc comment. Defaults to Tauri's conventional 1420
// for anyone running `vite` directly, outside that wrapper.
// @ts-expect-error process is a nodejs global
const port = Number(process.env.TAURI_DEV_PORT) || 1420;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
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
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
