import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  server: {
    port: 5174,
    // Dev only. In production the same binary serves both the UI and
    // these routes, so there is nothing to proxy.
    proxy: {
      "/config": "http://localhost:8080",
      "/devices": "http://localhost:8080",
      "/healthz": "http://localhost:8080",
      "/rooms": "http://localhost:8080",
      // Leave `changeOrigin` alone. The event stream refuses a
      // handshake whose Origin is not its own Host, and the default
      // (false) forwards this server's Host — so the two agree.
      // Turning it on rewrites Host to :8080 while Origin stays :5174,
      // and the socket starts 403ing with nothing obvious to blame.
      "/events": { target: "ws://localhost:8080", ws: true },
    },
  },
  build: {
    outDir: "dist",
    assetsDir: "assets",
    sourcemap: false,
    rollupOptions: {
      output: {
        entryFileNames: "assets/[name].[hash].js",
        chunkFileNames: "assets/[name].[hash].js",
        assetFileNames: "assets/[name].[hash][extname]",
      },
    },
  },
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
    css: false,
  },
});
