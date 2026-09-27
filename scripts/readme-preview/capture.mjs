import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { createServer } from "vite";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const output = join(root, "docs/screenshots");
const server = await createServer({
  root,
  cacheDir: join(root, "node_modules/.cache/readme-preview"),
  server: { host: "127.0.0.1", port: 0, strictPort: false, open: false },
});
let browser;

try {
  await mkdir(output, { recursive: true });
  await server.listen();
  const base = server.resolvedUrls?.local[0];
  if (!base) throw new Error("The local screenshot server did not provide a URL.");
  const url = new URL("scripts/readme-preview/", base).href;
  const response = await fetch(url);
  if (!response.ok) throw new Error(`Preview server returned HTTP ${response.status}.`);

  browser = await chromium.launch();
  const page = await browser.newPage({
    viewport: { width: 1440, height: 1058 },
    deviceScaleFactor: 1,
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
  await page.goto(url, { waitUntil: "networkidle" });
  await page.getByTitle("Mac desktop widget").waitFor();
  await page.getByText("FICTIONAL DEMO DATA", { exact: true }).waitFor();
  const waitForProjection = () => page.waitForFunction(() => {
    const curves = [...document.querySelectorAll("#planning .recharts-line-curve")];
    return curves.length === 2 && curves.every((curve) => {
      const dash = curve.getAttribute("stroke-dasharray");
      return !dash || Number.parseFloat(dash.trim().split(/[ ,]+/).at(-1)) === 0;
    });
  });
  await waitForProjection();
  const syncButton = page.getByRole("button", { name: "Sync accounts", exact: true });
  const beforeSync = await syncButton.boundingBox();
  assert.ok(beforeSync && beforeSync.y > 900, "Sync should float at the bottom of the dashboard.");
  await syncButton.click();
  await page.getByRole("button", { name: "Syncing...", exact: true }).waitFor();
  assert.equal(await page.getByRole("button", { name: "Syncing...", exact: true }).isDisabled(), true);
  await syncButton.waitFor();
  await page.getByText(/Received updates for 5 account/).waitFor();
  assert.equal(await page.getByTitle("Refresh exchange rates").count(), 1);
  const syncCalls = await page.evaluate(async () => {
    const { calls } = await import("/scripts/readme-preview/main.ts");
    return calls.filter((command) => command.endsWith("_sync"));
  });
  assert.deepEqual([...syncCalls].sort(), ["questrade_sync", "simplefin_sync", "snaptrade_sync"]);
  await page.getByText("Figures may be out of date or incomplete", { exact: true }).waitFor();
  await page.screenshot({ path: join(output, "dashboard.png"), animations: "disabled" });

  await page.getByRole("button", { name: "Review connections", exact: true }).click();
  await page.getByRole("heading", { name: "Harbor Bank", exact: true }).waitFor();
  assert.equal(await page.getByRole("button", { name: "Reconnect Harbor Bank in SimpleFIN", exact: true }).count(), 1);
  assert.equal(await page.getByRole("button", { name: "Manage Maple Bank in SimpleFIN", exact: true }).count(), 1);
  await page.getByRole("button", { name: "Reconnect Harbor Bank in SimpleFIN", exact: true }).click();
  await page.getByText("Reconnect Harbor Bank", { exact: true }).waitFor();
  await page.evaluate(async () => {
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    window.dispatchEvent(new Event("focus"));
    await emit("tauri://focus", true);
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.getByText(/Received data for 2 accounts/).waitFor();
  const afterReconnect = await page.evaluate(async () => {
    const { calls } = await import("/scripts/readme-preview/main.ts");
    return calls.filter((command) => command === "simplefin_sync").length;
  });
  assert.equal(afterReconnect, 2, "Browser/native return events should produce one additional SimpleFIN check.");
  await page.getByRole("button", { name: "Done", exact: true }).click();

  await page.locator('a[href="#planning"]').click();
  await page.getByText("Set my goals", { exact: true }).click();
  await page.locator(".desktop-main").evaluate((scroller) => {
    const heading = scroller.querySelector("#planning .dashboard-section__heading");
    const topbar = scroller.querySelector(".desktop-topbar");
    if (!heading || !topbar) throw new Error("The planning layout is missing.");
    const top = scroller.getBoundingClientRect().top + topbar.getBoundingClientRect().height + 24;
    scroller.scrollTop += heading.getBoundingClientRect().top - top;
  });
  await waitForProjection();
  await page.mouse.move(1430, 10);
  const afterScroll = await syncButton.boundingBox();
  assert.ok(afterScroll && Math.abs(afterScroll.y - beforeSync.y) < 2, "Sync must remain visible while the dashboard scrolls.");
  await page.screenshot({ path: join(output, "planning.png"), animations: "disabled" });

  await page.getByRole("link", { name: "Overview", exact: true }).click();
  await page.getByTitle("Mac desktop widget").click();
  await page.getByRole("dialog", { name: "Net worth on your Mac" }).waitFor();
  await page.mouse.move(1430, 10);
  await page.getByRole("dialog").screenshot({
    path: join(output, "mac-widget-settings.png"), animations: "disabled",
  });
  const sharing = page.getByRole("switch");
  assert.equal(await sharing.isChecked(), false, "Widget sharing must start disabled.");
  await sharing.check();
  assert.equal(await sharing.isChecked(), true);
  await sharing.uncheck();
  assert.equal(await sharing.isChecked(), false);
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await page.getByRole("dialog").waitFor({ state: "detached" });
  await page.getByTitle("Mac desktop widget").click();
  await page.getByRole("dialog", { name: "Net worth on your Mac" }).waitFor();
  await page.keyboard.press("Escape");
  await page.getByRole("dialog").waitFor({ state: "detached" });
  if (errors.length > 0) throw new Error(`UI errors while capturing screenshots:\n${errors.join("\n")}`);
  console.log(`Saved three fictional-data UI screenshots to ${output}.`);
} finally {
  if (browser) await browser.close();
  await server.close();
}
