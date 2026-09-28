import { COMPANY, expect, test } from "./fixtures";

test("a company registered by its probe can be renamed, and the dashboard shows the name", async ({
  loggedIn: page,
}) => {
  await page.goto("/companies");
  const row = page.locator(".companies-table tr", { hasText: COMPANY });
  await expect(row).toBeVisible();

  await row.getByRole("link").click();
  await expect(page).toHaveURL(new RegExp(`/companies/${COMPANY}$`));
  await page.locator("html[data-hydrated]").waitFor({ state: "attached" });
  const name = page.getByLabel("Name", { exact: true });
  await name.fill("e2e-co Ltd");
  await page.getByRole("button", { name: "Save" }).first().click();
  await expect(page.locator(".form-message")).toHaveText("Saved.");

  // The site the probe reports ("lab") can be named too.
  const site = page.locator(".sites-table tr", { hasText: "lab" });
  await site.getByRole("button", { name: "Edit" }).click();
  await page.locator('input[name="site_name"]').fill("lab (building 2)");
  await page.locator(".sites-table").getByRole("button", { name: "Save" }).click();
  await expect(page.locator(".sites-table")).toContainText("lab (building 2)");

  await page.goto("/");
  await expect(page.locator(".card-title", { hasText: "e2e-co Ltd" })).toBeVisible();

  // Put the name back: other tests look for the company by its ID.
  const res = await page.request.post(`/api/companies/${COMPANY}`, { data: { name: COMPANY } });
  expect(res.ok()).toBe(true);
  const siteRes = await page.request.post(`/api/companies/${COMPANY}/sites/lab`, { data: { name: "lab" } });
  expect(siteRes.ok()).toBe(true);
});

test("a Viewer sees company details but can't edit them", async ({ viewer: page }) => {
  await page.goto(`/companies/${COMPANY}`);
  await expect(page.getByLabel("Name", { exact: true })).toHaveJSProperty("readOnly", true);
  await expect(page.getByRole("button", { name: "Save" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Archive" })).toHaveCount(0);
});
