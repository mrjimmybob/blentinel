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
  // The details are shown as saved; Edit opens the form, Cancel drops changes.
  const details = page.locator(".company-details");
  await expect(details).toContainText(COMPANY);
  await details.getByRole("button", { name: "Edit" }).click();
  await page.getByLabel("Contact", { exact: true }).fill("not kept");
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(details).not.toContainText("not kept");

  await details.getByRole("button", { name: "Edit" }).click();
  await page.getByLabel("Name", { exact: true }).fill("e2e-co Ltd");
  await page.getByRole("button", { name: "Save" }).first().click();
  await expect(page.locator(".form-message")).toHaveText("Saved.");
  await expect(page.locator(".company-form")).toHaveCount(0);
  await expect(page.locator(".company-details")).toContainText("e2e-co Ltd");

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
  await expect(page.locator(".company-details")).toContainText(COMPANY);
  await expect(page.locator(".company-details").getByRole("button", { name: "Edit" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Save" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Archive" })).toHaveCount(0);
});

test("a company's and a site's thresholds inherit from the level above", async ({ loggedIn: page }) => {
  await page.goto(`/companies/${COMPANY}`);
  const section = page.locator(".admin-section.thresholds");
  const memory = section.locator("input[name=memory_warning]");
  await expect(memory).toHaveAttribute("placeholder", "90%");

  await memory.fill("70");
  await section.locator("button[name=thresholds_save]").click();
  await expect(section.locator(".success-message")).toContainText("Saved");

  // The site inherits the company's limit.
  await section.locator("select[name=thresholds_level]").selectOption("lab");
  await expect(memory).toHaveValue("");
  await expect(memory).toHaveAttribute("placeholder", "70%");
  await expect(memory).toHaveAttribute("title", "from the company");

  // Back to nothing set.
  await section.locator("select[name=thresholds_level]").selectOption("");
  await expect(memory).toHaveValue("70");
  await memory.fill("");
  await section.locator("button[name=thresholds_save]").click();
  await expect(section.locator(".success-message")).toContainText("Saved");
});

test("maintenance: a window and a silence are added and removed, a site has a time zone", async ({ loggedIn: page }) => {
  // The site's time zone, set on the company page.
  await page.goto(`/companies/${COMPANY}`);
  const site = page.locator("table.sites-table tbody tr", { hasText: "lab" });
  await site.getByRole("button", { name: "Edit" }).click();
  await page.locator("input[name=site_timezone]").fill("Europe/Madrid");
  await site.getByRole("button", { name: "Save" }).click();
  await expect(site).toContainText("Europe/Madrid");

  await page.getByRole("link", { name: "Maintenance and silences" }).click();
  await expect(page).toHaveURL(new RegExp(`/companies/${COMPANY}/maintenance$`));

  // A one-off window far ahead (nothing it covers is held back meanwhile).
  await page.locator("select[name=window_scope]").selectOption("site:lab");
  await expect(page.locator("input[name=window_timezone]")).toHaveValue("Europe/Madrid");
  await page.locator("input[name=window_reason]").fill("E2E router swap");
  await page.locator("select[name=window_every]").selectOption("once");
  await page.locator("input[name=window_once_start]").fill("2099-01-01T10:00");
  await page.locator("input[name=window_once_end]").fill("2099-01-01T09:00");
  await page.locator("button[name=window_add]").click();
  await expect(page.locator(".login-error")).toContainText("the end is after the start");
  await page.locator("input[name=window_once_end]").fill("2099-01-01T12:00");
  await page.locator("button[name=window_add]").click();
  const windowRow = page.locator("table.windows-table tbody tr", { hasText: "E2E router swap" });
  await expect(windowRow).toContainText("site lab");
  await expect(windowRow).toContainText("2099-01-01 10:00 – 12:00 (Europe/Madrid)");

  // A silence of the whole company, for an hour.
  await page.locator("select[name=silence_scope]").selectOption("company");
  await page.locator("input[name=silence_reason]").fill("E2E planned move");
  await page.locator("button[name=silence_add]").click();
  const silenceRow = page.locator("table.silences-table tbody tr", { hasText: "E2E planned move" });
  await expect(silenceRow).toContainText("The whole company");

  // Removed again, for the tests after this one.
  await silenceRow.getByRole("button", { name: "Remove" }).click();
  await expect(silenceRow).toHaveCount(0);
  await windowRow.getByRole("button", { name: "Remove" }).click();
  await expect(windowRow).toHaveCount(0);
  await page.goto(`/companies/${COMPANY}`);
  await site.getByRole("button", { name: "Edit" }).click();
  await page.locator("input[name=site_timezone]").fill("UTC");
  await site.getByRole("button", { name: "Save" }).click();
  await expect(site).toContainText("UTC");
});
