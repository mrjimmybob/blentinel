// Starts a real hub and a real, registered probe for the browser tests, the
// same way a user would: `hub --init`, first probe run to get its ID,
// [[probes]] entry (hot-reloaded), then the probe reporting every 5 s.
//
// Needs `npm run build` (build.mjs): the probe in target/debug, and the hub
// built by cargo-leptos in target/e2e with its web UI in target/site.

import { ChildProcess, execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { COMPANY, PROBE_NAME } from "./tests/fixtures";
const PORT = Number(process.env.E2E_PORT ?? 3999);
const ROOT = path.resolve(__dirname, "../..");
const BASE = `http://127.0.0.1:${PORT}`;

const exe = (name: string) => (process.platform === "win32" ? `${name}.exe` : name);
const HUB = path.join(ROOT, "target", "e2e", "debug", exe("hub"));
const PROBE = path.join(ROOT, "target", "debug", exe("probe"));
const WEB_UI = path.join(ROOT, "target", "site", "pkg");

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function waitFor(what: string, check: () => Promise<boolean>, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await check().catch(() => false)) return;
    await sleep(500);
  }
  throw new Error(`Timed out waiting for ${what}`);
}

function start(name: string, file: string, args: string[], cwd: string, log: string): ChildProcess {
  const out = fs.openSync(log, "w");
  const child = spawn(file, args, { cwd, stdio: ["ignore", out, out] });
  child.on("exit", (code) => {
    if (code !== null && code !== 0) console.error(`${name} exited with ${code}; see ${log}`);
  });
  return child;
}

/** Fail fast if the page needs web-UI files the hub can't serve (a hub not built by cargo-leptos). */
async function checkWebUi() {
  const html = await (await fetch(`${BASE}/login`)).text();
  const assets = new Set([...html.matchAll(/\/pkg\/[\w.-]+\.(?:js|wasm)/g)].map((m) => m[0]));
  for (const asset of assets) {
    if (!(await fetch(`${BASE}${asset}`)).ok) {
      throw new Error(`The hub's pages need ${asset}, which it can't serve. Run \`npm run build\`.`);
    }
  }
}

export default async function globalSetup() {
  for (const file of [HUB, PROBE, path.join(WEB_UI, "hub.wasm")]) {
    if (!fs.existsSync(file)) throw new Error(`${file} is missing. Run \`npm run build\` first.`);
  }
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "blentinel-e2e-"));
  const hubDir = path.join(dir, "hub");
  const probeDir = path.join(dir, "probe");
  fs.mkdirSync(hubDir);
  fs.mkdirSync(probeDir);

  // Hub: the --init template, on our port, with its state in the temp dir.
  execFileSync(HUB, ["--init"], { cwd: hubDir });
  const hubConfig = path.join(hubDir, "blentinel_hub.toml");
  const toml = fs
    .readFileSync(hubConfig, "utf8")
    .replace(/^port = .*$/m, `port = ${PORT}`) // first match only: [server]
    .replace(/^state_dir = .*$/m, `state_dir = "${hubDir.replace(/\\/g, "/")}"`);
  fs.writeFileSync(hubConfig, toml);
  // Run from the workspace root, like a developer would.
  const hub = start("hub", HUB, ["--config", hubConfig], ROOT, path.join(dir, "hub.log"));
  await waitFor("the hub", async () => (await fetch(`${BASE}/api/handshake`)).ok);
  await checkWebUi();

  // Probe: own folder (it keeps its identity next to its binary).
  const probeExe = path.join(probeDir, path.basename(PROBE));
  fs.copyFileSync(PROBE, probeExe);
  fs.writeFileSync(
    path.join(probeDir, "blentinel_probe.toml"),
    `[agent]
company_id = "${COMPANY}"
hub_url = "${BASE}"
interval = 5
site = "lab"

[[resources]]
name = "Hub port"
type = "tcp"
target = "127.0.0.1:${PORT}"

[[resources]]
name = "Closed port"
type = "tcp"
target = "127.0.0.1:1"
`,
  );
  // First run prints the probe's public key (its ID) and exits.
  const firstRun = execFileSync(probeExe, [], { cwd: probeDir, encoding: "utf8", timeout: 60_000 });
  const probeId = firstRun.match(/\b[0-9a-f]{64}\b/)?.[0];
  if (!probeId) throw new Error(`The probe printed no ID:\n${firstRun}`);
  fs.appendFileSync(hubConfig, `\n[[probes]]\nname = "${PROBE_NAME}"\npublic_key = "${probeId}"\n`);
  await sleep(2000); // hot reload of [[probes]]
  const probe = start("probe", probeExe, [], probeDir, path.join(dir, "probe.log"));

  // Ready once the hub has stored a report from the probe.
  const token = fs.readFileSync(path.join(hubDir, "hub_auth.token"), "utf8").trim();
  const login = await fetch(`${BASE}/api/login`, { method: "POST", body: token });
  const cookie = login.headers.get("set-cookie")?.split(";")[0] ?? "";
  await waitFor("the first report", async () => {
    const res = await fetch(`${BASE}/api/company/${COMPANY}/probes`, { headers: { cookie } });
    return res.ok && ((await res.json()) as unknown[]).length > 0;
  });

  // Visible to the tests (workers inherit the environment).
  process.env.E2E_BASE_URL = BASE;
  process.env.E2E_TOKEN = token;
  process.env.E2E_PROBE_ID = probeId;

  return async () => {
    probe.kill();
    hub.kill();
    await sleep(500);
    if (process.env.E2E_KEEP) {
      console.log(`Kept ${dir}`);
    } else {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  };
}
