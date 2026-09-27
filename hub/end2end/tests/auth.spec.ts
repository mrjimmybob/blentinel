import { expect, test, token } from "./fixtures";

test("without a session every page sends you to /login", async ({ page }) => {
  for (const path of ["/", "/admin", "/company/e2e-co"]) {
    await page.goto(path);
    await expect(page).toHaveURL(/\/login$/);
  }
});

test("a wrong token is refused", async ({ page }) => {
  await page.goto("/login");
  await page.getByPlaceholder("Admin token").fill("not-the-token");
  await page.getByRole("button", { name: "Log In" }).click();
  await expect(page.locator(".login-error")).toHaveText(/Invalid token/);
  await expect(page).toHaveURL(/\/login$/);
});

test("the admin token logs in, and logout ends the session", async ({ page }) => {
  await page.goto("/login");
  await page.getByPlaceholder("Admin token").fill(token());
  await page.getByRole("button", { name: "Log In" }).click();
  await expect(page).toHaveURL(/\/$/);

  await page.getByRole("button", { name: "Logout" }).click();
  await expect(page).toHaveURL(/\/login$/);
  await page.goto("/");
  await expect(page).toHaveURL(/\/login$/);
});
