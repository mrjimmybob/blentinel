// Shared test setup: every page fails its test on a browser error or a WASM
// panic; `loggedIn` (the Owner) and `viewer` start with a session (via the
// API, so only the login tests go through the form).

import { test as base, expect, Page } from "@playwright/test";

export { expect };
// Names of the company and probe that global-setup.ts creates.
export const COMPANY = "e2e-co";
export const PROBE_NAME = "e2e-probe";
// Accounts that global-setup.ts creates (the Owner through the setup wizard).
export const OWNER = "e2e-owner";
export const VIEWER = "e2e-viewer";
/** A Client limited to COMPANY. */
export const CLIENT = "e2e-client";
export const PASSWORD = "e2e-password-123";
/** The newer hub release the setup offers through [upgrades] release_url (D-75). */
export const NEWER_HUB = "999.0.0";

type Fixtures = { loggedIn: Page; viewer: Page; client: Page };

async function logIn(page: Page, username: string) {
  const res = await page.request.post("/api/auth/login", { data: { username, password: PASSWORD } });
  expect(res.ok()).toBe(true);
}

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

    // A page is interactive only once the WASM has hydrated it; clicking
    // earlier does nothing. So page loads return only after the app's marker.
    const hydrated = () => page.locator("html[data-hydrated]").waitFor({ state: "attached" });
    const goto = page.goto.bind(page);
    page.goto = async (...args: Parameters<Page["goto"]>) => {
      const response = await goto(...args);
      await hydrated();
      return response;
    };
    const reload = page.reload.bind(page);
    page.reload = async (...args: Parameters<Page["reload"]>) => {
      const response = await reload(...args);
      await hydrated();
      return response;
    };

    await use(page);
    expect(errors, "browser errors or WASM panics").toEqual([]);
  },
  loggedIn: async ({ page }, use) => {
    await logIn(page, OWNER);
    await use(page);
  },
  viewer: async ({ page }, use) => {
    await logIn(page, VIEWER);
    await use(page);
  },
  client: async ({ page }, use) => {
    await logIn(page, CLIENT);
    await use(page);
  },
});
