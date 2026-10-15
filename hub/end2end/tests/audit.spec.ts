import { expect, OWNER, test } from "./fixtures";

test("the audit log shows who did what and can be filtered", async ({ loggedIn: page }) => {
  await page.goto("/audit");
  const table = page.locator(".audit-table");
  // The setup wizard (global setup) and logins are in the log.
  await expect(table).toContainText("setup.complete");
  await expect(table).toContainText("login.success");
  await expect(table).toContainText(OWNER);

  await page.locator('select[name="audit_action"]').selectOption("setup");
  await expect(table.locator("tbody tr")).toHaveCount(1);
  await expect(table).toContainText("setup.complete");

  const csv = page.getByRole("link", { name: "Export CSV" });
  await expect(csv).toHaveAttribute("href", "/api/audit.csv?action=setup");
});

test("only admins see the audit log", async ({ viewer: page }) => {
  await page.goto("/");
  await expect(page.getByRole("link", { name: "Dashboard" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Audit" })).toHaveCount(0);
  expect((await page.request.get("/api/audit")).status()).toBe(403);
});
