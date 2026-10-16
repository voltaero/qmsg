/// <reference types="vitest/config" />
import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// In development the API is proxied, so the page and the API share an origin
// and the orchestrator needs no `app.web_origins`.
const api = process.env.QMSG_API ?? "http://127.0.0.1:50051";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "~": fileURLToPath(new URL("./src", import.meta.url)) } },
  server: {
    proxy: { "/qmsg.app.v1.App": { target: api, changeOrigin: true } },
  },
  build: { chunkSizeWarningLimit: 700 },
  test: { environment: "node" },
});
