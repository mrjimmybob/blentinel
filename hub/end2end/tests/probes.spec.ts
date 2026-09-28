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
  await expect(page.locator(".enrollment-command")).toContainText("probe enroll BLT1-");

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
