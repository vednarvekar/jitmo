import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri loads the built files from ./dist. Fixed port so `tauri dev` can find the dev server.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { outDir: "dist", target: "es2022" },
});
