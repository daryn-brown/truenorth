import assert from "node:assert/strict";
import { test } from "node:test";
import { architectures, bundleConfiguration, entitlements, widgetInfo, widgetProfile } from "./macos-widget.mjs";

const base = { identifier: "com.example.finance", productName: "Finance", version: "1.12.0" };
const env = { APPLE_SIGNING_IDENTITY: "Apple Development: Example", APPLE_TEAM_ID: "ABCDE12345" };
const files = {
  appEntitlements: "/build/App.entitlements",
  appInfo: "/build/App-Info.plist",
  extension: "/build/TrueNorthWidget.appex",
  bridge: "/build/libTrueNorthWidgetBridge.dylib",
};

test("widget targets match the parent, including universal releases", () => {
  assert.deepEqual(architectures("universal-apple-darwin"), ["arm64", "x86_64"]);
  assert.deepEqual(architectures("aarch64-apple-darwin"), ["arm64"]);
  assert.deepEqual(architectures("x86_64-apple-darwin"), ["x86_64"]);
  assert.deepEqual(architectures(undefined, "arm64"), ["arm64"]);
  assert.throws(() => architectures("x86_64-pc-windows-msvc"), /Unsupported/);
});

test("real signing identity and matching team configuration are mandatory", () => {
  assert.throws(() => widgetProfile(base, {}), /APPLE_SIGNING_IDENTITY/);
  assert.throws(() => widgetProfile(base, { ...env, APPLE_SIGNING_IDENTITY: "-" }), /Ad-hoc/);
  assert.throws(() => widgetProfile(base, { ...env, APPLE_TEAM_ID: "bad" }), /APPLE_TEAM_ID/);
});

test("only the widget is sandboxed, and both binaries share exactly one group", () => {
  const profile = widgetProfile(base, env);
  const claims = entitlements(profile);
  assert.equal(profile.group, "ABCDE12345.com.example.finance.shared");
  assert.deepEqual(claims.app["com.apple.security.application-groups"], [profile.group]);
  assert.deepEqual(claims.widget["com.apple.security.application-groups"], [profile.group]);
  assert.equal(claims.app["com.apple.security.app-sandbox"], undefined);
  assert.equal(claims.widget["com.apple.security.app-sandbox"], true);
  assert.equal(Object.keys(claims.widget).length, 2);
});

test("local builds keep the existing app identity without requiring updater artifacts", () => {
  const release = widgetProfile(base, env);
  const local = widgetProfile(base, { ...env, TRUENORTH_WIDGET_LOCAL: "1" });
  assert.equal(local.identifier, release.identifier);
  assert.equal(local.group, release.group);
  assert.equal(local.productName, release.productName);
  assert.equal(local.local, true);
  assert.equal(release.local, false);
  assert.equal(bundleConfiguration(local, files).bundle.createUpdaterArtifacts, false);
  assert.equal(bundleConfiguration(release, files).bundle.createUpdaterArtifacts, undefined);
});

test("the signed overlay builds before bundling and embeds the exact native products", () => {
  const profile = widgetProfile(base, env);
  const configuration = bundleConfiguration(profile, files);
  assert.equal(configuration.build.beforeBundleCommand, "node scripts/macos-widget.mjs prepare");
  assert.equal(configuration.bundle.macOS.signingIdentity, env.APPLE_SIGNING_IDENTITY);
  assert.equal(configuration.bundle.macOS.entitlements, files.appEntitlements);
  assert.equal(configuration.bundle.macOS.infoPlist, files.appInfo);
  assert.deepEqual(configuration.bundle.macOS.files, {
    "PlugIns/TrueNorthWidget.appex": files.extension,
    "Frameworks/libTrueNorthWidgetBridge.dylib": files.bridge,
  });
});

test("the extension has native WidgetKit metadata and the app's version", () => {
  const profile = widgetProfile(base, env);
  const info = widgetInfo(profile);
  assert.equal(info.NSExtension.NSExtensionPointIdentifier, "com.apple.widgetkit-extension");
  assert.equal(info.CFBundleIdentifier, `${profile.identifier}.NetWorthWidget`);
  assert.equal(info.CFBundleShortVersionString, base.version);
  assert.equal(info.CFBundleVersion, base.version);
  assert.equal(info.CFBundlePackageType, "XPC!");
  assert.equal(info.LSMinimumSystemVersion, "14.0");
  assert.equal(info.TrueNorthWidgetAppGroup, profile.group);
});
