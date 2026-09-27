import { readFileSync } from "node:fs";
import path from "node:path";
import { COMPANY, expect, test } from "./fixtures";

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
