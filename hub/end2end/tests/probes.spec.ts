import { createHash } from "node:crypto";
import * as fs from "node:fs";
import { COMPANY, PROBE_NAME, expect, test } from "./fixtures";

test("the probe trusts the hub's signed answers", async () => {
  // Every report is answered with an ack sealed for the probe and signed by
  // the hub (D-48); the probe logs a warning for any it can't trust.
  const log = fs.readFileSync(process.env.E2E_PROBE_LOG!, "utf8");
  expect(log).toContain("Report delivered to hub");
  expect(log).not.toContain("can't be trusted");
  expect(log).not.toContain("clock is off");
});

test("the probe runs the checks the hub gave it", async ({ loggedIn: page }) => {
  // Checks are hub records, sent in the signed answer to a report (D-49):
  // the probe has no config file, only the config.json it received.
  const res = await page.request.get(`/api/probes/${process.env.E2E_PROBE_ID}/checks`);
  expect(res.ok()).toBe(true);
  const state = await res.json();
  expect(state.sync).toBe("in_sync");
  expect(state.applied_revision).toBe(state.config_revision);
  expect(state.interval_secs).toBe(5);
  expect(state.checks.map((c: { name: string }) => c.name).sort()).toEqual(["Closed port", "Hub certificate", "Hub ping", "Hub port", "Hub redirect", "Probe program", "System service"]);

  const log = fs.readFileSync(process.env.E2E_PROBE_LOG!, "utf8");
  expect(log).toContain("New config from the hub");
  expect(log).not.toContain("can't use");
});

test("the probe says how it is doing, and the fleet view shows it", async ({ loggedIn: page }) => {
  // Each report carries the probe's health (protocol v8, D-73): when it
  // started, the reports it holds back, its clock against the hub's.
  const probes = await (await page.request.get("/api/probes")).json();
  const probe = probes.find((p: { id: string }) => p.id === process.env.E2E_PROBE_ID);
  expect(probe.health).toBeTruthy();
  expect(probe.health.queued_reports).toBe(0);
  expect(Math.abs(probe.health.clock_offset_secs)).toBeLessThanOrEqual(5);
  expect(probe.config_sync).toBeTruthy();

  await page.goto("/probes");
  await page.locator("html[data-hydrated]").waitFor({ state: "attached" });
  const row = page.locator(".fleet-table tbody tr", { hasText: PROBE_NAME });
  await expect(row).toHaveCount(1);
  await expect(row).toContainText(probe.probe_version);
  await expect(row).toContainText(COMPANY);
  await expect(row.locator(".fleet-badge")).toHaveText(/^(OK|ATTENTION)$/);

  // The filter narrows the list; "only those that need attention" keeps
  // the probe exactly when its badge says so.
  const filter = page.locator('input[name="fleet_filter"]');
  await filter.fill("no-such-probe-anywhere");
  await expect(row).toHaveCount(0);
  await filter.fill(PROBE_NAME);
  await expect(row).toHaveCount(1);
  const attention = (await row.locator(".fleet-badge").textContent()) === "ATTENTION";
  await page.locator('input[name="fleet_attention"]').check();
  await expect(row).toHaveCount(attention ? 1 : 0);

  // The probe's own page has its health too.
  await page.goto(`/probes/${process.env.E2E_PROBE_ID}`);
  const health = page.locator(".probe-health");
  await expect(health).toContainText("Running since");
  await expect(health).toContainText("Reports held back");
});

test("the company page lists the enrolled probe", async ({ loggedIn: page }) => {
  await page.goto(`/companies/${COMPANY}`);
  const row = page.locator(".probes-table tr", { hasText: PROBE_NAME });
  await expect(row).toContainText("Active");
  await expect(row).toContainText("lab");
});

test("Add probe shows a code once, and it can be revoked", async ({ loggedIn: page }) => {
  await page.goto(`/companies/${COMPANY}`);
  await page.getByRole("button", { name: "Add probe" }).click();
  await expect(page.locator('input[name="code_hub_url"]')).toHaveValue(/^https:\/\/127\.0\.0\.1:\d+$/);
  await page.locator('input[name="code_probe_name"]').fill("spare-probe");
  await page.getByRole("button", { name: "Create code" }).click();

  const code = page.locator(".enrollment-code");
  await expect(code).toHaveText(/^BLT1-/);
  const codeText = (await code.textContent())?.trim() ?? "";

  // One line per probe binary the hub has (D-53), checked by its SHA-256.
  const [download] = await (await page.request.get("/api/probe-downloads")).json();
  const line = page.locator(".install-command");
  await expect(line).toHaveCount(1);
  await expect(line).toContainText(download.os === "windows" ? "Windows" : "Linux");
  const command = line.locator(".enrollment-command");
  await expect(command).toContainText(`install --code ${codeText}`);
  await expect(command).toContainText(`/download/probe/${download.target}`);
  const shown = download.os === "windows" ? download.sha256.toUpperCase() : download.sha256;
  await expect(command).toContainText(shown);

  // The download is what the command checks for.
  const file = await page.request.get(`/download/probe/${download.target}`);
  expect(file.ok()).toBe(true);
  const hash = createHash("sha256").update(await file.body()).digest("hex");
  expect(hash).toBe(download.sha256);

  const pending = page.locator(".codes-table tr", { hasText: "spare-probe" });
  await expect(pending).toContainText("0 (no limit)");
  await pending.getByRole("button", { name: "Revoke" }).click();
  await expect(pending).toHaveCount(0);
});

test("Add probe waits for the company's first site", async ({ loggedIn: page }) => {
  const res = await page.request.post("/api/companies", { data: { id: "no-sites-yet", name: "No sites yet" } });
  expect(res.ok()).toBe(true);
  await page.goto("/companies/no-sites-yet");
  await expect(page.getByRole("button", { name: "Add probe" })).toBeDisabled();
  await expect(page.locator(".add-probe-hint")).toBeVisible();

  await page.locator("html[data-hydrated]").waitFor({ state: "attached" });
  await page.locator('input[name="new_site_name"]').fill("Office");
  await page.getByRole("button", { name: "Add site" }).click();
  await expect(page.getByRole("button", { name: "Add probe" })).toBeEnabled();
  await expect(page.locator(".add-probe-hint")).toHaveCount(0);

  // Leave the dashboard as the other tests expect it.
  await page.request.post("/api/companies/no-sites-yet/archive", { data: { archived: true } });
});

test("a Viewer sees the probes but can't add any", async ({ viewer: page }) => {
  await page.goto(`/companies/${COMPANY}`);
  await expect(page.locator(".probes-table")).toContainText(PROBE_NAME);
  await expect(page.getByRole("button", { name: "Add probe" })).toHaveCount(0);
});
