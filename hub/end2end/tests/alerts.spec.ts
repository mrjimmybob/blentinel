import { expect, test } from "./fixtures";

// Alert channels and routing (M3-03, D-66). Nothing leaves the machine: the
// webhook points at a closed local port, so "Send test" fails, as it should.
test("an admin sets up a channel, tests it and routes alerts to it", async ({ loggedIn: page }) => {
  await page.goto("/alerts");
  await expect(page.getByRole("heading", { name: "Alerts", exact: true })).toBeVisible();

  // The hub's address, for links in alerts.
  await page.locator('input[name="hub_url"]').fill("https://hub.example.com");
  await page.locator('button[name="settings_save"]').click();
  await expect(page.locator(".form-message")).toHaveText("Settings saved.");

  // A signed webhook; its key is write-only.
  const channels = page.locator(".channels-table");
  await page.locator('input[name="channel_name"]').fill("E2E hook");
  await page.locator('select[name="channel_kind"]').selectOption("webhook");
  await page.locator('input[name="channel_target"]').fill("http://127.0.0.1:9/hook");
  await page.locator('input[name="channel_secret"]').fill("e2e-signing-key");
  await page.locator('button[name="channel_save"]').click();
  await expect(channels).toContainText("E2E hook");
  await expect(channels).toContainText("(secret set)");
  await expect(channels).toContainText("never");
  await expect(page.locator("body")).not.toContainText("e2e-signing-key");

  // "Send test" says why it failed, and the channel keeps it.
  await page.locator('button[name="channel_test"]').click();
  await expect(page.locator(".login-error")).toContainText("Test failed");
  await expect(channels).toContainText("failed");

  // A change that leaves the key empty keeps it.
  await page.locator('button[name="channel_edit"]').click();
  await expect(page.locator('input[name="channel_secret"]')).toHaveAttribute("placeholder", /leave empty to keep it/);
  await page.locator('input[name="channel_name"]').fill("E2E webhook");
  await page.locator('button[name="channel_save"]').click();
  await expect(channels).toContainText("E2E webhook");
  await expect(channels).toContainText("(secret set)");

  // A rule: outages go to the webhook.
  await page.locator('button[name="rule_add"]').click();
  const rule = page.locator(".rule").first();
  await rule.locator('input[name="rule_name"]').fill("Outages");
  await rule.locator('input[name="rule_name"]').blur();
  await rule.locator(".rule-choice", { hasText: "Down" }).locator("input").check();
  await rule.locator(".rule-channels .rule-choice", { hasText: "E2E webhook" }).locator("input").check();

  // Reminders and escalation (D-69), checked like the hub checks them.
  await rule.locator('input[name="rule_remind"]').fill("10");
  await rule.locator('input[name="rule_remind"]').blur();
  await page.locator('button[name="rules_save"]').click();
  await expect(page.locator(".login-error").first()).toContainText("remind every 15-10080 minutes");
  await rule.locator('input[name="rule_remind"]').fill("30");
  await rule.locator('input[name="rule_remind"]').blur();
  await rule.locator('input[name="rule_escalate"]').fill("60");
  await rule.locator('input[name="rule_escalate"]').blur();
  await rule.locator(".rule-escalate-to .rule-choice", { hasText: "E2E webhook" }).locator("input").check();
  await page.locator('button[name="rules_save"]').click();
  await expect(page.locator(".form-message")).toHaveText("Rules saved.");

  await page.reload();
  const saved = page.locator(".rule").first();
  await expect(saved.locator('input[name="rule_name"]')).toHaveValue("Outages");
  await expect(saved.locator(".rule-channels .rule-choice", { hasText: "E2E webhook" }).locator("input")).toBeChecked();
  await expect(saved.locator('input[name="rule_remind"]')).toHaveValue("30");
  await expect(saved.locator('input[name="rule_escalate"]')).toHaveValue("60");
  await expect(saved.locator(".rule-escalate-to .rule-choice", { hasText: "E2E webhook" }).locator("input")).toBeChecked();

  // Removing the channel removes the rule that only sent to it.
  await page.locator('button[name="channel_delete"]').click();
  await expect(page.locator(".alerts-channels .empty-state")).toBeVisible();
  await expect(page.locator(".rule")).toHaveCount(0);

  // ntfy (D-70): any server; a random topic on the one typed; a warning for a short one.
  await page.locator('input[name="channel_name"]').fill("E2E phones");
  await page.locator('select[name="channel_kind"]').selectOption("ntfy");
  const topic = page.locator('input[name="channel_target"]');
  await topic.fill("https://ntfy.example.com/alerts");
  await expect(page.locator(".ntfy-topic .form-hint")).toBeVisible();
  await page.locator('button[name="ntfy_random_topic"]').click();
  await expect(topic).toHaveValue(/^https:\/\/ntfy\.example\.com\/blentinel-[0-9a-f]{16}$/);
  await expect(page.locator(".ntfy-topic .form-hint")).toHaveCount(0);
  await page.locator('button[name="channel_save"]').click(); // no token: optional
  await expect(channels).toContainText("E2E phones");
  await expect(channels).toContainText("ntfy");
  await page.locator('button[name="channel_delete"]').click();
  await expect(page.locator(".alerts-channels .empty-state")).toBeVisible();

  // The daily digest (D-71): a real time zone; without e-mail channels it is off.
  await page.locator('input[name="digest_timezone"]').fill("Mars/Olympus");
  await page.locator('button[name="digest_save"]').click();
  await expect(page.locator(".login-error").first()).toContainText("not a time zone");
  await page.locator('input[name="digest_timezone"]').fill("Europe/Madrid");
  await page.locator('input[name="digest_hour"]').fill("8");
  await page.locator('button[name="digest_save"]').click();
  await expect(page.locator(".form-message")).toHaveText("Digest saved.");
  await page.reload();
  await expect(page.locator('input[name="digest_timezone"]')).toHaveValue("Europe/Madrid");
  await expect(page.locator('input[name="digest_hour"]')).toHaveValue("8");
  await page.locator('button[name="digest_send"]').click();
  await expect(page.locator(".login-error").first()).toContainText("choose the e-mail channels");
});

