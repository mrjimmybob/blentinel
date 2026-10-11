import { Page } from "@playwright/test";
import { COMPANY, PROBE_NAME, expect, test } from "./fixtures";

async function activeSilences(page: Page): Promise<{ id: number; scope_id: string }[]> {
  const res = await page.request.get("/api/silences");
  expect(res.ok()).toBe(true);
  return res.json();
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
  await expect(devices).toHaveCount(6);
  await expect(devices.nth(0)).toContainText("Closed port");
  await expect(devices.nth(0)).toContainText("DOWN");
  await expect(devices.filter({ hasText: "Hub port" })).toContainText("UP");
  await expect(devices.filter({ hasText: "Hub certificate" })).toContainText("UP");
  await expect(devices.filter({ hasText: "Hub redirect" })).toContainText("UP");
  await expect(devices.filter({ hasText: "Probe program" })).toContainText("UP");
  await expect(devices.filter({ hasText: "Hub ping" })).toContainText("UP");
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
  const before = (await activeSilences(page)).length;

  await row.getByRole("button", { name: "Silence" }).click();
  await page.getByPlaceholder("e.g., Planned maintenance").fill("e2e maintenance");
  await page.locator(".modal-actions .btn-primary").click();
  await expect(row.getByRole("button", { name: /Muted/ })).toBeVisible();
  await expect.poll(async () => (await activeSilences(page)).length).toBe(before + 1);

  // Unmute without reloading the page first.
  await row.getByRole("button", { name: /Muted/ }).click();
  await expect(row.getByRole("button", { name: "Silence" })).toBeVisible();
  await expect.poll(async () => (await activeSilences(page)).length).toBe(before);

  // And the page agrees after a reload.
  await page.reload();
  await expect(
    page
      .locator("table.device-table tbody tr", { hasText: "Closed port" })
      .getByRole("button", { name: "Silence" }),
  ).toBeVisible();
});
