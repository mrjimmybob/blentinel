// The probe page (M2-05): checks, their results, sync state and interval;
// "Run checks now" through the command channel (M2-06).
// Changes here use a *disabled* check, which isn't sent to the probe, so
// the probe's config (and what the other tests see) never changes.
import type { Page } from "@playwright/test";
import { COMPANY, PROBE_NAME, expect, test } from "./fixtures";

const probePage = () => `/probes/${process.env.E2E_PROBE_ID}`;
const dialogSave = (page: Page) => page.locator(".check-dialog").getByRole("button", { name: "Save" }).click();

test("a probe's page shows its checks, their results and that it runs them", async ({ loggedIn: page }) => {
  await page.goto(`/companies/${COMPANY}`);
  await page.locator(".probes-table").getByRole("link", { name: PROBE_NAME }).click();
  await expect(page).toHaveURL(new RegExp(`${probePage()}$`));
  await page.locator("html[data-hydrated]").waitFor({ state: "attached" });

  await expect(page.locator("h1")).toHaveText(PROBE_NAME);
  await expect(page.locator(".sync-badge")).toHaveText("In sync");
  await expect(page.locator("#interval_secs")).toHaveValue("5");
  const rows = page.locator(".checks-table tbody tr");
  await expect(rows.filter({ hasText: "Hub port" })).toContainText("UP");
  await expect(rows.filter({ hasText: "Closed port" })).toContainText("DOWN");
  await expect(rows.filter({ hasText: "Closed port" })).toContainText("TCP port");
});

test("a check is added, edited and deleted from the probe's page", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  const name = `E2E uptime ${Date.now()}`;

  // The form checks values like the hub does, before sending.
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_host").fill("router.lan");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("IP address");

  // Another kind shows its own fields; this one has none.
  await page.locator("#check_kind").selectOption("uptime");
  await expect(page.locator("#check_host")).toHaveCount(0);
  await page.locator('input[name="check_enabled"]').uncheck();
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await expect(row).toContainText("Disabled");
  await expect(row).toContainText("Uptime");
  await expect(page.locator(".sync-badge")).toHaveText("In sync"); // disabled: not sent

  // When it alerts (D-63): empty is the default, 2 in a row; 0 is refused.
  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator('input[name="check_fail_after"]')).toHaveValue("");
  await page.locator("#check_name").fill(`${name} (renamed)`);
  await page.locator('input[name="check_fail_after"]').fill("0");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("1 to 10 failed results");
  await page.locator('input[name="check_fail_after"]').fill("3");
  await dialogSave(page);
  const renamed = page.locator(".checks-table tbody tr", { hasText: `${name} (renamed)` });
  await expect(renamed).toHaveCount(1);
  await renamed.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator('input[name="check_fail_after"]')).toHaveValue("3");
  await expect(page.locator('input[name="check_recover_after"]')).toHaveValue("");
  await page.getByRole("button", { name: "Cancel" }).click();

  await renamed.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: name })).toHaveCount(0);
});

test("an interval outside the allowed range is refused with the reason", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  await page.locator("#interval_secs").fill("1");
  await page.locator(".interval-editor").getByRole("button", { name: "Save" }).click();
  await expect(page.locator(".login-error").first()).toContainText("5-3600");
  await page.reload();
  await expect(page.locator("#interval_secs")).toHaveValue("5");
});

test("Run checks now reaches the probe and comes back done", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  await page.locator("html[data-hydrated]").waitFor({ state: "attached" });
  await page.getByRole("button", { name: "Run checks now" }).click();

  // Queued, then sent with the answer to the probe's next report (every 5 s
  // here), then done with the report after that. The page reloads every 15 s.
  const row = page.locator(".commands-table tbody tr").first();
  await expect(row).toContainText("Run checks now");
  await expect(row).toContainText("by ");
  await expect(row.locator(".command-badge")).toHaveText("Done", { timeout: 45_000 });
  await expect(row).toContainText(/ran \d+ checks?/);
});

test("a Viewer sees the checks but can't change them", async ({ viewer: page }) => {
  await page.goto(probePage());
  await expect(page.locator(".checks-table tbody tr", { hasText: "Hub port" })).toHaveCount(1);
  await expect(page.getByRole("button", { name: "Add check" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Edit" })).toHaveCount(0);
  await expect(page.locator("#interval_secs")).toBeDisabled();
  await expect(page.getByRole("button", { name: "Run checks now" })).toHaveCount(0);
});
