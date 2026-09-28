import assert from "node:assert/strict";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { createServer } from "vite";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const closeButton = (page, name) => page.getByRole("button", { name, exact: true });
const mainNotices = [
  "Dismiss connection warnings",
  "Dismiss sync notice",
  "Dismiss balance warning",
];

async function refreshDashboard(page) {
  await page.getByTitle("Refresh exchange rates").click();
  await page.waitForFunction(() =>
    !document.querySelector('button[title="Refresh exchange rates"]').disabled);
}

async function syncAccounts(page) {
  await page.getByRole("button", { name: "Sync accounts", exact: true }).click();
  await page.getByRole("button", { name: "Sync accounts", exact: true }).waitFor();
}

async function addSyncWarnings(page) {
  await page.evaluate(async () => {
    const { responses } = await import("/scripts/readme-preview/fixtures.ts");
    responses.simplefin_sync.warnings = ["Bank approval is required.", "Some balances are stale."];
    responses.snaptrade_sync.warnings = ["No accounts are selected for sync."];
  });
  await syncAccounts(page);
}

async function failCommand(page, command, message) {
  await page.evaluate(async ({ command, message }) => {
    const { responses } = await import("/scripts/readme-preview/fixtures.ts");
    Object.defineProperty(responses, command, {
      configurable: true,
      get() { throw new Error(`banner-test: ${message}`); },
    });
  }, { command, message });
}

