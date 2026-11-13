// The monthly availability report (M6-05): reached from the menu, built from
// the history the real probe's reports made while the tests ran (this
// month, so far), with a target, and as a CSV download. A Client sees it.
import type { Page } from "@playwright/test";
import { COMPANY, PROBE_NAME, expect, test } from "./fixtures";

/// This month in UTC, as the month picker and the hub write it.
const thisMonth = () => new Date().toISOString().slice(0, 7);

async function openThisMonth(page: Page) {
  await page.goto("/");
  await page.locator(".nav-links").getByRole("link", { name: "Reports" }).click();
  await page.getByRole("link", { name: COMPANY }).click();
  await expect(page).toHaveURL(new RegExp(`/company/${COMPANY}/report$`));
  await page.locator("html[data-hydrated]").waitFor({ state: "attached" });
  await page.locator("input[name=report_month]").fill(thisMonth());
  await page.locator("button[name=report_show]").click();
  await expect(page.locator("h1")).toContainText(COMPANY);
  await expect(page.locator(".report-note").first()).toContainText("The month isn't over");
}

test("a company's month: each check's availability, a target, and the CSV", async ({ loggedIn: page }) => {
  await openThisMonth(page);

  const site = page.locator(".report-site", { hasText: "lab" });
  await expect(site.locator("h2")).toContainText("lab");
  const row = (name: string) => site.locator(".report-checks tbody tr", { hasText: name });
  await expect(row("Hub port")).toContainText(PROBE_NAME);
  await expect(row("Hub port").locator("td.availability")).toHaveText("100%");
  await expect(row("Closed port").locator("td.availability")).toHaveText("0%");
  await expect(row("Hub port").getByRole("link", { name: "Hub port" })).toHaveAttribute("href", /^\/checks\//);

  // Rows below a target are marked.
  await page.locator("input[name=report_target]").fill("99.9");
  await page.locator("button[name=report_show]").click();
  await expect(page.locator(".report-note", { hasText: "target of 99.9%" })).toBeVisible();
  await expect(row("Closed port")).toHaveClass(/below-target/);
  await expect(row("Hub port")).not.toHaveClass(/below-target/);

  const [download] = await Promise.all([
    page.waitForEvent("download"),
    page.getByRole("link", { name: "Download CSV" }).click(),
  ]);
  expect(download.suggestedFilename()).toBe(`blentinel-${COMPANY}-${thisMonth()}.csv`);
});

test("a Client sees their company's report", async ({ client: page }) => {
  await openThisMonth(page);
  await expect(page.locator(".report-checks").first()).toContainText("Hub port");
});
