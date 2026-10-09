import { readFileSync } from "node:fs";
import path from "node:path";
import { COMPANY, PASSWORD, PROBE_NAME, expect, test } from "./fixtures";

// The version the hub was built with: [workspace.package] in the root Cargo.toml.
const cargoToml = readFileSync(path.resolve(__dirname, "../../../Cargo.toml"), "utf8");
const version = cargoToml.match(/\[workspace\.package\][^[]*?version = "([^"]+)"/)?.[1] ?? "";

test("the admin page shows the real hub version and the probe", async ({ loggedIn: page }) => {
  expect(version, "workspace version in Cargo.toml").not.toBe("");
  await page.goto("/admin");

  const versionSection = page.locator(".admin-section", { hasText: "Hub Version" });
  await expect(versionSection).toContainText(version);
  await expect(versionSection).not.toContainText("Build");

  await expect(page.locator("body")).toContainText(COMPANY);
});

test("the web UI comes from inside the hub binary, cached by its fingerprint", async ({ page }) => {
  // The hub runs from a folder with nothing but its config (global-setup.ts),
  // so these can only be the embedded files (D-51).
  const wasm = await page.request.get("/pkg/hub.wasm");
  expect(wasm.status()).toBe(200);
  expect(wasm.headers()["content-type"]).toBe("application/wasm");
  const etag = wasm.headers()["etag"];
  expect(etag).toMatch(/^"[0-9a-f]{16}"$/);

  const again = await page.request.get("/pkg/hub.wasm", { headers: { "if-none-match": etag } });
  expect(again.status()).toBe(304);
  expect((await page.request.get("/pkg/no-such-file.js")).status()).toBe(404);
});

// One rollout per hub: these two must not run at the same time.
test.describe.serial("probe updates", () => {
  test("the hub's release can be approved and updates stopped", async ({ loggedIn: page }) => {
    // The setup gave the hub a signed release of the probe under test (D-55).
    await page.goto("/admin");
    const section = page.locator(".probe-updates");
    await expect(section).toContainText("Updates are off");
    const release = section.locator("table.releases tr", { hasText: version });
    await expect(release).toContainText(process.platform === "win32" ? "windows x86_64" : "linux x86_64");
    // The probe runs that very version, so it counts there and nothing is offered to it.
    await expect(section.locator("table.probe-versions")).toContainText(PROBE_NAME);
    await expect(release.locator("td").nth(3)).toHaveText("1");

    await release.getByRole("button", { name: "Approve" }).click();
    await expect(section).toContainText(`Probes are offered ${version}`);
    await expect(section).toContainText("Rolling out to every probe.");
    await expect(release).toContainText("approved");
    await expect(release.getByRole("button", { name: "Remove" })).toHaveCount(0);
    const probe = section.locator("table.probe-versions tr", { hasText: PROBE_NAME });
    await expect(probe.locator(".update-status-cell")).toHaveText("Up to date");

    await section.getByRole("button", { name: "Stop updates" }).click();
    await expect(section).toContainText("Updates are off");
  });

  test("canaries first, pause, roll out now, and a company's policy", async ({ loggedIn: page }) => {
    // D-74: the probe under test as the canary; it already runs the release.
    await page.goto("/admin");
    const section = page.locator(".probe-updates");
    const probe = section.locator("table.probe-versions tr", { hasText: PROBE_NAME });
    await probe.locator("input[name=canary]").check();
    await expect(probe.locator("input[name=canary]")).toBeChecked();

    await section.locator("input[name=soak_minutes]").fill("120");
    const release = section.locator("table.releases tr", { hasText: version });
    await release.getByRole("button", { name: "Approve" }).click();
    await expect(section).toContainText("Canaries first: 1 of 1 canaries run it.");

    await section.getByRole("button", { name: "Pause" }).click();
    await expect(section).toContainText(/Paused by \S+ on .*: paused by/);
    await section.getByRole("button", { name: "Resume" }).click();
    await expect(section).toContainText("Canaries first");
    await section.getByRole("button", { name: "Roll out to all now" }).click();
    await expect(section).toContainText("Rolling out to every probe (sent by");

    // The probe's company takes no updates; then follows again, in a window.
    const company = section.locator("table.company-updates tr", { hasText: COMPANY });
    await company.locator("select[name=update_mode]").selectOption("off");
    await company.locator("button[name=update_policy_save]").click();
    await expect(probe.locator(".update-status-cell")).toHaveText("Off for its company");
    await page.reload();
    await expect(company.locator("select[name=update_mode]")).toHaveValue("off");

    // A window that starts and ends at the same time is refused in the page.
    await company.locator("select[name=update_mode]").selectOption("follow");
    await company.locator("input[name=window_start]").fill("02:00");
    await company.locator("input[name=window_end]").fill("02:00");
    await company.locator("button[name=update_policy_save]").click();
    await expect(section.locator(".error-message")).toContainText("Not saved");
    await company.locator("input[name=window_end]").fill("05:00");
    await company.locator("button[name=update_policy_save]").click();
    await expect(probe.locator(".update-status-cell")).toHaveText("Up to date");
    await page.reload();
    await expect(company.locator("input[name=window_end]")).toHaveValue("05:00");

    // Leave the hub as it was for the other tests.
    await company.locator("input[name=window_start]").fill("");
    await company.locator("input[name=window_end]").fill("");
    await company.locator("button[name=update_policy_save]").click();
    await expect(company.locator("input[name=window_start]")).toHaveValue("");
    await probe.locator("input[name=canary]").uncheck();
    await section.getByRole("button", { name: "Stop updates" }).click();
    await expect(section).toContainText("Updates are off");
    await expect(probe.locator("input[name=canary]")).not.toBeChecked();
  });
});

