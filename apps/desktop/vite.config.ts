import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Fixed port for `tauri dev` (tauri.conf.json devUrl). Never exposed beyond localhost.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: "127.0.0.1" },
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
});