test("dismissible dashboard banners", { timeout: 120_000 }, async (t) => {
  const server = await createServer({
    root,
    cacheDir: join(root, "node_modules/.cache/banner-tests"),
    server: { host: "127.0.0.1", port: 0, strictPort: false, open: false },
  });
  let browser;
  let page;
  let runtimeErrors;
  let consoleErrors;

  try {
    await server.listen();
    const base = server.resolvedUrls?.local[0];
    assert.ok(base, "The local preview server must provide a URL.");
    assert.equal((await fetch(base)).status, 200);
    browser = await chromium.launch();

    t.beforeEach(async () => {
      page = await browser.newPage({
        viewport: { width: 1440, height: 1000 },
        colorScheme: "dark",
        reducedMotion: "reduce",
      });
      runtimeErrors = [];
      consoleErrors = [];
      page.on("pageerror", (error) => runtimeErrors.push(error.message));
      page.on("console", (message) => {
        if (message.type() === "error") consoleErrors.push(message.text());
      });
      await page.route("**/*", (route) =>
        new URL(route.request().url()).origin === new URL(base).origin
          ? route.continue()
          : route.abort());
      await page.goto(new URL("scripts/readme-preview/", base).href, { waitUntil: "networkidle" });
      await closeButton(page, "Dismiss balance warning").waitFor();
    });

    t.afterEach(async () => {
      try {
        assert.deepEqual(runtimeErrors, [], "Banners must not cause runtime errors.");
        assert.deepEqual(
          consoleErrors.filter((message) => !message.includes("banner-test:")),
          [],
          "Only deliberately injected command failures may be logged.",
        );
      } finally {
        await page.close();
      }
    });

    await t.test("dismisses each banner independently with mouse and keyboard", async () => {
      await addSyncWarnings(page);
      const amount = await page.locator(".net-worth-card__amount").textContent();
      await page.getByRole("button", { name: "Review connections", exact: true }).first().click();
      const modal = page.locator(".tn-connections-modal");
      await modal.getByText("Approve the bank connection in your banking app.", { exact: true }).first().waitFor();
      await page.locator(".tn-modal-overlay").click({ position: { x: 5, y: 5 } });

      for (const width of [1440, 960]) {
        await page.setViewportSize({ width, height: 1000 });
        const layout = await page.locator(".desktop-dismissible-banner").evaluateAll((banners) =>
          banners.map((banner) => {
            const bounds = banner.getBoundingClientRect();
            const content = banner.querySelector(".desktop-dismissible-banner__content").getBoundingClientRect();
            const close = banner.querySelector("button.desktop-dismissible-banner__close").getBoundingClientRect();
            return {
              fits: close.left >= content.right && close.right <= bounds.right && close.bottom <= bounds.bottom,
              targetSize: close.width >= 24 && close.height >= 24,
            };
          }));
        assert.equal(layout.length, 3);
        assert.ok(layout.every(({ fits, targetSize }) => fits && targetSize),
          `Close buttons must fit beside the content at ${width}px.`);
      }

      await closeButton(page, "Dismiss connection warnings").click();
      assert.equal(await closeButton(page, "Dismiss connection warnings").count(), 0);
      assert.equal(await closeButton(page, "Dismiss sync notice").count(), 1);
      assert.equal(await closeButton(page, "Dismiss balance warning").count(), 1);

      await closeButton(page, "Dismiss sync notice").focus();
      await page.keyboard.press("Enter");
      assert.equal(await closeButton(page, "Dismiss sync notice").count(), 0);
      await closeButton(page, "Dismiss balance warning").focus();
      await page.keyboard.press("Space");
      assert.equal(await closeButton(page, "Dismiss balance warning").count(), 0);
      assert.equal(await page.locator(".desktop-dismissible-banner").count(), 0);
      assert.equal(await page.locator(".net-worth-card__amount").textContent(), amount);
      assert.match(await page.locator(".desktop-topbar__status").textContent(), /Connections need attention/);

      await page.getByTitle("Connections", { exact: true }).click();
      await modal.getByText("Approve the bank connection in your banking app.", { exact: true }).first().waitFor();
    });

    await t.test("keeps unchanged notices dismissed across polling, refreshes, and syncs", async () => {
      await page.clock.install();
      await page.evaluate(async () => {
        const { simplefinStatus } = await import("/scripts/readme-preview/fixtures.ts");
        simplefinStatus.messages = ["Check your connections.", "Some balances are stale."];
        simplefinStatus.connections[1].status = "stale";
        simplefinStatus.connections[1].accounts[0].health.status = "stale";
      });
      await addSyncWarnings(page);
      for (const name of mainNotices) await closeButton(page, name).click();

      await page.evaluate(async () => {
        const { responses, simplefinStatus } = await import("/scripts/readme-preview/fixtures.ts");
        responses.simplefin_sync.warnings.reverse();
        simplefinStatus.messages.reverse();
        simplefinStatus.connections.reverse();
        simplefinStatus.last_synced_at = new Date().toISOString();
        simplefinStatus.last_attempt_at = new Date().toISOString();
        for (const bank of simplefinStatus.connections) {
          bank.messages.reverse();
          bank.accounts.reverse();
          for (const account of bank.accounts) account.health.balance_as_of = new Date().toISOString();
        }
      });
      const healthCalls = await page.evaluate(async () => {
        const { calls } = await import("/scripts/readme-preview/main.ts");
        return calls.filter((command) => command === "simplefin_get_status").length;
      });
      await page.clock.fastForward(60_000);
      await page.waitForFunction(async (before) => {
        const { calls } = await import("/scripts/readme-preview/main.ts");
        return calls.filter((command) => command === "simplefin_get_status").length > before;
      }, healthCalls);
      await refreshDashboard(page);
      await syncAccounts(page);
      await page.getByTitle("Toggle home currency").click();
      for (const name of mainNotices) assert.equal(await closeButton(page, name).count(), 0);
      assert.match(await page.locator(".desktop-topbar__status").textContent(), /Connections need attention/);
    });

    await t.test("resurfaces changed warnings without restoring unrelated notices", async () => {
      await addSyncWarnings(page);
      for (const name of mainNotices) await closeButton(page, name).click();
      await page.evaluate(async () => {
        const { responses } = await import("/scripts/readme-preview/fixtures.ts");
        responses.simplefin_sync.warnings = ["A different bank needs approval."];
      });
      await syncAccounts(page);
      await closeButton(page, "Dismiss connection warnings").waitFor();
      assert.match(await page.getByRole("alert").textContent(), /A different bank needs approval/);
      assert.equal(await closeButton(page, "Dismiss sync notice").count(), 0);
      assert.equal(await closeButton(page, "Dismiss balance warning").count(), 0);
      await closeButton(page, "Dismiss connection warnings").click();

      await page.evaluate(async () => {
        const { responses } = await import("/scripts/readme-preview/fixtures.ts");
        responses.simplefin_sync.accounts_synced = 1;
      });
      await syncAccounts(page);
      await closeButton(page, "Dismiss sync notice").waitFor();
      assert.equal(await closeButton(page, "Dismiss connection warnings").count(), 0);
      assert.equal(await closeButton(page, "Dismiss balance warning").count(), 0);
      await closeButton(page, "Dismiss sync notice").click();

      await page.evaluate(async () => {
        const { simplefinStatus } = await import("/scripts/readme-preview/fixtures.ts");
        simplefinStatus.connections[0].accounts[0].health.status = "missing";
        simplefinStatus.connections[0].accounts[0].health.message = "No balance was supplied.";
      });
      await refreshDashboard(page);
      await closeButton(page, "Dismiss balance warning").waitFor();
      assert.equal(await closeButton(page, "Dismiss connection warnings").count(), 0);
      assert.equal(await closeButton(page, "Dismiss sync notice").count(), 0);
    });

    await t.test("shows recurring issues after recovery and forgets dismissals on restart", async () => {
      const original = await page.evaluate(async () => {
        const { simplefinStatus } = await import("/scripts/readme-preview/fixtures.ts");
        return structuredClone(simplefinStatus);
      });
      await closeButton(page, "Dismiss balance warning").click();
      await page.evaluate(async () => {
        const { simplefinStatus } = await import("/scripts/readme-preview/fixtures.ts");
        for (const bank of simplefinStatus.connections) {
          bank.status = "current";
          bank.messages = [];
          for (const account of bank.accounts) {
            account.health.status = "current";
            account.health.message = null;
          }
        }
      });
      await refreshDashboard(page);
      assert.equal(await closeButton(page, "Dismiss balance warning").count(), 0);
      assert.match(await page.locator(".desktop-topbar__status").textContent(), /5 accounts linked/);
      await page.evaluate(async (status) => {
        const { simplefinStatus } = await import("/scripts/readme-preview/fixtures.ts");
        Object.assign(simplefinStatus, status);
      }, original);
      await refreshDashboard(page);
      await closeButton(page, "Dismiss balance warning").waitFor();
      await closeButton(page, "Dismiss balance warning").click();
      await page.reload({ waitUntil: "networkidle" });
      await closeButton(page, "Dismiss balance warning").waitFor();
    });

    await t.test("allows exchange-rate errors to be dismissed until their details change", async () => {
      await failCommand(page, "refresh_fx_rates", "Exchange rates unavailable.");
      await refreshDashboard(page);
      await closeButton(page, "Dismiss exchange rate warning").click();
      await refreshDashboard(page);
      assert.equal(await closeButton(page, "Dismiss exchange rate warning").count(), 0);
      await failCommand(page, "refresh_fx_rates", "The exchange-rate request timed out.");
      await refreshDashboard(page);
      await closeButton(page, "Dismiss exchange rate warning").waitFor();
      assert.match(await page.getByRole("alert").textContent(), /request timed out/);
    });

    await t.test("allows load and health errors to be dismissed without clearing their state", async () => {
      await failCommand(page, "get_net_worth", "Dashboard data unavailable.");
      await failCommand(page, "simplefin_get_status", "Connection health unavailable.");
      await refreshDashboard(page);
      await closeButton(page, "Dismiss dashboard warning").click();
      await closeButton(page, "Dismiss connection warnings").click();
      await closeButton(page, "Dismiss balance warning").click();
      await refreshDashboard(page);
      for (const name of ["Dismiss dashboard warning", "Dismiss connection warnings", "Dismiss balance warning"]) {
        assert.equal(await closeButton(page, name).count(), 0);
      }
      assert.match(await page.locator(".desktop-topbar__status").textContent(), /Connections need attention/);

      await failCommand(page, "get_net_worth", "The dashboard request timed out.");
      await failCommand(page, "simplefin_get_status", "The health request timed out.");
      await refreshDashboard(page);
      for (const name of ["Dismiss dashboard warning", "Dismiss connection warnings", "Dismiss balance warning"]) {
        await closeButton(page, name).waitFor();
      }
    });

    await t.test("preserves widget retry after a changed error", async () => {
      await failCommand(page, "refresh_mac_widget", "Widget refresh unavailable.");
      await refreshDashboard(page);
      await closeButton(page, "Dismiss widget warning").click();
      await refreshDashboard(page);
      assert.equal(await closeButton(page, "Dismiss widget warning").count(), 0);
      await failCommand(page, "refresh_mac_widget", "The widget request timed out.");
      await refreshDashboard(page);
      await closeButton(page, "Dismiss widget warning").waitFor();
      await page.evaluate(async () => {
        const { responses, widgetSettings } = await import("/scripts/readme-preview/fixtures.ts");
        Object.defineProperty(responses, "refresh_mac_widget", { configurable: true, value: widgetSettings });
      });
      await page.getByRole("button", { name: "Retry widget refresh", exact: true }).click();
      await closeButton(page, "Dismiss widget warning").waitFor({ state: "detached" });
    });
  } finally {
    if (browser) await browser.close();
    await server.close();
  }
});
