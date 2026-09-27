import { defineConfig } from "@playwright/test";

// Must match global-setup.ts, which starts the hub on this port.
const port = Number(process.env.E2E_PORT ?? 3999);

export default defineConfig({
  testDir: "./tests",
  globalSetup: "./global-setup.ts",
  // One hub serves every test; the few that change data undo it.
  fullyParallel: true,
  workers: 2,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: [["list"], ["html", { open: "never" }]],
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    // The installed Google Chrome by default (no download). Without it, run
    // `npx playwright install chromium` and set E2E_BROWSER_CHANNEL=chromium.
    channel: process.env.E2E_BROWSER_CHANNEL ?? "chrome",
    trace: "retain-on-failure",
  },
});
