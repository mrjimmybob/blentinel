// The probe page (M2-05): checks, their results, sync state and interval;
// "Run checks now" through the command channel (M2-06).
// Changes here use a *disabled* check, which isn't sent to the probe, so
// the probe's config (and what the other tests see) never changes.
import type { Page } from "@playwright/test";
import { COMPANY, PROBE_NAME, expect, test } from "./fixtures";

const probePage = () => `/probes/${process.env.E2E_PROBE_ID}`;
const dialogSave = (page: Page) => page.locator(".check-dialog").getByRole("button", { name: "Save" }).click();

test("a probe's page shows its checks, their results and that it runs them", async ({ loggedIn: page }) => {
  await page.goto(`/companies/${COMPANY}`);
  await page.locator(".probes-table").getByRole("link", { name: PROBE_NAME }).click();
  await expect(page).toHaveURL(new RegExp(`${probePage()}$`));
  await page.locator("html[data-hydrated]").waitFor({ state: "attached" });

  await expect(page.locator("h1")).toHaveText(PROBE_NAME);
  await expect(page.locator(".sync-badge")).toHaveText("In sync");
  await expect(page.locator("#interval_secs")).toHaveValue("5");
  const rows = page.locator(".checks-table tbody tr");
  await expect(rows.filter({ hasText: "Hub port" })).toContainText("UP");
  await expect(rows.filter({ hasText: "Closed port" })).toContainText("DOWN");
  await expect(rows.filter({ hasText: "Closed port" })).toContainText("TCP port");

  // M5-01: the real probe read the hub's own certificate (self-signed, 10
  // years: the check doesn't verify who issued it) and counted its days.
  const certificate = rows.filter({ hasText: "Hub certificate" });
  await expect(certificate).toContainText("TLS certificate");
  await expect(certificate).toContainText("UP");
  await expect(certificate).toContainText(/Certificate expires in \d+ days/);
});

// M5-06, D-83, on a disabled check (the probe's config stays).
test("a service check names a service; the real probe asked about one", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  // The machine's service manager answered the real probe.
  const system = page.locator(".checks-table tbody tr", { hasText: "System service" });
  await expect(system).toContainText("UP");
  await expect(system).toContainText(/is (running|active)/);

  const name = `E2E service ${Date.now()}`;
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_kind").selectOption("service");
  // Running or not: there is nothing to set limits on.
  await expect(page.locator('input[name="check_warning_at"]')).toHaveCount(0);
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("service name can't be empty");
  await page.locator("#check_service").fill("Spooler");
  await page.locator('input[name="check_enabled"]').uncheck();
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await expect(row).toContainText("Spooler");
  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator("#check_service")).toHaveValue("Spooler");
  await page.getByRole("button", { name: "Cancel" }).click();
  await row.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: name })).toHaveCount(0);
});

// M5-07, D-82, on a disabled check (the probe's config stays).
test("a DNS check: a name, a record type, the server to ask and the answers that are fine", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  const name = `E2E dns ${Date.now()}`;
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_kind").selectOption("dns");
  await page.locator("#check_dns_name").fill("example.com");
  await page.locator("#check_server").fill("dns.example.com");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("DNS server is an IP address");
  await page.locator("#check_server").fill("192.168.1.1");
  await page.locator("#check_record").selectOption("mx");
  await page.locator("#check_expect").fill("10 mail.example.com, 20 backup.example.com");
  // How long an answer may take is its latency limit, as for a ping.
  await expect(page.locator('input[name="check_warning_at"]')).toBeVisible();
  await page.locator('input[name="check_enabled"]').uncheck();
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await expect(row).toContainText("DNS name");
  await expect(row).toContainText("example.com (MX at 192.168.1.1)");
  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator("#check_record")).toHaveValue("mx");
  await expect(page.locator("#check_expect")).toHaveValue("10 mail.example.com, 20 backup.example.com");
  await page.getByRole("button", { name: "Cancel" }).click();
  await row.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: name })).toHaveCount(0);
});

