import {fileURLToPath} from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
export default defineConfig({
  plugins: [react()],
  resolve: {alias: {"@r-music/sdk": fileURLToPath(new URL("../../packages/sdk/src/index.ts", import.meta.url))}},
  base: "./",
  server: { proxy: { "/api": { target: process.env.RMUSIC_DEV_CORE||"http://127.0.0.1:7800", ws: true } } },
  build: { target: "es2022" },
});
