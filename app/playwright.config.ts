import { defineConfig } from "@playwright/test";

// End-to-end tests of the bar: the real page in a headless Edge (already on Windows, so no
// browser download) talking to the real engine on a SIMULATED PC (see e2e/harness.ts).
// Nothing here opens a window on the desktop or changes the machine. The page is served on
// port 1421, so a `pnpm tauri dev` running on 1420 is left alone.
export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  expect: { timeout: 8_000 },
  fullyParallel: true,
  workers: 4,
  reporter: [["list"]],
  outputDir: "./e2e-results",
  use: {
    baseURL: "http://127.0.0.1:1421",
    channel: "msedge",
    headless: true,
    // The window is 440 px wide plus the 16 px margin on each side.
    viewport: { width: 472, height: 420 },
    screenshot: "only-on-failure",
  },
  webServer: {
    command: "pnpm exec vite --port 1421 --strictPort --host 127.0.0.1",
    url: "http://127.0.0.1:1421",
    reuseExistingServer: true,
    timeout: 60_000,
  },
});
