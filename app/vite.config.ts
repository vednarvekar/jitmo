/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri loads the built files from ./dist. Fixed port so `tauri dev` can find the dev server.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { outDir: "dist", target: "es2022" },
  // Unit tests only: e2e/ holds the Playwright specs (run with `pnpm e2e`), which Vitest
  // must not pick up.
  test: { include: ["src/**/*.test.{ts,tsx}"] },
});
