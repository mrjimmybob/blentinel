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
