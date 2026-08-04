import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Deliberately separate from vite.config.ts rather than merged into it —
// that file's `server`/`clearScreen` options are all Tauri-dev-server
// specific and irrelevant here.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  test: {
    environment: "jsdom",
    setupFiles: ["./src/vitest-setup.ts"],
  },
});
