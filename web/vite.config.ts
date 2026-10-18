/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// One UI for the website and the Tauri desktop app (crates/slop-desktop).
// In development the API runs on :8080 and Vite proxies /v1 to it, so the site and API share
// an origin (needed for the SameSite session cookie and the API's CSRF checks).
const api = process.env.SLOP_API_URL ?? "http://127.0.0.1:8080";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    proxy: { "/v1": { target: api, changeOrigin: false } },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: { target: "es2022", sourcemap: true },
  test: { environment: "jsdom" },
});
