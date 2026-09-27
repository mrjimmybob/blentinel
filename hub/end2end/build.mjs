// Builds what the browser tests run: the probe, and the hub with its web UI.
//
// The hub goes to its own target folder (target/e2e): cargo-leptos compiles
// the web-asset names into the hub, and a plain `cargo build` of the hub (e.g.
// by `cargo test-all`) would overwrite target/debug/hub without them.

import { execSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const run = (command, cwd, env = {}) =>
  execSync(command, { cwd, stdio: "inherit", env: { ...process.env, ...env } });

run("cargo build -p probe", root);
run("cargo leptos build", path.join(root, "hub"), {
  CARGO_TARGET_DIR: path.join(root, "target", "e2e"),
});
