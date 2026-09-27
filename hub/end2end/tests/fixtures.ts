// Shared test setup: every page fails its test on a browser error or a WASM
// panic, and `loggedIn` starts with a session (via the API, so only the
// login tests go through the form).

import { test as base, expect, Page } from "@playwright/test";

export { expect };
export const COMPANY = "e2e-co";
export const PROBE_NAME = "e2e-probe";
export const token = () => process.env.E2E_TOKEN ?? "";

type Fixtures = { loggedIn: Page };

export const test = base.extend<Fixtures>({
  page: async ({ page }, use) => {
    const errors: string[] = [];
    // HTTP failures are logged as console errors too; tests check responses themselves.
    page.on("console", (m) => {
      if (m.type() === "error" && !m.text().startsWith("Failed to load resource")) {
        errors.push(m.text());
      }
    });
    page.on("pageerror", (e) => errors.push(String(e)));
    await use(page);
    expect(errors, "browser errors or WASM panics").toEqual([]);
  },
  loggedIn: async ({ page }, use) => {
    const res = await page.request.post("/api/login", { data: token() });
    expect(res.ok()).toBe(true);
    await use(page);
  },
});
