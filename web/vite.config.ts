import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In dev the overlay window loads the Vite server (set OVERLAI_UI_DEV=http://localhost:5173
// for the core); API and WebSocket calls are proxied to the core on its fixed port.
const CORE = `http://127.0.0.1:${process.env.OVERLAI_PORT ?? "47823"}`;

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      "/api": { target: CORE, changeOrigin: true },
      "/ws": { target: CORE, ws: true, changeOrigin: true },
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
