// The probe's machine, the checks it suggests, and check templates (M5-12,
// D-85). The real probe sent its real inventory with its first report.
// Nothing here changes the checks the probe runs (other tests watch them):
// a template is applied where the probe already has every check of it.
import { expect, test } from "./fixtures";

const probePage = () => `/probes/${process.env.E2E_PROBE_ID}`;

test("the probe's page shows what its machine is made of", async ({ loggedIn: page }) => {
  // What the hub keeps: the system, its disks, its running services.
  const machine = await (await page.request.get(`/api/probes/${process.env.E2E_PROBE_ID}/machine`)).json();
  expect(machine.inventory.os.name).toBeTruthy();
  expect(["windows", "linux"]).toContain(machine.inventory.os.family);
  expect(machine.inventory.volumes.length).toBeGreaterThan(0);
  expect(machine.inventory.services.length).toBeGreaterThan(0);

  await page.goto(probePage());
  const section = page.locator(".machine-section");
  await expect(section).toContainText(machine.inventory.os.name);
  await expect(section).toContainText(machine.inventory.volumes[0].mount);
  await expect(section.locator(".machine-services summary")).toHaveText(/^\d+ running$/);
  // The list of services opens.
  await section.locator(".machine-services summary").click();
  await expect(section.locator(".machine-services li").first()).toBeVisible();
});

test("checks are suggested from the machine: what the probe doesn't check yet", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  const suggested = page.locator(".suggested-checks");
  // The test probe checks no CPU, memory, disk or uptime: all four are proposed.
  for (const name of ["CPU", "Memory", "Disks", "Uptime", "Pending updates"]) {
    await expect(suggested.locator("li", { hasText: name }).first()).toBeVisible();
  }
  await expect(suggested.locator("li", { hasText: "Disks" })).toContainText("its fixed disks");
  await expect(suggested.getByRole("button", { name: "Add all suggested" })).toBeVisible();
  // It already asks about a service and pings: neither is proposed twice.
  await expect(suggested.locator("li", { hasText: "System service" })).toHaveCount(0);
});

test("a built-in template asks for its value before it adds anything", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  await page.getByRole("button", { name: "Templates…" }).click();
  const dialog = page.locator(".templates-dialog");
  for (const name of ["Machine baseline", "Website", "Backup freshness", "Domain controller"]) {
    await expect(dialog.locator(".template-card h4", { hasText: name })).toBeVisible();
  }
  const website = dialog.locator(".template-card", { hasText: "A site as its visitors meet it" });
  await expect(website).toContainText("Checks: {host} page, {host} certificate, {host} DNS");
  await website.getByRole("button", { name: "Add to this probe" }).click();
  await expect(dialog.locator(".login-error")).toContainText("Host name is needed (e.g. www.example.com)");
  await website.locator("input").fill("not a host name");
  await website.getByRole("button", { name: "Add to this probe" }).click();
  await expect(dialog.locator(".login-error")).toContainText("not a host name page:");
  await dialog.getByRole("button", { name: "Close" }).click();
  await expect(dialog).toHaveCount(0);
});

// M5-09, D-87. Nothing is added: an SNMP check made here would run.
test("the SNMP templates ask for the device and its community, typed like a password", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  await page.getByRole("button", { name: "Templates…" }).click();
  const dialog = page.locator(".templates-dialog");
  for (const name of [
    "Printer (SNMP)",
    "Switch or router ports (SNMP)",
    "Synology NAS (SNMP)",
    "QNAP NAS (SNMP)",
    "APC UPS (SNMP)",
  ]) {
    await expect(dialog.locator(".template-card h4", { hasText: name })).toBeVisible();
  }
  const printer = dialog.locator(".template-card", { hasText: "A network printer" });
  await expect(printer).toContainText("Checks: {host} printer");
  await expect(printer.locator("input")).toHaveCount(2);
  await expect(printer.locator("input").nth(0)).toHaveAttribute("type", "text");
  await expect(printer.locator("input").nth(1)).toHaveAttribute("type", "password");
  await printer.locator("input").nth(0).fill("192.168.1.9");
  await printer.getByRole("button", { name: "Add to this probe" }).click();
  await expect(dialog.locator(".login-error")).toContainText("SNMP community (v2c) is needed");
  await dialog.getByRole("button", { name: "Close" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: "192.168.1.9 printer" })).toHaveCount(0);
});

test("a saved template is applied: the probe keeps the checks it already has", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  // The checks are there before anything is applied.
  await expect(page.locator(".checks-table tbody tr", { hasText: "Hub port" })).toHaveCount(1);
  await page.getByRole("button", { name: "Templates…" }).click();
  const dialog = page.locator(".templates-dialog");
  // Saved by the setup from this probe's own checks.
  const baseline = dialog.locator(".template-card", { hasText: "E2E baseline" });
  await expect(baseline).toContainText("saved by");
  await expect(baseline).toContainText("Hub port");
  await baseline.getByRole("button", { name: "Add to this probe" }).click();
  await expect(dialog.locator(".templates-message")).toHaveText(
    "E2E baseline: Nothing added: the probe already has these checks.",
  );
  await dialog.getByRole("button", { name: "Close" }).click();
  // Still one of each: nothing was doubled.
  await expect(page.locator(".checks-table tbody tr", { hasText: "Hub port" })).toHaveCount(1);
});

test("a probe's checks are saved as a template, and the template deleted", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  await page.getByRole("button", { name: "Templates…" }).click();
  const dialog = page.locator(".templates-dialog");
  const name = `E2E template ${Date.now()}`;

  await dialog.getByRole("button", { name: "Save as template" }).click();
  await expect(dialog.locator(".login-error")).toContainText("a template needs a name");
  await dialog.locator("#template_name").fill("website");
  await dialog.getByRole("button", { name: "Save as template" }).click();
  await expect(dialog.locator(".login-error")).toContainText("already exists");

  await dialog.locator("#template_name").fill(name);
  await dialog.locator("#template_description").fill("Saved by a browser test");
  await dialog.getByRole("button", { name: "Save as template" }).click();
  await expect(dialog.locator(".templates-message")).toHaveText("Template saved.");
  await expect(dialog.locator("#template_name")).toHaveValue("");
  const card = dialog.locator(".template-card", { hasText: name });
  await expect(card).toContainText("Saved by a browser test");
  await expect(card).toContainText("Hub certificate");

  await card.getByRole("button", { name: "Delete" }).click();
  await expect(dialog.locator(".templates-message")).toHaveText("Template deleted.");
  await expect(dialog.locator(".template-card", { hasText: name })).toHaveCount(0);
  // A built-in one has no Delete.
  const builtin = dialog.locator(".template-card", { hasText: "Machine baseline" });
  await expect(builtin.getByRole("button", { name: "Delete" })).toHaveCount(0);
});

test("a Viewer sees the machine, but no suggestions and no templates", async ({ viewer: page }) => {
  await page.goto(probePage());
  await expect(page.locator(".machine-section .machine-services summary")).toHaveText(/running/);
  await expect(page.locator(".suggested-checks")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Templates…" })).toHaveCount(0);
});