// M5-05, D-81, on a disabled check (the probe's config stays).
test("a check of every disk takes the disks to leave out", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  const name = `E2E disks ${Date.now()}`;
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_kind").selectOption("disks");
  // Its limits are the disk's: percent used.
  await expect(page.locator('input[name="check_warning_at"]')).toBeVisible();
  await page.locator("#check_ignore").fill("E:, /mnt/*");
  await page.locator('input[name="check_enabled"]').uncheck();
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await expect(row).toContainText("all disks but E:, /mnt/*");
  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator("#check_ignore")).toHaveValue("E:, /mnt/*");
  await page.getByRole("button", { name: "Cancel" }).click();
  await row.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: name })).toHaveCount(0);
});

// M5-04, D-80, on a disabled check (the probe's config stays).
test("a ping says how many echoes and which TCP port to try without an answer", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  // The real probe reached this machine: by echo, or by the hub's port.
  const hubPing = page.locator(".checks-table tbody tr", { hasText: "Hub ping" });
  await expect(hubPing).toContainText("UP");
  await expect(hubPing).toContainText(/Answered in|TCP port \d+ answers/);

  const name = `E2E ping ${Date.now()}`;
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_host").fill("10.0.0.1");
  await page.locator("#check_count").fill("50");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("1-20");
  await page.locator("#check_count").fill("10");
  await page.locator("#check_tcp_port").fill("443");
  await page.locator('input[name="check_enabled"]').uncheck();
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator("#check_count")).toHaveValue("10");
  await expect(page.locator("#check_tcp_port")).toHaveValue("443");
  await page.getByRole("button", { name: "Cancel" }).click();
  await row.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: name })).toHaveCount(0);
});

// M5-03, D-79, on a disabled check (the probe's config stays).
test("a file check: a full path, a name pattern, a smallest size, and limits in hours", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  // The real probe found its own program in its folder, minutes old.
  const program = page.locator(".checks-table tbody tr", { hasText: "Probe program" });
  await expect(program).toContainText("UP");
  await expect(program).toContainText(/minutes old, \d/);

  const name = `E2E backup ${Date.now()}`;
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_kind").selectOption("file");
  await page.locator("#check_file_path").fill("backups");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("full one");
  await page.locator("#check_file_path").fill("/var/backups");
  await page.locator("#check_pattern").fill("*.bak");
  await page.locator("#check_min_mb").fill("lots");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("number of MB");
  await page.locator("#check_min_mb").fill("500");
  await page.locator('input[name="check_recursive"]').check();
  // Its limits are hours: a weekly backup.
  await page.locator('input[name="check_warning_at"]').fill("170");
  await page.locator('input[name="check_enabled"]').uncheck();
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await expect(row).toContainText("/var/backups (*.bak)");
  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator("#check_pattern")).toHaveValue("*.bak");
  await expect(page.locator("#check_min_mb")).toHaveValue("500");
  await expect(page.locator('input[name="check_recursive"]')).toBeChecked();
  await expect(page.locator('input[name="check_warning_at"]')).toHaveValue("170");
  await expect(page.locator('input[name="check_down_at"]')).toHaveAttribute("placeholder", "50 h");
  await page.getByRole("button", { name: "Cancel" }).click();

  await row.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: name })).toHaveCount(0);
});

