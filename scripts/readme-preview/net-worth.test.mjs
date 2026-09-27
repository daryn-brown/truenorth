import assert from "node:assert/strict";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { createServer } from "vite";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

test("net-worth category bubbles", { timeout: 60_000 }, async (t) => {
  const server = await createServer({
    root,
    cacheDir: join(root, "node_modules/.cache/net-worth-tests"),
    server: { host: "127.0.0.1", port: 0, strictPort: false, open: false },
  });
  let browser;

  try {
    await server.listen();
    const base = server.resolvedUrls?.local[0];
    assert.ok(base, "The local preview server must provide a URL.");
    const response = await fetch(base);
    assert.equal(response.status, 200);
    browser = await chromium.launch();
    const page = await browser.newPage({
      viewport: { width: 1440, height: 900 },
      colorScheme: "dark",
      reducedMotion: "reduce",
    });
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    await page.route("**/*", (route) => {
      const request = new URL(route.request().url());
      return request.origin === new URL(base).origin ? route.continue() : route.abort();
    });
    await page.goto(new URL("scripts/readme-preview/", base).href, { waitUntil: "networkidle" });
    await page.getByRole("group", { name: "Investments", exact: true }).waitFor();
    const card = page.locator(".net-worth-card");
    const labels = card.locator(".net-worth-orb > span");
    const amounts = card.locator(".net-worth-orb > strong");

    await t.test("shows all three categories in order and toggles their currency", async () => {
      assert.deepEqual(await labels.allTextContents(), ["Investments", "Savings", "Liabilities"]);
      assert.deepEqual(await amounts.allTextContents(), ["$202,500", "$47,790", "$0"]);
      await page.getByTitle("Toggle home currency").click();
      assert.deepEqual(await amounts.allTextContents(), ["US$150,000", "US$35,400", "US$0"]);

      for (const width of [1440, 1360, 960]) {
        await page.setViewportSize({ width, height: 900 });
        const clipped = await card.locator(".net-worth-orb > span, .net-worth-orb > strong")
          .evaluateAll((elements) => elements
            .filter((element) => element.scrollWidth > element.clientWidth + 1)
            .map((element) => element.textContent));
        assert.deepEqual(clipped, [], `Category labels and amounts must fit at ${width}px.`);
      }
    });

    await t.test("renders debt and calls out unclassified assets without calling them savings", async () => {
      await page.evaluate(async () => {
        const { responses } = await import("/scripts/readme-preview/fixtures.ts");
        const nw = responses.get_net_worth;
        nw.allocation.liabilities = { usd: 1250, cad: 1687.5 };
        nw.allocation.unclassified = { usd: 10000, cad: 13500 };
        nw.total_usd += 10000 - 1250;
        nw.total_cad += 13500 - 1687.5;
      });
      await page.getByTitle("Refresh exchange rates").click();
      await card.getByText(/Unclassified assets:/).waitFor();
      assert.deepEqual(await amounts.allTextContents(), ["US$150,000", "US$35,400", "US$1,250"]);
      assert.match(await card.getByText(/Unclassified assets:/).textContent(), /US\$10,000\.00/);
      await page.getByTitle("Toggle home currency").click();
      assert.deepEqual(await amounts.allTextContents(), ["$202,500", "$47,790", "$1,688"]);
      assert.match(await card.getByText(/Unclassified assets:/).textContent(), /\$13,500\.00/);
    });

    await t.test("preserves the first-account prompt when there are no accounts", async () => {
      await page.evaluate(async () => {
        const { responses } = await import("/scripts/readme-preview/fixtures.ts");
        responses.list_accounts = [];
        responses.get_net_worth = {
          ...responses.get_net_worth,
          accounts: [],
          total_usd: 0,
          total_cad: 0,
          allocation: {
            investments: { usd: 0, cad: 0 },
            savings: { usd: 0, cad: 0 },
            liabilities: { usd: 0, cad: 0 },
            unclassified: { usd: 0, cad: 0 },
          },
        };
        responses.get_net_worth_delta = null;
      });
      await page.getByTitle("Refresh exchange rates").click();
      await card.getByText("Add your first account", { exact: true }).waitFor();
      assert.equal(await card.locator(".net-worth-orb").count(), 1);
      assert.equal(await labels.count(), 0);
      assert.equal(await card.getByText(/Unclassified assets:/).count(), 0);
    });

    assert.deepEqual(errors, [], "The preview must not log runtime errors.");
  } finally {
    if (browser) await browser.close();
    await server.close();
  }
});
