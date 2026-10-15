// Builds what the browser tests run: the probe, and the hub with its web UI
// embedded, as `blentinelmake hub build` does (D-51): the front end first,
// then the server with BLENTINEL_EMBED_SITE (hub/build.rs).
//
// The hub goes to its own target folder (target/e2e): a plain `cargo build`
// of the hub (e.g. by `cargo test-all`) would overwrite target/debug/hub
// without the web UI.

//
// Both trust the test release key (release-key.json, D-55), so the setup can
// give the hub a signed release.

import { execSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const { seed } = JSON.parse(fs.readFileSync(path.join(here, "release-key.json"), "utf8"));
// Ed25519 private key from its 32-byte seed (PKCS#8 prefix), then the raw public key.
const key = crypto.createPrivateKey({
  key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), Buffer.from(seed, "hex")]),
  format: "der",
  type: "pkcs8",
});
const publicKey = Buffer.from(crypto.createPublicKey(key).export({ format: "jwk" }).x, "base64url").toString("hex");
const keys = { BLENTINEL_RELEASE_KEYS: publicKey };

const run = (command, cwd, env = {}) =>
  execSync(command, { cwd, stdio: "inherit", env: { ...process.env, ...keys, ...env } });

run("cargo build -p probe", root);
const hub = path.join(root, "hub");
const targetDir = path.join(root, "target", "e2e");
run("cargo leptos build --frontend-only", hub, { CARGO_TARGET_DIR: targetDir });
run("cargo leptos build --server-only", hub, {
  CARGO_TARGET_DIR: targetDir,
  BLENTINEL_EMBED_SITE: path.join(root, "target", "site"),
});
