// SPDX-License-Identifier: AGPL-3.0-or-later
/// <reference types="vitest/config" />
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

import { renderDocument } from "./src/markdown";

/**
 * guide.html: the site's page with the publishing guide already in it, served at /publish/guide
 * (crates/slop-api/src/web.rs). Tools and AI agents that don't run JavaScript get the whole
 * guide; in a browser, the app replaces it with the same guide on load.
 */
function prerenderGuide(): Plugin {
  return {
    name: "slop-prerender-guide",
    apply: "build",
    // After the HTML is written, since Vite emits index.html late in the build.
    writeBundle(options) {
      const dir = options.dir ?? "dist";
      const guide = renderDocument(readFileSync("public/publishing.md", "utf8"));
      const html = readFileSync(join(dir, "index.html"), "utf8")
        .replace("<title>SLAPorSLOP</title>", "<title>How to publish · SLAPorSLOP</title>")
        .replace(
          '<div id="root"></div>',
          `<div id="root"><div class="page narrow guide">` +
            `<p>For tools and AI agents, this guide is also plain Markdown: <a href="/publishing.md">/publishing.md</a></p>` +
            `<article class="prose doc">${guide}</article></div></div>`,
        );
      writeFileSync(join(dir, "guide.html"), html);
    },
  };
}

// One UI for the website and the Tauri desktop app (crates/slop-desktop).
// In development the API runs on :8080 and Vite proxies /v1 to it, so the site and API share
// an origin (needed for the SameSite session cookie and the API's CSRF checks).
const api = process.env.SLOP_API_URL ?? "http://127.0.0.1:8080";

export default defineConfig({
  plugins: [react(), prerenderGuide()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    proxy: { "/v1": { target: api, changeOrigin: false } },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  // The site a desktop app build is for (its SLOP_API_URL), for the page bar's address
  // (components/BrowserBar.tsx); the website uses its own address instead.
  define: { __SLOP_SITE_URL__: JSON.stringify(process.env.SLOP_API_URL ?? "") },
  build: {
    target: "es2022",
    sourcemap: true,
    // The desktop app's small sign-in window is its own page (crates/slop-desktop).
    rollupOptions: { input: { main: "index.html", login: "login.html", overlay: "overlay.html" } },
  },
  test: { environment: "jsdom" },
});
