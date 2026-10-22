import { defineConfig } from "@playwright/test";

// Must match global-setup.ts, which starts the hub on this port (plain HTTP,
// for the probe) and the next one (HTTPS, the web UI; split mode, D-43).
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
  // Never retried, in CI or locally: a retry that passes hides a flaky test,
  // and a flaky test is a bug to fix (M0-26 hid behind one).
  retries: 0,
  reporter: [["list"], ["html", { open: "never" }]],
  // The key rotation (D-61) changes the hub's certificate and key and revokes
  // enrollment codes, so it runs once every other test is done.
  projects: [
    { name: "main", testIgnore: /rotation\.spec\.ts/ },
    { name: "rotation", testMatch: /rotation\.spec\.ts/, dependencies: ["main"] },
  ],
  use: {
    baseURL: `https://127.0.0.1:${port + 1}`,
    ignoreHTTPSErrors: true, // the hub's self-signed certificate
    // The CSRF header the web UI sends on POSTs, for API calls made by tests.
    extraHTTPHeaders: { "X-Blentinel": "1" },
    // The installed Google Chrome by default (no download). Without it, run
    // `npx playwright install chromium` and set E2E_BROWSER_CHANNEL=chromium.
    channel: process.env.E2E_BROWSER_CHANNEL ?? "chrome",
    trace: "retain-on-failure",
  },
});