// M5-02, D-78, on a disabled check (the probe's config stays).
test("a web page check says what it expects: status codes, text, headers, redirects", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  // The real probe asked the hub's plain port and got the redirect it expects.
  const redirect = page.locator(".checks-table tbody tr", { hasText: "Hub redirect" });
  await expect(redirect).toContainText("UP");
  await expect(redirect).toContainText(/HTTP 30\d/);

  const name = `E2E page ${Date.now()}`;
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_kind").selectOption("http");
  await page.locator("#check_url").fill("https://example.com/login");

  // Checked like the hub checks them, before anything is sent.
  await page.locator("#check_status").fill("2xx");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("status code");
  await page.locator("#check_status").fill("200, 401");
  await page.locator("#check_contains").fill("Sign in");
  await page.locator("#check_method").selectOption("head");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("HEAD");
  await page.locator("#check_method").selectOption("get");
  await page.locator("#check_headers").fill("Accept-Language: en");
  await page.locator('input[name="check_no_redirects"]').check();
  await page.locator('input[name="check_unverified"]').check();
  await page.locator('input[name="check_enabled"]').uncheck();
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  // The dialog shows what was saved.
  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator("#check_status")).toHaveValue("200, 401");
  await expect(page.locator("#check_contains")).toHaveValue("Sign in");
  await expect(page.locator("#check_method")).toHaveValue("get");
  await expect(page.locator("#check_headers")).toHaveValue("Accept-Language: en");
  await expect(page.locator('input[name="check_no_redirects"]')).toBeChecked();
  await expect(page.locator('input[name="check_unverified"]')).toBeChecked();
  await page.getByRole("button", { name: "Cancel" }).click();

  await row.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: name })).toHaveCount(0);
});

// M5-01, D-77, on a disabled check (the probe's config stays).
test("a TLS certificate check: port 443 unless typed, days-left limits the other way round", async ({
  loggedIn: page,
}) => {
  await page.goto(probePage());
  const name = `E2E certificate ${Date.now()}`;
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_kind").selectOption("tls");
  await page.locator("#check_host").fill("example.com");
  await expect(page.locator('input[name="check_unverified"]')).not.toBeChecked();

  // Days left fall: the warning limit is the higher one.
  await page.locator('input[name="check_warning_at"]').fill("7");
  await page.locator('input[name="check_down_at"]').fill("30");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("above the down limit");
  await page.locator('input[name="check_warning_at"]').fill("30");
  await page.locator('input[name="check_down_at"]').fill("7");
  await page.locator('input[name="check_unverified"]').check();
  await page.locator('input[name="check_enabled"]').uncheck();
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await expect(row).toContainText("TLS certificate");
  await expect(row).toContainText("example.com:443");

  // The dialog shows what was saved.
  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator("#check_port")).toHaveValue("443");
  await expect(page.locator('input[name="check_unverified"]')).toBeChecked();
  await expect(page.locator('input[name="check_warning_at"]')).toHaveValue("30");
  await page.getByRole("button", { name: "Cancel" }).click();

  await row.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: name })).toHaveCount(0);
});

test("a check is added, edited and deleted from the probe's page", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  const name = `E2E uptime ${Date.now()}`;

  // The form checks values like the hub does, before sending.
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_host").fill("router lan");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("a name or an IP address");

  // A ping has latency limits (D-64); uptime has none.
  await expect(page.locator('input[name="check_warning_at"]')).toBeVisible();
  await page.locator('input[name="check_warning_at"]').fill("slow");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("a number");

  // Another kind shows its own fields; this one has none.
  await page.locator("#check_kind").selectOption("uptime");
  await expect(page.locator("#check_host")).toHaveCount(0);
  await expect(page.locator('input[name="check_warning_at"]')).toHaveCount(0);
  await page.locator('input[name="check_enabled"]').uncheck();
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await expect(row).toContainText("Disabled");
  await expect(row).toContainText("Uptime");
  await expect(page.locator(".sync-badge")).toHaveText("In sync"); // disabled: not sent

  // When it alerts (D-63): empty is the default, 2 in a row; 0 is refused.
  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator('input[name="check_fail_after"]')).toHaveValue("");
  await page.locator("#check_name").fill(`${name} (renamed)`);
  await page.locator('input[name="check_fail_after"]').fill("0");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("1 to 10 failed results");
  await page.locator('input[name="check_fail_after"]').fill("3");
  await dialogSave(page);
  const renamed = page.locator(".checks-table tbody tr", { hasText: `${name} (renamed)` });
  await expect(renamed).toHaveCount(1);
  await renamed.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator('input[name="check_fail_after"]')).toHaveValue("3");
  await expect(page.locator('input[name="check_recover_after"]')).toHaveValue("");
  await page.getByRole("button", { name: "Cancel" }).click();

  await renamed.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: name })).toHaveCount(0);
});

