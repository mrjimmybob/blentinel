import { expect, OWNER, PASSWORD, test } from "./fixtures";

test("without a session every page sends you to /login", async ({ page }) => {
  for (const path of ["/", "/admin", "/users", "/account", "/company/e2e-co"]) {
    await page.goto(path);
    await expect(page).toHaveURL(/\/login$/);
  }
});

test("the setup wizard is closed once the hub has users", async ({ page }) => {
  await page.goto("/login");
  await expect(page.getByPlaceholder("Username")).toBeVisible();
  await expect(page).toHaveURL(/\/login$/); // not sent to /setup
  const res = await page.request.post("/api/setup", {
    data: { code: "WHATEVER", username: "intruder", display_name: "", password: "long-enough-password" },
  });
  expect(res.status()).toBe(409);
});

test("a wrong password is refused", async ({ page }) => {
  await page.goto("/login");
  await page.getByPlaceholder("Username").fill(OWNER);
  await page.getByPlaceholder("Password").fill("not-the-password");
  await page.getByRole("button", { name: "Log In" }).click();
  await expect(page.locator(".login-error")).toHaveText(/wrong username or password/);
  await expect(page).toHaveURL(/\/login$/);
});

test("username and password log in, and logout ends the session", async ({ page }) => {
  await page.goto("/login");
  await page.getByPlaceholder("Username").fill(OWNER);
  await page.getByPlaceholder("Password").fill(PASSWORD);
  await page.getByPlaceholder("Password").press("Enter");
  await expect(page).toHaveURL(/\/$/);
  await expect(page.locator(".header-user")).toContainText("E2E Owner");
  await expect(page.locator(".header-user")).toContainText("Owner");

  await page.getByRole("button", { name: "Logout" }).click();
  await expect(page).toHaveURL(/\/login$/);
  await page.goto("/");
  await expect(page).toHaveURL(/\/login$/);
});
