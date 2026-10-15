import { expect, test } from "./fixtures";

// Incidents (M3-05, D-67). The e2e probe's "Closed port" check is always
// down, so it has an open incident a few reports after the probe starts.
async function openIncident(page: import("@playwright/test").Page) {
  await page.goto("/incidents");
  const row = page.locator(".incidents-table tbody tr", { hasText: "Closed port" });
  await expect(async () => {
    await page.reload();
    await expect(row).toBeVisible({ timeout: 1_000 });
  }).toPass({ timeout: 45_000 });
  await expect(row.locator(".incident-badge")).toHaveText("DOWN");
  await row.getByRole("link", { name: "Closed port" }).click();
  await expect(page.getByRole("heading", { name: /Closed port/ })).toBeVisible();
}

test("a technician's view: an open incident is acknowledged and noted", async ({ loggedIn: page }) => {
  await openIncident(page);
  const timeline = page.locator(".incident-timeline");
  await expect(timeline).toContainText("Opened");
  await expect(page.locator(".incident-ack")).toHaveText("Not acknowledged");

  await page.locator('textarea[name="incident_note"]').fill("e2e: on my way");
  await page.locator('button[name="incident_acknowledge"]').click();
  await expect(page.locator(".incident-ack")).toContainText("Acknowledged by e2e-owner");
  await expect(page.locator('button[name="incident_acknowledge"]')).toHaveCount(0);
  await expect(timeline).toContainText("e2e: on my way");

  await page.locator('textarea[name="incident_note"]').fill("e2e: the port is closed on purpose");
  await page.locator('button[name="incident_add_note"]').click();
  await expect(timeline).toContainText("e2e: the port is closed on purpose");

  await page.goto("/incidents");
  await expect(page.locator(".incidents-table")).toContainText("e2e-owner");
});

test("a client sees the incident but not what people wrote", async ({ client: page }) => {
  await openIncident(page);
  await expect(page.locator(".incident-timeline")).toContainText("Opened");
  await expect(page.locator(".incident-timeline")).not.toContainText("e2e:");
  await expect(page.locator('textarea[name="incident_note"]')).toHaveCount(0);
});
