// What is wrong now (M6-03): reached from the menu; the real probe's
// "Closed port" check is always Down, its "Hub port" always Up. Search,
// status and kind filters run on the hub. A Client sees their company's.
import type { Page } from "@playwright/test";
import { COMPANY, PROBE_NAME, expect, test } from "./fixtures";

async function openProblems(page: Page) {
  await page.goto("/");
  await page.locator(".nav-links").getByRole("link", { name: "Problems" }).click();
  await expect(page).toHaveURL(/\/problems$/);
  await page.locator("html[data-hydrated]").waitFor({ state: "attached" });
}

const closedPort = (page: Page) => page.locator(".problems-table tbody tr", { hasText: "Closed port" });

test("the problems list shows what is down, by company and site, and filters it", async ({ loggedIn: page }) => {
  await openProblems(page);
  const company = page.locator(".problems-company", { hasText: COMPANY });
  await expect(company.locator("h2")).toContainText(COMPANY);
  await expect(company.locator(".problems-site").first()).toHaveText("lab");

  const row = closedPort(page);
  await expect(row).toContainText("DOWN");
  await expect(row).toContainText(PROBE_NAME);
  await expect(row).toContainText("127.0.0.1:1");
  await expect(row.getByRole("link", { name: "Closed port" })).toHaveAttribute("href", /^\/checks\//);
  // Its incident, and whether someone is on it (the incidents test may be).
  await expect(row.getByRole("link", { name: /^#\d+$/ })).toBeVisible();
  await expect(row.locator("td").nth(6)).toHaveText(/Not yet|\d{4}-\d{2}-\d{2} \d{2}:\d{2}/);
  await expect(page.locator(".problems-table tbody tr", { hasText: "Hub port" })).toHaveCount(0);

  // Search: words found in a check's name; nothing for a check that is Up.
  const search = page.locator("input[name=problems_search]");
  await search.fill("closed");
  await expect(row).toBeVisible();
  await search.fill("hub port");
  await expect(page.locator(".empty-state")).toHaveText("No problem matches the search and filters.");
  await search.fill("");
  await expect(row).toBeVisible();

  // Status: Warning only hides it; Down shows it again.
  await page.locator("input[name=problems_warning]").check();
  await expect(row).toHaveCount(0);
  await page.locator("input[name=problems_down]").check();
  await expect(row).toBeVisible();
  await page.locator("input[name=problems_warning]").uncheck();
  await page.locator("input[name=problems_down]").uncheck();

  // Kind: the TCP ports.
  const kind = page.locator("select[name=problems_kind]");
  await expect(kind.locator("option[value=tcp]")).toHaveCount(1);
  await kind.selectOption("tcp");
  await expect(row).toBeVisible();
  await kind.selectOption("");
  await expect(row).toBeVisible();
});

test("a Client sees their company's problems", async ({ client: page }) => {
  await openProblems(page);
  await expect(page.locator(".problems-company", { hasText: COMPANY })).toBeVisible();
  await expect(closedPort(page)).toContainText("DOWN");
});
