import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import { tmpdir } from "node:os";
import { join } from "node:path";

const host = process.env.TAURI_DEV_HOST;

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  clearScreen: false,
  // Fora da pasta do projeto: sincronizadores (Dropbox) e antivírus travam renames em node_modules/.vite.
  cacheDir: join(tmpdir(), "hsync-vite-cache"),
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    // Os renderizadores de Office (pptx/xlsx) são chunks grandes, mas carregados sob demanda.
    chunkSizeWarningLimit: 1500,
    // dist/ é limpo por scripts/clean-dist.mjs (com novas tentativas) antes do build.
    emptyOutDir: false,
    // scripts/bundle.mjs compila fora da pasta do projeto (ver HSYNC_OUT_DIR).
    outDir: process.env.HSYNC_OUT_DIR || "dist",
  },
});