// The page reloads its data after every action. That reload must keep what is
// typed and not saved yet, in every form: it rebuilt them and emptied them
// (CI #69). The reload after "Save settings" is held until something is typed
// in each form, so the order is forced, not hoped for; its answer carries a
// marker, which shows when the page has applied it.
test("a reload after an action keeps unsaved input in every form", async ({ loggedIn: page }) => {
  const marker = "2099-12-31 (reloaded)";
  let loads = 0;
  let release!: () => void;
  const held = new Promise<void>((resolve) => (release = resolve));
  await page.route("**/api/admin/alerts", async (route) => {
    if (route.request().method() !== "GET") return route.continue();
    loads += 1;
    if (loads < 2) return route.continue();
    await held;
    const response = await route.fetch();
    const json = await response.json();
    json.digest_last_sent = marker;
    await route.fulfill({ response, json });
  });

  await page.goto("/alerts");
  await expect(page.locator('input[name="hub_url"]')).toBeVisible();
  await page.locator('input[name="hub_url"]').fill("https://hub.example.com");
  await page.locator('button[name="settings_save"]').click();
  await expect(page.locator(".form-message")).toHaveText("Settings saved.");
  await expect.poll(() => loads).toBe(2); // asked for, and held

  // Unsaved input in each form, while the reload waits.
  await page.locator('input[name="channel_name"]').fill("Typed, not saved");
  await page.locator('button[name="rule_add"]').click();
  const newRule = page.locator(".rule").last();
  await newRule.locator('input[name="rule_name"]').fill("Unsaved rule");
  await newRule.locator('input[name="rule_name"]').blur();
  await page.locator('input[name="digest_timezone"]').fill("Asia/Tokyo");
  await page.locator('input[name="smtp_server"]').fill("mail.unsaved.example");

  release();
  await expect(page.locator(".alerts-digest")).toContainText(`Last sent for ${marker}.`);

  await expect(page.locator('input[name="channel_name"]')).toHaveValue("Typed, not saved");
  await expect
    .poll(() => page.locator('input[name="rule_name"]').evaluateAll((els) => els.map((e) => (e as HTMLInputElement).value)))
    .toContain("Unsaved rule");
  await expect(page.locator('input[name="digest_timezone"]')).toHaveValue("Asia/Tokyo");
  await expect(page.locator('input[name="smtp_server"]')).toHaveValue("mail.unsaved.example");
});

test("viewers don't see the alerts page", async ({ viewer: page }) => {
  await page.goto("/");
  await expect(page.getByRole("link", { name: "Dashboard" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Alerts" })).toHaveCount(0);
  expect((await page.request.get("/api/admin/alerts")).status()).toBe(403);
});
