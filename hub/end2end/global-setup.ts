// Starts a real hub and a real, enrolled probe for the browser tests, the
// same way a user would: `hub --init` (the UI on HTTPS, D-43), the setup
// wizard in a browser (with the code from setup.code), a company and site,
// "Add probe" (an enrollment code), `probe enroll <code>`, its checks and a
// 5 s interval set on the hub (the probe has no config file, D-49), then the
// probe reporting over HTTPS pinned by the code (D-47): its first answer
// brings the checks. Also creates a Viewer and a Client.
//
// Needs `npm run build` (build.mjs): the probe in target/debug, and the hub
// built by cargo-leptos in target/e2e with its web UI in target/site.

import { chromium } from "@playwright/test";
import { ChildProcess, execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { CLIENT, COMPANY, OWNER, PASSWORD, PROBE_NAME, VIEWER } from "./tests/fixtures";
const PORT = Number(process.env.E2E_PORT ?? 3999);
const HTTPS_PORT = PORT + 1;
const ROOT = path.resolve(__dirname, "../..");
// The web UI, the API and the probe: HTTPS, self-signed (the probe pins it).
const BASE = `https://127.0.0.1:${HTTPS_PORT}`;
// Node's fetch in this setup accepts the hub's self-signed certificate
// (test-only; the browser uses ignoreHTTPSErrors).
process.env.NODE_TLS_REJECT_UNAUTHORIZED = "0";
const CSRF = { "x-blentinel": "1" };

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

/** The first-run wizard, as a new user would do it: /login sends them to /setup. */
async function runSetupWizard(code: string) {
  const browser = await chromium.launch({ channel: process.env.E2E_BROWSER_CHANNEL ?? "chrome" });
  try {
    const page = await browser.newPage({ ignoreHTTPSErrors: true });
    await page.goto(`${BASE}/login`);
    await page.waitForURL(/\/setup$/);
    await page.locator("html[data-hydrated]").waitFor({ state: "attached" });
    await page.getByPlaceholder("Setup code").fill(code);
    await page.getByPlaceholder("Username").fill(OWNER);
    await page.getByPlaceholder("Display name (optional)").fill("E2E Owner");
    await page.getByPlaceholder("Password (at least 12 characters)").fill(PASSWORD);
    await page.getByPlaceholder("Repeat the password").fill(PASSWORD);
    await page.getByRole("button", { name: "Create Owner" }).click();
    await page.waitForURL(`${BASE}/`);
  } finally {
    await browser.close();
  }
}

async function login(username: string, password = PASSWORD): Promise<string> {
  const res = await fetch(`${BASE}/api/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json", ...CSRF },
    body: JSON.stringify({ username, password }),
  });
  if (!res.ok) throw new Error(`login as ${username}: ${res.status} ${await res.text()}`);
  return res.headers.get("set-cookie")?.split(";")[0] ?? "";
}

/** A user with the shared test password (created with a temporary one, then changed). */
async function createUser(ownerCookie: string, username: string, role: string, scope?: unknown) {
  const post = (url: string, cookie: string, body: unknown) =>
    fetch(`${BASE}${url}`, {
      method: "POST",
      headers: { "content-type": "application/json", cookie, ...CSRF },
      body: JSON.stringify(body),
    });
  const created = await post("/api/users", ownerCookie, { username, display_name: "", role, scope });
  if (!created.ok) throw new Error(`create ${username}: ${await created.text()}`);
  const { temporary_password } = (await created.json()) as { temporary_password: string };
  const session = await login(username, temporary_password);
  const changed = await post("/api/me/password", session, {
    current_password: temporary_password,
    new_password: PASSWORD,
  });
  if (!changed.ok) throw new Error(`${username} password: ${await changed.text()}`);
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
    .replace(/^https_port = .*$/m, `https_port = ${HTTPS_PORT}`)
    .replace(/^state_dir = .*$/m, `state_dir = "${hubDir.replace(/\\/g, "/")}"`);
  fs.writeFileSync(hubConfig, toml);
  // Run from the workspace root, like a developer would.
  const hub = start("hub", HUB, ["--config", hubConfig], ROOT, path.join(dir, "hub.log"));
  await waitFor("the hub", async () => (await fetch(`${BASE}/api/setup`)).ok);
  await checkWebUi();
  await runSetupWizard(fs.readFileSync(path.join(hubDir, "setup.code"), "utf8").trim());
  const cookie = await login(OWNER);
  await createUser(cookie, VIEWER, "viewer");

  // The company and site the probe will belong to, and "Add probe".
  const api = async (url: string, body: unknown) => {
    const res = await fetch(`${BASE}${url}`, {
      method: "POST",
      headers: { "content-type": "application/json", cookie, ...CSRF },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`POST ${url}: ${res.status} ${await res.text()}`);
    return res.json();
  };
  await api("/api/companies", { id: COMPANY, name: COMPANY });
  await api(`/api/companies/${COMPANY}/sites`, { id: "lab", name: "lab" });
  const { code } = (await api("/api/enrollment-codes", {
    company_id: COMPANY,
    site_id: "lab",
    probe_name: PROBE_NAME,
    hub_url: BASE,
  })) as { code: string };

  // Probe: own folder (it keeps its identity, enrollment and checks next to its binary).
  const probeExe = path.join(probeDir, path.basename(PROBE));
  fs.copyFileSync(PROBE, probeExe);
  const enrolled = execFileSync(probeExe, ["enroll", code], { cwd: probeDir, encoding: "utf8", timeout: 60_000 });
  if (!enrolled.includes("Enrolled with")) throw new Error(`Enrollment failed:\n${enrolled}`);

  // Its checks and interval, on the hub.
  const [record] = (await (await fetch(`${BASE}/api/probes`, { headers: { cookie } })).json()) as { id: string }[];
  await api(`/api/probes/${record.id}`, { name: PROBE_NAME, site_id: "lab", interval_secs: 5 });
  await api(`/api/probes/${record.id}/checks`, {
    name: "Hub port",
    spec: { kind: "tcp", host: "127.0.0.1", port: PORT },
  });
  await api(`/api/probes/${record.id}/checks`, {
    name: "Closed port",
    spec: { kind: "tcp", host: "127.0.0.1", port: 1 },
  });
  const probe = start("probe", probeExe, [], probeDir, path.join(dir, "probe.log"));

  // Ready once the hub has results of both checks: the first report gets the
  // checks in its answer, and the probe runs them at once.
  await waitFor("the first results", async () => {
    const res = await fetch(`${BASE}/api/probe/${record.id}/devices`, { headers: { cookie } });
    return res.ok && ((await res.json()) as unknown[]).length === 2;
  });

  // A customer's login, limited to the company.
  await createUser(cookie, CLIENT, "client", { mode: "only", companies: [COMPANY] });

  // Visible to the tests (workers inherit the environment).
  process.env.E2E_BASE_URL = BASE;
  process.env.E2E_PROBE_ID = record.id;
  process.env.E2E_PROBE_LOG = path.join(probeDir, "blentinel_probe.log");

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
