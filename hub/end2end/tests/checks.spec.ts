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

  // M5-01: the real probe read the hub's own certificate (self-signed, 10
  // years: the check doesn't verify who issued it) and counted its days.
  const certificate = rows.filter({ hasText: "Hub certificate" });
  await expect(certificate).toContainText("TLS certificate");
  await expect(certificate).toContainText("UP");
  await expect(certificate).toContainText(/Certificate expires in \d+ days/);
});

// M5-01, D-77, on a disabled check (the probe's config stays).
test("a TLS certificate check: port 443 unless typed, days-left limits the other way round", async ({
  loggedIn: page,
}) => {
  await page.goto(probePage());
  const name = `E2E certificate ${Date.now()}`;
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_kind").selectOption("tls");
  await page.locator("#check_host").fill("example.com");
  await expect(page.locator('input[name="check_unverified"]')).not.toBeChecked();

  // Days left fall: the warning limit is the higher one.
  await page.locator('input[name="check_warning_at"]').fill("7");
  await page.locator('input[name="check_down_at"]').fill("30");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("above the down limit");
  await page.locator('input[name="check_warning_at"]').fill("30");
  await page.locator('input[name="check_down_at"]').fill("7");
  await page.locator('input[name="check_unverified"]').check();
  await page.locator('input[name="check_enabled"]').uncheck();
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await expect(row).toContainText("TLS certificate");
  await expect(row).toContainText("example.com:443");

  // The dialog shows what was saved.
  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator("#check_port")).toHaveValue("443");
  await expect(page.locator('input[name="check_unverified"]')).toBeChecked();
  await expect(page.locator('input[name="check_warning_at"]')).toHaveValue("30");
  await page.getByRole("button", { name: "Cancel" }).click();

  await row.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: name })).toHaveCount(0);
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

  // A ping has latency limits (D-64); uptime has none.
  await expect(page.locator('input[name="check_warning_at"]')).toBeVisible();
  await page.locator('input[name="check_warning_at"]').fill("slow");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("a number");

  // Another kind shows its own fields; this one has none.
  await page.locator("#check_kind").selectOption("uptime");
  await expect(page.locator("#check_host")).toHaveCount(0);
  await expect(page.locator('input[name="check_warning_at"]')).toHaveCount(0);
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

// Check dependencies (D-68), on a disabled check (the probe's config stays).
test("a check is put behind another and shows it", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  const name = `E2E behind ${Date.now()}`;
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_kind").selectOption("uptime");
  await page.locator("#check_depends_on").selectOption({ label: "Hub port" });
  await page.locator('input[name="check_enabled"]').uncheck();
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await expect(row.locator(".check-behind")).toHaveText(" · behind Hub port");

  // The dialog shows it; "Nothing" takes it away.
  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator("#check_depends_on option:checked")).toHaveText("Hub port");
  await expect(page.locator("#check_depends_on option", { hasText: name })).toHaveCount(0);
  await page.locator("#check_depends_on").selectOption({ label: "Nothing" });
  await dialogSave(page);
  await expect(row.locator(".check-behind")).toHaveCount(0);

  await row.getByRole("button", { name: "Delete" }).click();
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

test("a command reaches the probe between its reports, in a second or two", async ({ loggedIn: page }) => {
  // D-76: the probe asks for commands on its own loop and the hub holds that
  // request until one is queued. Queued just after a report, a command that
  // waited for the next report would take about 5 s (the interval here);
  // through the held poll it is done well before.
  const id = process.env.E2E_PROBE_ID;
  const lastSeen = async () =>
    ((await (await page.request.get("/api/probes")).json()) as { id: string; last_seen_at: string | null }[]).find(
      (p) => p.id === id,
    )?.last_seen_at;
  const before = await lastSeen();
  await expect.poll(lastSeen, { timeout: 15_000, intervals: [100] }).not.toBe(before);

  const started = Date.now();
  const queued = await page.request.post(`/api/probes/${id}/commands`, {
    headers: { "x-blentinel": "1" },
    data: { kind: "run_checks" },
  });
  expect(queued.ok()).toBeTruthy();
  const command = (await queued.json()) as { id: string };
  await expect
    .poll(
      async () =>
        ((await (await page.request.get(`/api/probes/${id}/commands`)).json()) as { id: string; state: string }[]).find(
          (c) => c.id === command.id,
        )?.state,
      { timeout: 3_000, intervals: [100] },
    )
    .toBe("succeeded");
  expect(Date.now() - started).toBeLessThan(3_000);
});

test("a Viewer sees the checks but can't change them", async ({ viewer: page }) => {
  await page.goto(probePage());
  await expect(page.locator(".checks-table tbody tr", { hasText: "Hub port" })).toHaveCount(1);
  await expect(page.getByRole("button", { name: "Add check" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Edit" })).toHaveCount(0);
  await expect(page.locator("#interval_secs")).toBeDisabled();
  await expect(page.getByRole("button", { name: "Run checks now" })).toHaveCount(0);
});