// Check dependencies (D-68), on a disabled check (the probe's config stays).
test("a check is put behind another and shows it", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  const name = `E2E behind ${Date.now()}`;
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_kind").selectOption("uptime");
  await page.locator("#check_depends_on").selectOption({ label: "Hub port" });
  await page.locator('input[name="check_enabled"]').uncheck();
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await expect(row.locator(".check-behind")).toHaveText(" · behind Hub port");

  // The dialog shows it; "Nothing" takes it away.
  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator("#check_depends_on option:checked")).toHaveText("Hub port");
  await expect(page.locator("#check_depends_on option", { hasText: name })).toHaveCount(0);
  await page.locator("#check_depends_on").selectOption({ label: "Nothing" });
  await dialogSave(page);
  await expect(row.locator(".check-behind")).toHaveCount(0);

  await row.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: name })).toHaveCount(0);
});

test("an interval outside the allowed range is refused with the reason", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  await page.locator("#interval_secs").fill("1");
  await page.locator(".interval-editor").getByRole("button", { name: "Save" }).click();
  await expect(page.locator(".login-error").first()).toContainText("5-3600");
  await page.reload();
  await expect(page.locator("#interval_secs")).toHaveValue("5");
});

test("Run checks now reaches the probe and comes back done", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  await page.locator("html[data-hydrated]").waitFor({ state: "attached" });
  await page.getByRole("button", { name: "Run checks now" }).click();

  // Queued, then sent with the answer to the probe's next report (every 5 s
  // here), then done with the report after that. The page reloads every 15 s.
  const row = page.locator(".commands-table tbody tr").first();
  await expect(row).toContainText("Run checks now");
  await expect(row).toContainText("by ");
  await expect(row.locator(".command-badge")).toHaveText("Done", { timeout: 45_000 });
  await expect(row).toContainText(/ran \d+ checks?/);
});

test("a command reaches the probe between its reports, in a second or two", async ({ loggedIn: page }) => {
  // D-76: the probe asks for commands on its own loop and the hub holds that
  // request until one is queued. Queued just after a report, a command that
  // waited for the next report would be sent 5 s later (the interval here)
  // and done after that; through the held poll it is done well before. The
  // probe gets it at once, then runs its checks, which takes it over 2 s
  // here (Windows takes 2 s to refuse the closed port's connection): so the
  // bound is 4.5 s, under the 5 s that waiting for a report would cost.
  const id = process.env.E2E_PROBE_ID;
  const lastSeen = async () =>
    ((await (await page.request.get("/api/probes")).json()) as { id: string; last_seen_at: string | null }[]).find(
      (p) => p.id === id,
    )?.last_seen_at;
  const before = await lastSeen();
  await expect.poll(lastSeen, { timeout: 15_000, intervals: [100] }).not.toBe(before);

  const started = Date.now();
  const queued = await page.request.post(`/api/probes/${id}/commands`, {
    headers: { "x-blentinel": "1" },
    data: { kind: "run_checks" },
  });
  expect(queued.ok()).toBeTruthy();
  const command = (await queued.json()) as { id: string };
  await expect
    .poll(
      async () =>
        ((await (await page.request.get(`/api/probes/${id}/commands`)).json()) as { id: string; state: string }[]).find(
          (c) => c.id === command.id,
        )?.state,
      { timeout: 4_500, intervals: [100] },
    )
    .toBe("succeeded");
  expect(Date.now() - started).toBeLessThan(4_500);
});

