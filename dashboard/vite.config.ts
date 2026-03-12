import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "path";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    proxy: {
      "/api": "http://127.0.0.1:3131",
    },
  },
  build: {
    outDir: "dist",
    chunkSizeWarningLimit: 1300,
    sourcemap: true,
  },
  define: {
    // Replaced at build time; runtime override via window.__API_BASE_URL__ takes precedence
    "__API_BASE_URL__": JSON.stringify(""),
  },
});
