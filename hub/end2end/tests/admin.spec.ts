import { readFileSync } from "node:fs";
import path from "node:path";
import { COMPANY, PROBE_NAME, expect, test } from "./fixtures";

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

test("probe updates: the hub's release can be approved and updates stopped", async ({ loggedIn: page }) => {
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
  await expect(release).toContainText("approved");
  await expect(release.getByRole("button", { name: "Remove" })).toHaveCount(0);

  await section.getByRole("button", { name: "Stop updates" }).click();
  await expect(section).toContainText("Updates are off");
});
