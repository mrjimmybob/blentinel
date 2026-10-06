// Rotating the hub's certificate and key (M2-13, D-61) with the real probe:
// it takes the next pins from the hub's signed answers, the hub switches its
// HTTPS certificate and key in place, and the probe carries on reporting.
// Runs after every other test (the "rotation" project in
// playwright.config.ts): a switch changes what the other tests rely on.

import { readFileSync } from "node:fs";
import * as tls from "node:tls";
import { PROBE_NAME, expect, test } from "./fixtures";

const port = Number(process.env.E2E_PORT ?? 3999) + 1;

/** SHA-256 of the certificate the hub serves on HTTPS right now. */
function servedCertificate(): Promise<string> {
  return new Promise((resolve, reject) => {
    const socket = tls.connect({ host: "127.0.0.1", port, rejectUnauthorized: false }, () => {
      resolve(socket.getPeerCertificate().fingerprint256);
      socket.end();
    });
    socket.on("error", reject);
  });
}

test("a certificate and key rotation reaches the real probe, which keeps reporting", async ({ loggedIn: page }) => {
  test.setTimeout(120_000);
  const keys = async () => (await (await page.request.get("/api/admin/hub-keys")).json()) as any;
  const before = await keys();
  const certBefore = await servedCertificate();

  await page.goto("/admin");
  const section = page.locator(".admin-section.hub-keys");
  await expect(section).toContainText("Probes pin the hub's key");

  // A window the hub refuses never leaves the page.
  await section.locator("input[name=rotate_key]").check();
  await section.locator("input[name=rotate_window]").fill("0");
  await section.locator("button[name=rotation_start]").click();
  await expect(section).toContainText("Not started: the window is 1 to 90 days.");

  await section.locator("input[name=rotate_tls]").check();
  await section.locator("input[name=rotate_window]").fill("14");
  await section.locator("button[name=rotation_start]").click();
  await expect(section).toContainText("Rotating the certificate and key");

  // The probe reports every 5 s: one answer brings the next pins, the
  // following report says it holds them.
  await expect
    .poll(async () => (await keys()).rotation?.ready, { timeout: 60_000 })
    .toBe(1);
  await page.reload();
  await expect(section).toContainText("1 of 1 probes hold the new keys");

  await section.locator("button[name=rotation_switch]").click();
  await expect(section.locator("table.rotation-history")).toContainText("switched");
  const after = await keys();
  expect(after.rotation).toBeNull();
  expect(after.hub_key).not.toBe(before.hub_key);
  expect(after.tls_spki_sha256).not.toBe(before.tls_spki_sha256);
  expect(await servedCertificate(), "the new certificate, without a restart").not.toBe(certBefore);

  // The probe follows: it says so, and keeps delivering reports.
  const log = process.env.E2E_PROBE_LOG!;
  await expect
    .poll(() => readFileSync(log, "utf8").includes("The hub switched to its new keys"), { timeout: 30_000 })
    .toBe(true);
  const lastSeen = async () => {
    const probes = (await (await page.request.get("/api/probes")).json()) as any[];
    return probes.find((p) => p.name === PROBE_NAME)?.last_seen_at as string;
  };
  const seenAtSwitch = await lastSeen();
  await expect.poll(lastSeen, { timeout: 30_000 }).not.toBe(seenAtSwitch);
  expect(readFileSync(log, "utf8")).not.toMatch(/can't be trusted|isn't the one this probe pins/);
});
