import { defineConfig } from "vite";
import { devtools } from "@tanstack/devtools-vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "path";

const host = process.env.TAURI_DEV_HOST;

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  plugins: [
    // MUST be first — the plugin documents that ordering, and it is what
    // strips every devtools import from a production bundle.
    //
    // `removeDevtoolsOnBuild` defaults to true, so `vite build` emits nothing
    // devtools-related and the packages stay devDependencies. That default is
    // the whole reason to use the plugin rather than gating the component on
    // `import.meta.env.DEV` by hand: the docs recommend the hand-rolled
    // conditional only for non-Vite projects, and a conditional is a thing
    // someone can later get wrong, whereas the strip is unconditional.
    //
    // Not added to vitest.config.ts, which has its own plugins array: the
    // event bus would open a port per test run for no benefit.
    devtools(),
    tailwindcss(),
    react({
      babel: {
        plugins: [
          ["babel-plugin-react-compiler", { target: "18" }],
        ],
      },
    }),
  ],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },

  // Vite options tailored for Tauri development
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 5174,
        }
      : undefined,
    watch: {
      // Tell vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
  // Env variables starting with VITE_ or TAURI_ENV_* are exposed
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    // Tauri uses WebView2 (Chromium) on Windows, WebKitGTK on Linux,
    // and WebKit on macOS. All three webviews support modern syntax,
    // so use `esnext` per Tauri 2 conventions. Earlier safari13 / safari14
    // attempts failed esbuild transpile because Vite's optimizeDeps
    // adds chrome87/edge88/es2019 overrides that combine into an LCD
    // that can't transpile TanStack vendor destructuring patterns.
    // `esnext` skips transpilation entirely — the webview handles it natively.
    target: "esnext",
    // Don't minify for debug builds
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    // Produce sourcemaps for debug builds
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes("node_modules")) {
            if (/[\\/](react|react-dom)[\\/]/.test(id)) return "vendor-react";
            if (id.includes("@tanstack")) return "vendor-tanstack";
          }
        },
      },
    },
  },
}));
