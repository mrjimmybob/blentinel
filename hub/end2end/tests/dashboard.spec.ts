import { COMPANY, expect, test } from "./fixtures";

test("the company card shows the probe's live state", async ({ loggedIn: page }) => {
  await page.goto("/");
  const card = page.locator(`a.company-card[href="/company/${COMPANY}"]`);
  await expect(card).toBeVisible();
  // One device up, one down (a closed port): a live outage is critical.
  await expect(card).toHaveClass(/card-border-red/);
  await expect(card).toContainText(COMPANY);
});

test("the dashboard refreshes every 15 s and stops when you leave", async ({ loggedIn: page }) => {
  test.setTimeout(90_000);
  let polls = 0;
  page.on("request", (r) => {
    if (r.url().endsWith("/api/dashboard/companies")) polls++;
  });
  await page.goto("/");
  await expect(page.locator("a.company-card")).toBeVisible();

  const before = polls;
  await page.waitForTimeout(16_500);
  expect(polls, "a refresh within 16.5 s").toBeGreaterThan(before);

  await page.getByRole("link", { name: "Admin" }).click();
  await expect(page).toHaveURL(/\/admin$/);
  await page.waitForTimeout(1_000); // requests of the new page itself
  const afterLeaving = polls;
  await page.waitForTimeout(16_500);
  expect(polls, "no dashboard refresh after leaving it").toBe(afterLeaving);
});

test("the theme choice survives a reload", async ({ loggedIn: page }) => {
  await page.goto("/");
  const html = page.locator("html");
  await expect(html).toHaveAttribute("data-theme", "light");
  await page.getByRole("button", { name: /Dark/ }).click();
  await expect(html).toHaveAttribute("data-theme", "dark");
  await page.reload();
  await expect(html).toHaveAttribute("data-theme", "dark");
  await expect(page.getByRole("button", { name: /Light/ })).toBeVisible();
});