test("a Viewer sees the checks but can't change them", async ({ viewer: page }) => {
  await page.goto(probePage());
  await expect(page.locator(".checks-table tbody tr", { hasText: "Hub port" })).toHaveCount(1);
  await expect(page.getByRole("button", { name: "Add check" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Edit" })).toHaveCount(0);
  await expect(page.locator("#interval_secs")).toBeDisabled();
  await expect(page.getByRole("button", { name: "Run checks now" })).toHaveCount(0);
});

// M5-08, D-86, on a disabled check (the probe's config stays).
test("an SNMP check: a community nobody reads back, v3 users, values by OID", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  const name = `E2E snmp ${Date.now()}`;
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_kind").selectOption("snmp");
  await page.locator("#check_host").fill("192.168.1.5");
  // How fast the agent answers is its latency limit.
  await expect(page.locator('input[name="check_warning_at"]')).toBeVisible();
  await page.locator("#check_oids").fill("sysUpTime.0");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("isn't an OID in numbers");
  await page.locator("#check_oids").fill("Uptime = 1.3.6.1.2.1.1.3.0\nUplink = 1.3.6.1.2.1.2.2.1.8.1 = 1");
  // Enabled, it needs its community: the hub says so.
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("the community is needed");
  await expect(page.locator("#check_community")).toHaveAttribute("type", "password");
  await page.locator("#check_community").fill("e2e-community-s3cret");
  await page.locator('input[name="check_enabled"]').uncheck();
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await expect(row).toContainText("SNMP");
  await expect(row).toContainText("192.168.1.5");
  // Nothing the browser was sent holds the community again.
  const listed = await (await page.request.get(`/api/probes/${process.env.E2E_PROBE_ID}/checks`)).text();
  expect(listed).not.toContain("e2e-community-s3cret");
  expect(listed).toContain('"secrets_set":["community"]');

  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator("#check_oids")).toHaveValue("Uptime = 1.3.6.1.2.1.1.3.0\nUplink = 1.3.6.1.2.1.2.2.1.8.1 = 1");
  await expect(page.locator("#check_community")).toHaveValue("");
  await expect(page.locator("#check_community")).toHaveAttribute("placeholder", "saved: leave empty to keep it");
  // v3 has a user and passwords instead, shown as they are needed.
  await page.locator("#check_snmp_version").selectOption("v3");
  await expect(page.locator("#check_community")).toHaveCount(0);
  await expect(page.locator("#check_auth_password")).toHaveCount(0);
  await page.locator("#check_snmp_user").fill("monitor");
  await page.locator("#check_snmp_auth").selectOption("sha256");
  await page.locator("#check_auth_password").fill("short");
  await page.locator("#check_snmp_privacy").selectOption("aes128");
  await page.locator("#check_priv_password").fill("long enough to pass");
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);
  // Disabled, it may wait for a password that is long enough; its secrets changed with its version.
  const again = await (await page.request.get(`/api/probes/${process.env.E2E_PROBE_ID}/checks`)).json();
  const saved = again.checks.find((c: { name: string }) => c.name === name);
  expect(saved.spec.version).toBe("v3");
  expect(saved.secrets_set).toEqual(["auth_password", "priv_password"]);

  await row.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: name })).toHaveCount(0);
});