test("backups: back up now, set the schedule, delete a backup", async ({ loggedIn: page }) => {
  await page.goto("/admin");
  const section = page.locator(".admin-section.backups");
  await expect(section).toContainText("Daily at 03:00 UTC; the last 7 kept.");
  await expect(section).toContainText("holds the hub's private key");

  // The scheduler may add a scheduled backup meanwhile: count manual ones only.
  const manual = section.locator("table.backup-list tr", { hasText: "manual" });
  const before = await manual.count();
  await section.getByRole("button", { name: "Back up now" }).click();
  await expect(manual).toHaveCount(before + 1);
  await expect(manual.first()).toContainText(/\d+(\.\d)? (KB|MB)/);

  // A schedule the hub refuses never leaves the page.
  await section.locator("input[name=backup_keep]").fill("0");
  await section.locator("button[name=backup_save]").click();
  await expect(section).toContainText("Not saved: keep 1 to 365 backups.");

  await section.locator("select[name=backup_hour]").selectOption("22");
  await section.locator("input[name=backup_keep]").fill("5");
  await section.locator("button[name=backup_save]").click();
  await expect(section).toContainText("Daily at 22:00 UTC; the last 5 kept.");
  await page.reload();
  await expect(section).toContainText("Daily at 22:00 UTC; the last 5 kept.");

  // Downloading asks for the password again (the backup holds the hub's key).
  const row = manual.first();
  await row.getByRole("button", { name: "Download" }).click();
  await row.locator("input[name=backup_password]").fill("wrong-password");
  await row.locator("button[name=backup_download]").click();
  await expect(section.locator(".error-message")).toBeVisible();
  await row.locator("input[name=backup_password]").fill(PASSWORD);
  const [download] = await Promise.all([
    page.waitForEvent("download"),
    row.locator("button[name=backup_download]").click(),
  ]);
  expect(download.suggestedFilename()).toMatch(/^blentinel-hub-\d{8}-\d{6}\.tar\.gz$/);
  await expect(row.getByRole("button", { name: "Download" })).toBeVisible();

  await row.getByRole("button", { name: "Delete" }).click();
  await row.getByRole("button", { name: "No" }).click();
  await expect(manual).toHaveCount(before + 1);
  await row.getByRole("button", { name: "Delete" }).click();
  await row.getByRole("button", { name: "Yes" }).click();
  await expect(manual).toHaveCount(before);

  // Put the default back for anything after this test.
  await section.locator("select[name=backup_hour]").selectOption("3");
  await section.locator("input[name=backup_keep]").fill("7");
  await section.locator("button[name=backup_save]").click();
  await expect(section).toContainText("Daily at 03:00 UTC; the last 7 kept.");
});

