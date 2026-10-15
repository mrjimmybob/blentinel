import { COMPANY, expect, PASSWORD, test } from "./fixtures";

test("a Viewer sees no Admin, Users or Silence controls", async ({ viewer: page }) => {
  await page.goto("/");
  await expect(page.locator(".header-user")).toContainText("Viewer");
  await expect(page.getByRole("link", { name: "Dashboard" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Admin" })).toHaveCount(0);
  await expect(page.getByRole("link", { name: "Users" })).toHaveCount(0);
  await expect(page.locator(".db-size-indicator")).toHaveCount(0);

  await page.goto(`/company/${COMPANY}`);
  await expect(page.locator(".device-table")).toBeVisible();
  await expect(page.getByRole("button", { name: "Silence" })).toHaveCount(0);

  // The hub refuses it anyway.
  const res = await page.request.post("/api/silence", {
    data: { check_id: "k", reason: "no", duration_hours: 1 },
  });
  expect(res.status()).toBe(403);
});

test("an Owner creates a user, who must set a password at first login", async ({ loggedIn: page, browser }) => {
  const username = `tech-${Date.now()}`;
  await page.goto("/users");
  await page.locator('input[name="new_username"]').fill(username);
  await page.locator('select[name="new_role"]').selectOption("technician");
  await page.getByRole("button", { name: "Add user" }).click();
  const temporary = (await page.locator(".temporary-password").textContent())?.trim() ?? "";
  expect(temporary.length).toBeGreaterThanOrEqual(12);
  await expect(page.locator(".users-table")).toContainText(username);
  await expect(page.locator(".users-table")).toContainText("Must change password");

  // The new user, in their own browser context.
  const context = await browser.newContext();
  const newUser = await context.newPage();
  await newUser.goto("/login");
  await newUser.locator("html[data-hydrated]").waitFor({ state: "attached" });
  await newUser.getByPlaceholder("Username").fill(username);
  await newUser.getByPlaceholder("Password").fill(temporary);
  await newUser.getByRole("button", { name: "Log In" }).click();
  await expect(newUser).toHaveURL(/\/account$/);
  await expect(newUser.locator(".notice-warning")).toContainText("temporary password");
  await newUser.locator("html[data-hydrated]").waitFor({ state: "attached" });
  await newUser.getByLabel("Current password").fill(temporary);
  await newUser.getByLabel("New password (at least 12 characters)").fill(PASSWORD);
  await newUser.getByLabel("Repeat the new password").fill(PASSWORD);
  await newUser.getByRole("button", { name: "Change password" }).click();
  await expect(newUser).toHaveURL(/\/$/);
  await expect(newUser.locator(".header-user")).toContainText("Technician");
  await context.close();

  // Deactivate them again, so the shared hub stays tidy.
  await page.reload();
  const row = page.locator(".users-table tr", { hasText: username });
  await row.getByRole("button", { name: "Edit" }).click();
  await page.locator('input[name="edit_active"]').uncheck();
  await page.getByRole("button", { name: "Save" }).click();
  await expect(row).toContainText("Deactivated");
});