// M5-09, D-87, on a disabled check (the probe's config stays).
test("an SNMP check: a table, a share of its capacity, and limits on a value", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  const name = `E2E snmp table ${Date.now()}`;
  const supply =
    "Supply = 1.3.6.1.2.1.43.11.1.1.9 ; rows 1.3.6.1.2.1.43.11.1.1.6 ; of 1.3.6.1.2.1.43.11.1.1.8 ; warn < 20 ; down < 5";
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_kind").selectOption("snmp");
  await page.locator("#check_host").fill("192.168.1.9");
  await page.locator('input[name="check_enabled"]').uncheck();
  // An option nobody knows, then limits that contradict each other.
  await page.locator("#check_oids").fill("Fan = 1.3.6.1.4.1.9.1 ; sometimes");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText('Fan: "sometimes": not an option');
  await page.locator("#check_oids").fill("Fan = 1.3.6.1.4.1.9.1 ; warn > 60 ; down < 5");
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("both over or both under");
  // As typed loosely; the form shows it tidied.
  await page
    .locator("#check_oids")
    .fill(`${supply}\nTemperature=1.3.6.1.4.1.6574.1.2.0;unit °C;x 0.1;WARN >55\nPort = 1.3.6.1.2.1.2.2.1.8 ; walk ; means 1=up,2=down ; not down`);
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  const listed = await (await page.request.get(`/api/probes/${process.env.E2E_PROBE_ID}/checks`)).json();
  const saved = listed.checks.find((c: { name: string }) => c.name === name);
  expect(saved.spec.oids[0]).toEqual({
    oid: "1.3.6.1.2.1.43.11.1.1.9",
    name: "Supply",
    walk: true,
    rows: "1.3.6.1.2.1.43.11.1.1.6",
    of: "1.3.6.1.2.1.43.11.1.1.8",
    warn: { under: 20 },
    down: { under: 5 },
  });
  expect(saved.spec.oids[2].names).toEqual({ "1": "up", "2": "down" });

  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator("#check_oids")).toHaveValue(
    `${supply}\nTemperature = 1.3.6.1.4.1.6574.1.2.0 ; / 10 ; unit °C ; warn > 55\nPort = 1.3.6.1.2.1.2.2.1.8 ; walk ; means 1=up, 2=down ; not down`,
  );
  await page.locator(".check-dialog").getByRole("button", { name: "Cancel" }).click();
  await row.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: name })).toHaveCount(0);
});

// M5-10, D-88, on a disabled check (the probe's config stays).
test("a database check: an engine, a host, and its usual port unless another is typed", async ({ loggedIn: page }) => {
  await page.goto(probePage());
  const name = `E2E database ${Date.now()}`;
  await page.getByRole("button", { name: "Add check" }).click();
  await page.locator("#check_name").fill(name);
  await page.locator("#check_kind").selectOption("database");
  // Every engine the probe can greet, with its port.
  await expect(page.locator("#check_engine option")).toHaveText([
    "PostgreSQL (port 5432)",
    "MySQL or MariaDB (port 3306)",
    "Microsoft SQL Server (port 1433)",
    "MongoDB (port 27017)",
  ]);
  // How fast it answers is its latency limit; it has no secrets.
  await expect(page.locator('input[name="check_warning_at"]')).toBeVisible();
  await expect(page.locator("#check_community")).toHaveCount(0);
  await page.locator('input[name="check_enabled"]').uncheck();
  await dialogSave(page);
  await expect(page.locator(".check-dialog .login-error")).toContainText("the host can't be empty");
  await page.locator("#check_host").fill("db.example.lan");
  await page.locator("#check_engine").selectOption("mssql");
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);

  const row = page.locator(".checks-table tbody tr", { hasText: name });
  await expect(row).toContainText("Database server");
  await expect(row).toContainText("db.example.lan:1433 (Microsoft SQL Server)");

  await row.getByRole("button", { name: "Edit" }).click();
  await expect(page.locator("#check_engine")).toHaveValue("mssql");
  await expect(page.locator("#check_port")).toHaveValue("");
  await page.locator("#check_engine").selectOption("mongodb");
  await page.locator("#check_port").fill("27018");
  await dialogSave(page);
  await expect(page.locator(".check-dialog")).toHaveCount(0);
  await expect(row).toContainText("db.example.lan:27018 (MongoDB)");
  const listed = await (await page.request.get(`/api/probes/${process.env.E2E_PROBE_ID}/checks`)).json();
  const saved = listed.checks.find((c: { name: string }) => c.name === name);
  expect(saved.spec).toEqual({ kind: "database", engine: "mongodb", host: "db.example.lan", port: 27018 });

  await row.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".checks-table tbody tr", { hasText: name })).toHaveCount(0);
});
