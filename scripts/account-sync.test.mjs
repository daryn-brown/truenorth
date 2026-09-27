import assert from "node:assert/strict";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { createServer } from "vite";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const server = await createServer({
  root,
  cacheDir: join(root, "node_modules/.cache/account-sync-preview"),
  server: { host: "127.0.0.1", port: 0, strictPort: false, open: false },
});
let browser;

try {
  await server.listen();
  const base = server.resolvedUrls?.local[0];
  if (!base) throw new Error("The isolated preview did not provide a local URL.");
  const url = new URL("scripts/account-sync-preview/", base).href;
  assert.equal((await fetch(url)).ok, true);
  browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1100, height: 800 }, reducedMotion: "reduce" });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("**/*", (route) => new URL(route.request().url()).origin === new URL(base).origin
    ? route.continue() : route.abort());
  await page.addInitScript(() => {
    window.confirm = () => { throw new Error("Browser confirm must not be used for account actions."); };
  });
  const load = async () => {
    await page.goto(url, { waitUntil: "networkidle" });
    await page.getByRole("button", { name: "Delete Roth duplicate", exact: true }).waitFor();
  };
  const calls = (command) => page.evaluate((name) =>
    window.accountSyncFixture.calls.filter((call) => call.command === name), command);
  const fail = (command) => page.evaluate((name) => { window.accountSyncFixture.failCommand = name; }, command);
  const openReview = async () => {
    await page.getByRole("button", { name: "Choose accounts to sync", exact: true }).click();
    await page.getByRole("group", { name: "Individual (****0580)", exact: true }).waitFor();
  };
  const firstChoice = () => page.getByLabel("Sync choice for Individual (****0580)", { exact: true });
  const secondChoice = () => page.getByLabel("Sync choice for Individual (****7986)", { exact: true });
  const duplicateChoice = () => page.getByLabel("Sync choice for Roth IRA (****6755)", { exact: true });
  const questradeChoice = () => page.getByLabel("Sync choice for TFSA (****1111)", { exact: true });
  const deleteDialog = () => page.getByRole("dialog", { name: "Delete account?", exact: true });
  const selectionDialog = () => page.getByRole("dialog", { name: "Confirm account selection", exact: true });

  await load();
  await page.getByRole("button", { name: "Delete Roth duplicate", exact: true }).click();
  assert.match(await deleteDialog().innerText(), /stored snapshots and transactions are kept/i);
  assert.match(await deleteDialog().innerText(), /SnapTrade/);
  await deleteDialog().getByRole("button", { name: "Cancel", exact: true }).click();
  assert.equal((await calls("delete_account")).length, 0);
  await page.getByRole("button", { name: "Delete Roth duplicate", exact: true }).click();
  await fail("delete_account");
  await deleteDialog().getByRole("button", { name: "Delete account", exact: true }).click();
  await deleteDialog().getByRole("alert").filter({ hasText: "Could not delete account" }).waitFor();
  assert.equal(await page.getByRole("button", { name: "Delete Roth duplicate", exact: true }).count(), 1);
  await page.evaluate(() => { window.accountSyncFixture.holdDelete = true; });
  await deleteDialog().getByRole("button", { name: "Delete account", exact: true }).click();
  await page.waitForFunction(() => window.accountSyncFixture.releaseDelete !== null);
  assert.equal(await deleteDialog().getByRole("button", { name: "Saving...", exact: true }).isDisabled(), true);
  await page.keyboard.press("Escape");
  assert.equal(await deleteDialog().isVisible(), true);
  await page.evaluate(() => window.accountSyncFixture.releaseDelete());
  await deleteDialog().waitFor({ state: "detached" });
  assert.equal(await page.getByRole("button", { name: "Delete Roth duplicate", exact: true }).count(), 0);
  assert.equal((await calls("delete_account")).length, 2);
  console.log("PASS delete cancel, confirmation, non-reentrant request, visible error, and immediate removal");

  await load();
  await page.getByRole("button", { name: "Delete Roth duplicate", exact: true }).click();
  await fail("get_net_worth");
  await deleteDialog().getByRole("button", { name: "Delete account", exact: true }).click();
  await deleteDialog().waitFor({ state: "detached" });
  await page.getByRole("alert").filter({ hasText: "was hidden successfully" }).waitFor();
  assert.equal((await calls("delete_account")).length, 1);
  await page.getByRole("button", { name: "Retry refresh", exact: true }).click();
  await page.getByRole("alert").filter({ hasText: "was hidden successfully" }).waitFor({ state: "detached" });
  assert.equal((await calls("delete_account")).length, 1);
  console.log("PASS refresh failure is distinct from deletion failure; retry does not repeat deletion");

  await load();
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await openReview();
  assert.equal(await firstChoice().inputValue(), "unreviewed");
  assert.equal(await secondChoice().inputValue(), "unreviewed");
  assert.equal(await duplicateChoice().inputValue(), "sync");
  assert.equal(await questradeChoice().inputValue(), "unreviewed");
  assert.equal(await duplicateChoice().locator('option[value="link"]').count(), 0);
  assert.match(await page.getByRole("region", { name: "Choose accounts to sync" }).innerText(), /cannot be reassigned or merged/);
  assert.equal((await calls("snaptrade_save_account_choices")).length, 0);
  await questradeChoice().selectOption("create");
  await duplicateChoice().selectOption("ignore");
  await page.getByRole("button", { name: "Save choices", exact: true }).click();
  await page.getByText("Account choices saved. Sync now updates only selected accounts.", { exact: true }).waitFor();
  const saved = await calls("snaptrade_save_account_choices");
  assert.deepEqual(saved[0].payload.payload.choices, [
    { remote_id: "snap-3", action: "ignore" }, { remote_id: "snap-qt", action: "create" },
  ]);
  await page.getByRole("button", { name: "Sync now", exact: true }).click();
  await page.getByText(/2 new accounts need review/).waitFor();
  await openReview();
  assert.equal(await firstChoice().inputValue(), "unreviewed");
  assert.equal(await secondChoice().inputValue(), "unreviewed");
  assert.equal(await duplicateChoice().inputValue(), "ignore");
  assert.equal(await questradeChoice().inputValue(), "sync");
  await duplicateChoice().selectOption("sync");
  await page.getByRole("button", { name: "Save choices", exact: true }).click();
  await selectionDialog().waitFor();
  assert.match(await selectionDialog().innerText(), /restores the hidden account/);
  await selectionDialog().getByRole("button", { name: "Cancel", exact: true }).click();
  assert.equal((await calls("snaptrade_save_account_choices")).length, 1);
  await duplicateChoice().selectOption("ignore");
  console.log("PASS new-account exclusion, durable choices, existing duplicate Ignore, and explicit Resume confirmation");

  await firstChoice().selectOption("link");
  await page.getByLabel("Existing account for Individual (****0580)", { exact: true }).selectOption("1");
  await secondChoice().selectOption("link");
  assert.equal(await page.getByLabel("Existing account for Individual (****7986)", { exact: true }).locator('option[value="1"]').isDisabled(), true);
  assert.equal(await page.getByRole("button", { name: "Save choices", exact: true }).isDisabled(), true);
  await secondChoice().selectOption("unreviewed");
  await page.getByRole("button", { name: "Save choices", exact: true }).click();
  await selectionDialog().waitFor();
  assert.match(await selectionDialog().innerText(), /switches its source from SimpleFIN/);
  await selectionDialog().getByRole("button", { name: "Confirm and save", exact: true }).click();
  await page.getByText("Account choices saved. Sync now updates only selected accounts.", { exact: true }).waitFor();
  const linkSave = (await calls("snaptrade_save_account_choices")).at(-1);
  assert.deepEqual(linkSave.payload.payload.choices, [{ remote_id: "snap-1", action: "link", account_id: 1 }]);
  await page.getByRole("button", { name: "Banks via SimpleFIN", exact: true }).click();
  await openReview();
  assert.equal(await firstChoice().inputValue(), "ignore");
  await page.getByText(/Ignored here; Individual \(0580\) now uses SnapTrade/).waitFor();
  console.log("PASS explicit source-switch confirmation, duplicate-target guard, and shared SimpleFIN review");

  await page.getByRole("button", { name: "Back", exact: true }).click();
  await page.getByRole("button", { name: "Brokerages via SnapTrade", exact: true }).click();
  await fail("snaptrade_discover_accounts");
  await page.getByRole("button", { name: "Choose accounts to sync", exact: true }).click();
  await page.getByRole("alert").filter({ hasText: "Synthetic failure for snaptrade_discover_accounts" }).waitFor();
  assert.equal(await page.getByRole("button", { name: "Save choices", exact: true }).isDisabled(), true);
  await page.getByRole("button", { name: "Reload accounts", exact: true }).click();
  await questradeChoice().waitFor();
  await questradeChoice().selectOption("ignore");
  await fail("snaptrade_save_account_choices");
  await page.getByRole("button", { name: "Save choices", exact: true }).click();
  await page.getByRole("alert").filter({ hasText: "Synthetic failure for snaptrade_save_account_choices" }).waitFor();
  await page.getByRole("button", { name: "Reload accounts", exact: true }).click();
  await questradeChoice().waitFor();
  assert.equal(await questradeChoice().inputValue(), "sync");
  await questradeChoice().selectOption("ignore");
  await firstChoice().selectOption("ignore");
  await page.getByRole("button", { name: "Save choices", exact: true }).click();
  await page.getByText("Account choices saved. Sync now updates only selected accounts.", { exact: true }).waitFor();
  await page.getByRole("button", { name: "Sync now", exact: true }).click();
  await page.getByRole("alert").filter({ hasText: "No accounts are selected" }).waitFor();
  await page.getByRole("button", { name: "Disconnect brokerage", exact: true }).click();
  const disconnect = page.getByRole("dialog", { name: "Disconnect SnapTrade?", exact: true });
  await disconnect.getByRole("button", { name: "Cancel", exact: true }).click();
  assert.equal((await calls("snaptrade_disconnect")).length, 0);
  await page.getByRole("button", { name: "Disconnect brokerage", exact: true }).click();
  await disconnect.getByRole("button", { name: "Disconnect", exact: true }).click();
  await page.getByText("Brokerage disconnected.", { exact: true }).waitFor();
  assert.equal((await calls("snaptrade_disconnect")).length, 1);
  console.log("PASS discovery/save/empty-selection errors and non-native disconnect confirmation");

  await load();
  await page.getByRole("button", { name: "Sync accounts", exact: true }).click();
  await page.getByText(/Received updates for 3 account/).waitFor();
  await page.getByRole("alert").filter({ hasText: "SnapTrade: 3 new account(s) need review" }).waitFor();
  assert.equal((await calls("snaptrade_save_account_choices")).length, 0);
  assert.equal((await calls("simplefin_save_account_choices")).length, 0);
  assert.equal((await calls("snaptrade_sync")).length, 1);
  assert.deepEqual((await calls("simplefin_sync"))[0].payload, { automatic: false });
  await page.getByRole("button", { name: "Delete Roth duplicate", exact: true }).click();
  assert.equal(await page.getByRole("button", { name: "Sync accounts", exact: true }).count(), 0);
  await deleteDialog().getByRole("button", { name: "Delete account", exact: true }).click();
  await deleteDialog().waitFor({ state: "detached" });
  await page.getByRole("button", { name: "Sync accounts", exact: true }).click();
  await page.getByRole("alert").filter({ hasText: "SnapTrade: Error: No accounts are selected" }).waitFor();
  assert.equal(await page.getByRole("button", { name: "Delete Roth duplicate", exact: true }).count(), 0);
  await page.getByText(/Received updates for 2 account/).waitFor();
  console.log("PASS floating/global sync preserves exclusions, exposes review counts, and keeps other providers working");

  await page.goto(`${url}?automatic=1`, { waitUntil: "networkidle" });
  await page.getByText(/SimpleFIN: 1 new account\(s\) need review/).waitFor();
  assert.deepEqual((await calls("simplefin_sync")).map((call) => call.payload), [{ automatic: true }]);
  assert.equal((await calls("snaptrade_sync")).length, 0);
  assert.equal((await calls("simplefin_save_account_choices")).length, 0);
  await page.getByTitle("Connections", { exact: true }).click();
  await page.getByRole("button", { name: "Banks via SimpleFIN", exact: true }).click();
  await page.getByRole("button", { name: "Manage Robinhood in SimpleFIN", exact: true }).click();
  await page.evaluate(() => {
    window.dispatchEvent(new Event("focus"));
    window.dispatchEvent(new Event("focus"));
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.getByText(/Received data for 2 accounts/).waitFor();
  assert.equal((await calls("simplefin_sync")).length, 2);
  assert.deepEqual((await calls("simplefin_sync"))[1].payload, { automatic: false });
  console.log("PASS automatic sync and reauthentication-return sync use saved choices without duplicate focus requests");
  assert.equal(errors.length, 0, errors.join("\n"));
} finally {
  if (browser) await browser.close();
  await server.close();
}
