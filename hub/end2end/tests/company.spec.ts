import { Page } from "@playwright/test";
import { COMPANY, PROBE_NAME, expect, test } from "./fixtures";

// The active silences with `reason`. A test watches its own silence, never
// the hub's count: tests run in parallel on one hub, and the maintenance
// test adds and removes a silence of its own meanwhile (M0-26).
async function silencesFor(page: Page, reason: string): Promise<{ id: number }[]> {
  const res = await page.request.get("/api/silences");
  expect(res.ok()).toBe(true);
  const all: { id: number; reason: string }[] = await res.json();
  return all.filter((s) => s.reason === reason);
}

test("a company card opens the company page with its probe and devices", async ({
  loggedIn: page,
}) => {
  await page.goto("/");
  await page.locator(`a.company-card[href="/company/${COMPANY}"]`).click();
  await expect(page).toHaveURL(new RegExp(`/company/${COMPANY}$`));

  const probe = page.locator("tr.probe-summary-row");
  await expect(probe).toHaveCount(1);
  await expect(probe).toContainText(PROBE_NAME);
  await expect(probe).toContainText("lab");

  // A single probe shows its devices directly, the down one first.
  const devices = page.locator("table.device-table tbody tr");
  await expect(devices).toHaveCount(7);
  await expect(devices.nth(0)).toContainText("Closed port");
  await expect(devices.nth(0)).toContainText("DOWN");
  await expect(devices.filter({ hasText: "Hub port" })).toContainText("UP");
  await expect(devices.filter({ hasText: "Hub certificate" })).toContainText("UP");
  await expect(devices.filter({ hasText: "Hub redirect" })).toContainText("UP");
  await expect(devices.filter({ hasText: "Probe program" })).toContainText("UP");
  await expect(devices.filter({ hasText: "Hub ping" })).toContainText("UP");
  await expect(devices.filter({ hasText: "System service" })).toContainText("UP");
});

test("the uptime chart renders and its range can be changed", async ({ loggedIn: page }) => {
  await page.goto(`/company/${COMPANY}`);
  await expect(page.locator("svg").first()).toBeVisible();
  await expect(page.getByText("Uptime — Last 24 Hours")).toBeVisible();

  const request = page.waitForRequest((r) => r.url().includes("/uptime?range=7d"));
  await page.locator(".range-pills").getByRole("button", { name: "7d", exact: true }).click();
  await request;
  await expect(page.getByText("Uptime — Last 7 Days")).toBeVisible();
});

test("silencing a device and unmuting it again updates the hub", async ({ loggedIn: page }) => {
  await page.goto(`/company/${COMPANY}`);
  const row = page.locator("table.device-table tbody tr", { hasText: "Closed port" });
  // Its own on each attempt: a retry mustn't count the first try's silence.
  const reason = `e2e maintenance ${test.info().retry}`;

  await row.getByRole("button", { name: "Silence" }).click();
  await page.getByPlaceholder("e.g., Planned maintenance").fill(reason);
  await page.locator(".modal-actions .btn-primary").click();
  await expect(row.getByRole("button", { name: /Muted/ })).toBeVisible();
  await expect.poll(async () => (await silencesFor(page, reason)).length).toBe(1);

  // Unmute without reloading the page first.
  await row.getByRole("button", { name: /Muted/ }).click();
  await expect(row.getByRole("button", { name: "Silence" })).toBeVisible();
  await expect.poll(async () => (await silencesFor(page, reason)).length).toBe(0);

  // And the page agrees after a reload.
  await page.reload();
  await expect(
    page
      .locator("table.device-table tbody tr", { hasText: "Closed port" })
      .getByRole("button", { name: "Silence" }),
  ).toBeVisible();
});

test("the Silence buttons appear when the user loads after the devices", async ({
  loggedIn: page,
}) => {
  // The page asks for the user and the devices at once, and either may come
  // first. Hold the user back until the devices are on the page, as a busy
  // hub might: the buttons must still appear once the user is known.
  let releaseUser!: () => void;
  const userHeld = new Promise<void>((resolve) => (releaseUser = resolve));
  await page.route("**/api/auth/me", async (route) => {
    await userHeld;
    await route.continue();
  });

  await page.goto(`/company/${COMPANY}`);
  const row = page.locator("table.device-table tbody tr", { hasText: "Closed port" });
  // Either label: the test beside this one silences the same check, in
  // parallel, on the same hub.
  const silence = row.getByRole("button", { name: /Silence|Muted/ });
  // The devices are drawn while the user is still nobody: no buttons yet.
  await expect(row).toContainText("DOWN");
  await expect(silence).toHaveCount(0);

  releaseUser();
  await expect(silence).toBeVisible();
});
