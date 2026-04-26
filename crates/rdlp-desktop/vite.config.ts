import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "path";

const host = process.env.TAURI_DEV_HOST;

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  plugins: [
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
    // and WebKit on macOS. safari13 is too old for modern destructuring
    // patterns shipped by upstream deps (TanStack vendor chunks fail
    // esbuild transpile with "Transforming destructuring to the
    // configured target environment ('safari13' + 2 overrides) is not
    // supported yet"). safari14 (released Sept 2020) supports the full
    // destructuring set and matches Tauri 2's effective minimum macOS
    // target in practice.
    target:
      process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari14",
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
