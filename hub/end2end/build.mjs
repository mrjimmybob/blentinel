// Builds what the browser tests run: the probe, and the hub with its web UI
// embedded, as `blentinelmake hub build` does (D-51): the front end first,
// then the server with BLENTINEL_EMBED_SITE (hub/build.rs).
//
// The hub goes to its own target folder (target/e2e): a plain `cargo build`
// of the hub (e.g. by `cargo test-all`) would overwrite target/debug/hub
// without the web UI.

import { execSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const run = (command, cwd, env = {}) =>
  execSync(command, { cwd, stdio: "inherit", env: { ...process.env, ...env } });

run("cargo build -p probe", root);
const hub = path.join(root, "hub");
const targetDir = path.join(root, "target", "e2e");
run("cargo leptos build --frontend-only", hub, { CARGO_TARGET_DIR: targetDir });
run("cargo leptos build --server-only", hub, {
  CARGO_TARGET_DIR: targetDir,
  BLENTINEL_EMBED_SITE: path.join(root, "target", "site"),
});
