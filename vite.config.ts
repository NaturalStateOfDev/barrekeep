import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed port and won't auto-pick a fallback.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // Don't watch the Rust source — it triggers Tauri's own rebuild.
      ignored: ["**/src-tauri/**"],
    },
  },
});
