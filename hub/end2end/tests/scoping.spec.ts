import { COMPANY, expect, test } from "./fixtures";

test("a Client sees their company, read-only and without internal notes", async ({ client: page }) => {
  await page.goto("/");
  await expect(page.locator(".header-user")).toContainText("Client");
  await expect(page.locator(`a.company-card[href="/company/${COMPANY}"]`)).toBeVisible();
  await expect(page.getByRole("link", { name: "Admin" })).toHaveCount(0);
  await expect(page.getByRole("link", { name: "Users" })).toHaveCount(0);

  await page.goto(`/company/${COMPANY}`);
  await expect(page.locator(".device-table")).toBeVisible();
  await expect(page.getByRole("button", { name: "Silence" })).toHaveCount(0);

  const detail = await (await page.request.get(`/api/companies/${COMPANY}`)).json();
  expect(detail.company.notes).toBeNull();
  expect(detail.company.contact_email).toBeNull();
  // Anything else is "not found", not "forbidden".
  const other = await page.request.get("/api/companies/someone-else");
  expect(other.status()).toBe(404);
});

test("an Owner gives a user all companies except some", async ({ loggedIn: page }) => {
  const username = `boss-${Date.now()}`;
  await page.goto("/users");
  await page.locator('input[name="new_username"]').fill(username);
  await page.locator('select[name="new_role"]').selectOption("viewer");
  await page.locator('input[name="new_scope"][value="except"]').check();
  await page.locator(`input[name="new_company_${COMPANY}"]`).check();
  await page.getByRole("button", { name: "Add user" }).click();
  await expect(page.locator(".temporary-password")).toBeVisible();

  const row = page.locator(".users-table tr", { hasText: username });
  await expect(row.locator(".scope-cell")).toContainText("All except");
  await expect(row.locator(".scope-cell")).toContainText(COMPANY);

  // Tidy up: deactivate.
  await row.getByRole("button", { name: "Edit" }).click();
  await page.locator('input[name="edit_active"]').uncheck();
  await page.getByRole("button", { name: "Save" }).click();
  await expect(row).toContainText("Deactivated");
});