test("HTTPS certificate: a failed request is shown, and the hub keeps its own certificate", async ({ loggedIn: page }) => {
  await page.goto("/admin");
  const section = page.locator(".admin-section.certificate");
  await expect(section.locator(".certificate-status")).toContainText("Self-signed");

  // Refused in the page: a name that can't have a public certificate.
  await section.locator("input[name=acme_enabled]").check();
  await section.locator("input[name=acme_domains]").fill("127.0.0.1");
  await section.locator("input[name=acme_terms]").check();
  await section.locator("button[name=acme_save]").click();
  await expect(section).toContainText("Not saved: 127.0.0.1: an IP address can't have a public certificate");

  // An ACME directory nothing answers at: saved, the reason shown, nothing replaced.
  await section.locator("input[name=acme_domains]").fill("hub.example.com");
  await section.locator("select[name=acme_authority]").selectOption("custom");
  await section.locator("input[name=acme_directory]").fill("https://127.0.0.1:1/directory");
  await section.locator("button[name=acme_save]").click();
  await expect(section.locator(".error-message")).toContainText("Saved, but no certificate yet");
  await expect(section.locator(".certificate-failed")).toContainText("can't open an account with https://127.0.0.1:1/directory");
  await expect(section.locator(".certificate-status")).toContainText("Self-signed");
  await expect(section.locator("input[name=acme_domains]")).toHaveValue("hub.example.com");

  // "Renew now" asks again and says why it failed.
  await section.locator("button[name=acme_renew]").click();
  await expect(section.locator(".error-message")).toContainText("can't open an account");

  // Off again: nothing to renew.
  await section.locator("input[name=acme_enabled]").uncheck();
  await section.locator("button[name=acme_save]").click();
  await expect(section.locator(".success-message")).toHaveText("Saved.");
  await expect(section.locator("button[name=acme_renew]")).toHaveCount(0);
  await expect(section.locator(".certificate-status")).toContainText("Self-signed");
});

test("thresholds: the hub's defaults show what applies, refuse nonsense and save", async ({ loggedIn: page }) => {
  await page.goto("/admin");
  const section = page.locator(".admin-section.thresholds");
  // Nothing set: the built-in defaults show as placeholders, with where they come from.
  const diskWarning = section.locator("input[name=disk_warning]");
  await expect(diskWarning).toHaveValue("");
  await expect(diskWarning).toHaveAttribute("placeholder", "90%");
  await expect(diskWarning).toHaveAttribute("title", "from the built-in default");
  await expect(section.locator("input[name=latency_down]")).toHaveAttribute("placeholder", "off");

  await diskWarning.fill("95");
  await section.locator("input[name=disk_down]").fill("90");
  await section.locator("button[name=thresholds_save]").click();
  await expect(section.locator(".error-message")).toContainText("below the down limit");

  // Memory only: the browser tests' probe has no memory check.
  await diskWarning.fill("");
  await section.locator("input[name=disk_down]").fill("");
  await section.locator("input[name=memory_warning]").fill("75");
  await section.locator("button[name=thresholds_save]").click();
  await expect(section.locator(".success-message")).toContainText("Saved");
  await page.reload();
  await expect(section.locator("input[name=memory_warning]")).toHaveValue("75");

  // Back to nothing set, for the tests after this one.
  await section.locator("input[name=memory_warning]").fill("");
  await section.locator("button[name=thresholds_save]").click();
  await expect(section.locator(".success-message")).toContainText("Saved");
});
