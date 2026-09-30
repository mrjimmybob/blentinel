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
  expect(state.checks.map((c: { name: string }) => c.name).sort()).toEqual(["Closed port", "Hub port"]);

  const log = fs.readFileSync(process.env.E2E_PROBE_LOG!, "utf8");
  expect(log).toContain("New config from the hub");
  expect(log).not.toContain("can't use");
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
  await expect(pending).toContainText("0 / 1");
  await pending.getByRole("button", { name: "Revoke" }).click();
  await expect(pending).toHaveCount(0);
});

test("a Viewer sees the probes but can't add any", async ({ viewer: page }) => {
  await page.goto(`/companies/${COMPANY}`);
  await expect(page.locator(".probes-table")).toContainText(PROBE_NAME);
  await expect(page.getByRole("button", { name: "Add probe" })).toHaveCount(0);
});
