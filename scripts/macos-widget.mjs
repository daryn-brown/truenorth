import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const output = join(root, "src-tauri/target/macos-widget");
const configPath = join(output, "tauri.widget.conf.json");
const sources = join(root, "src-tauri/macos/widget");

export function architectures(target, hostArch = process.arch) {
  switch (target || (hostArch === "arm64" ? "aarch64-apple-darwin" : "x86_64-apple-darwin")) {
    case "universal-apple-darwin": return ["arm64", "x86_64"];
    case "aarch64-apple-darwin": return ["arm64"];
    case "x86_64-apple-darwin": return ["x86_64"];
    default: throw new Error(`Unsupported widget build target: ${target}`);
  }
}

export function widgetProfile(base, env) {
  const identity = env.APPLE_SIGNING_IDENTITY?.trim();
  const team = env.APPLE_TEAM_ID?.trim();
  if (!identity || identity === "-") {
    throw new Error("Set APPLE_SIGNING_IDENTITY to an installed Apple signing identity. Ad-hoc signing cannot authorize the widget app group.");
  }
  if (!/^[A-Z0-9]{10}$/.test(team ?? "")) {
    throw new Error("Set APPLE_TEAM_ID to the 10-character team ID from that signing certificate.");
  }
  const identifier = base.identifier;
  return {
    identity,
    team,
    identifier,
    group: `${team}.${identifier}.shared`,
    productName: base.productName,
    version: base.version,
    local: env.TRUENORTH_WIDGET_LOCAL === "1",
  };
}

export function entitlements(profile) {
  const app = { "com.apple.security.application-groups": [profile.group] };
  return {
    app,
    widget: { ...app, "com.apple.security.app-sandbox": true },
  };
}

export function widgetInfo(profile) {
  return {
    CFBundleDisplayName: `${profile.productName} Net Worth`,
    CFBundleName: "TrueNorthWidget",
    CFBundleExecutable: "TrueNorthWidget",
    CFBundleIdentifier: `${profile.identifier}.NetWorthWidget`,
    CFBundleInfoDictionaryVersion: "6.0",
    CFBundlePackageType: "XPC!",
    CFBundleShortVersionString: profile.version,
    CFBundleVersion: profile.version,
    CFBundleSupportedPlatforms: ["MacOSX"],
    LSMinimumSystemVersion: "14.0",
    TrueNorthWidgetAppGroup: profile.group,
    NSExtension: { NSExtensionPointIdentifier: "com.apple.widgetkit-extension" },
  };
}

function run(command, args, { capture = false, input } = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    stdio: capture || input !== undefined ? "pipe" : "inherit",
    input,
    env: process.env,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} failed (${result.status}): ${result.stderr ?? ""}${result.stdout ?? ""}`);
  }
  return `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
}

function plist(path, value) {
  run("/usr/bin/plutil", ["-convert", "xml1", "-o", path, "-"], { input: JSON.stringify(value) });
}

function baseConfig() {
  return JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"));
}

function paths() {
  const directory = join(output, "signed");
  return {
    directory,
    extension: join(directory, "TrueNorthWidget.appex"),
    bridge: join(directory, "libTrueNorthWidgetBridge.dylib"),
    appEntitlements: join(directory, "App.entitlements"),
    widgetEntitlements: join(directory, "Widget.entitlements"),
    appInfo: join(directory, "App-Info.plist"),
  };
}

export function bundleConfiguration(profile, files) {
  return {
    identifier: profile.identifier,
    productName: profile.productName,
    build: { beforeBundleCommand: "node scripts/macos-widget.mjs prepare" },
    bundle: {
      ...(profile.local ? { createUpdaterArtifacts: false } : {}),
      macOS: {
        signingIdentity: profile.identity,
        entitlements: files.appEntitlements,
        infoPlist: files.appInfo,
        files: {
          "PlugIns/TrueNorthWidget.appex": files.extension,
          "Frameworks/libTrueNorthWidgetBridge.dylib": files.bridge,
        },
      },
    },
  };
}

function configure(profile) {
  const files = paths();
  mkdirSync(files.directory, { recursive: true });
  const claims = entitlements(profile);
  plist(files.appEntitlements, claims.app);
  plist(files.widgetEntitlements, claims.widget);
  plist(files.appInfo, { TrueNorthWidgetAppGroup: profile.group });
  writeFileSync(configPath, JSON.stringify(bundleConfiguration(profile, files), null, 2) + "\n");
  return files;
}

function verifyTeam(path, team) {
  const signature = run("/usr/bin/codesign", ["--display", "--verbose=4", path], { capture: true });
  if (!signature.split("\n").includes(`TeamIdentifier=${team}`)) {
    throw new Error(`The signature on ${path} does not belong to APPLE_TEAM_ID=${team}.`);
  }
}

function sign(path, profile, entitlementPath) {
  const args = ["--force", "--sign", profile.identity, "--options", "runtime"];
  args.push(profile.identity.startsWith("Apple Development:") ? "--timestamp=none" : "--timestamp");
  if (entitlementPath) args.push("--entitlements", entitlementPath);
  run("/usr/bin/codesign", [...args, path]);
  verifyTeam(path, profile.team);
}

function compile(directory, archs) {
  const sdk = run("xcrun", ["--sdk", "macosx", "--show-sdk-path"], { capture: true });
  const widgets = [];
  const bridges = [];
  for (const arch of archs) {
    const slice = join(directory, arch);
    mkdirSync(slice, { recursive: true });
    const common = [
      "swiftc", "-swift-version", "5", "-O", "-parse-as-library",
      "-sdk", sdk, "-target", `${arch}-apple-macosx14.0`,
      "-module-cache-path", join(output, "module-cache"),
      join(sources, "Shared/WidgetSnapshot.swift"),
    ];
    const widget = join(slice, "TrueNorthWidget");
    run("xcrun", [
      ...common, "-emit-executable", "-application-extension", "-module-name", "TrueNorthWidgets",
      join(sources, "TrueNorthWidget.swift"),
      // Extension startup discovers Swift @main through the separate __swift5_entry record.
      "-Xlinker", "-e", "-Xlinker", "_NSExtensionMain",
      "-o", widget,
    ]);
    const bridge = join(slice, "libTrueNorthWidgetBridge.dylib");
    run("xcrun", [
      ...common, "-emit-library", "-module-name", "TrueNorthWidgetBridge",
      join(sources, "WidgetBridge.swift"),
      "-Xlinker", "-install_name", "-Xlinker", "@rpath/libTrueNorthWidgetBridge.dylib",
      "-o", bridge,
    ]);
    widgets.push(widget);
    bridges.push(bridge);
  }
  const extension = join(directory, "TrueNorthWidget.appex");
  const executable = join(extension, "Contents/MacOS/TrueNorthWidget");
  mkdirSync(dirname(executable), { recursive: true });
  const bridge = join(directory, "libTrueNorthWidgetBridge.dylib");
  for (const [slices, destination] of [[widgets, executable], [bridges, bridge]]) {
    if (slices.length === 1) copyFileSync(slices[0], destination);
    else run("xcrun", ["lipo", "-create", ...slices, "-output", destination]);
    const actual = run("xcrun", ["lipo", "-archs", destination], { capture: true }).split(/\s+/);
    if (actual.length !== archs.length || archs.some((arch) => !actual.includes(arch))) {
      throw new Error(`Native binary architectures do not match the app: ${destination} (${actual.join(", ")}).`);
    }
  }
  for (const arch of archs) {
    const headers = run("xcrun", ["otool", "-arch", arch, "-l", executable], { capture: true });
    if (!/sectname\s+__swift5_entry\b/.test(headers)) {
      throw new Error(`The ${arch} widget has no discoverable Swift @main entry.`);
    }
  }
  return { extension, bridge };
}

function prepare(profile) {
  const files = configure(profile);
  compile(files.directory, architectures(process.env.TAURI_ENV_TARGET_TRIPLE));
  plist(join(files.extension, "Contents/Info.plist"), widgetInfo(profile));
  sign(files.bridge, profile);
  sign(files.extension, profile, files.widgetEntitlements);
  console.log(`Prepared signed widget for ${profile.productName} (${profile.team}).`);
}

function verifyBundle(app, profile) {
  const extension = join(app, "Contents/PlugIns/TrueNorthWidget.appex");
  const bridge = join(app, "Contents/Frameworks/libTrueNorthWidgetBridge.dylib");
  for (const artifact of [extension, bridge, app]) {
    run("/usr/bin/codesign", ["--verify", "--strict", "--verbose=2", artifact]);
    verifyTeam(artifact, profile.team);
  }
  for (const artifact of [app, extension]) {
    const result = spawnSync("/usr/bin/codesign", ["--display", "--entitlements", ":-", artifact], { encoding: "utf8" });
    if (result.status !== 0) throw new Error(result.stderr);
    const claims = JSON.parse(run("/usr/bin/plutil", ["-convert", "json", "-o", "-", "-"], { input: result.stdout }));
    if (!claims["com.apple.security.application-groups"]?.includes(profile.group)) {
      throw new Error(`Missing widget app-group entitlement on ${artifact}.`);
    }
    if (artifact === extension && claims["com.apple.security.app-sandbox"] !== true) {
      throw new Error("The widget extension must be sandboxed.");
    }
  }
  run("/usr/bin/codesign", ["--verify", "--deep", "--strict", app]);
  console.log(`Verified widget bundle: ${app}`);
}

function option(args, key) {
  const index = args.indexOf(key);
  if (index >= 0) return args[index + 1];
  return args.find((value) => value.startsWith(`${key}=`))?.slice(key.length + 1);
}

async function main() {
  if (process.platform !== "darwin") throw new Error("Native widget builds require macOS.");
  const [command, ...args] = process.argv.slice(2);
  if (command === "check") {
    compile(join(output, "check"), ["arm64", "x86_64"]);
    const checks = join(output, "check/WidgetSnapshotChecks");
    run("xcrun", [
      "swiftc", "-swift-version", "5", "-Onone", "-parse-as-library",
      "-module-cache-path", join(output, "module-cache"),
      join(sources, "Shared/WidgetSnapshot.swift"),
      join(sources, "Tests/WidgetSnapshotTests.swift"),
      "-o", checks,
    ]);
    run(checks, []);
    return;
  }
  if (args.includes("--local")) process.env.TRUENORTH_WIDGET_LOCAL = "1";
  const profile = widgetProfile(baseConfig(), process.env);
  if (command === "configure") {
    configure(profile);
  } else if (command === "prepare") {
    prepare(profile);
  } else if (command === "verify") {
    if (!args[0]) throw new Error("Supply the path to the bundled .app.");
    verifyBundle(resolve(args[0]), profile);
  } else if (command === "build") {
    configure(profile);
    const tauriArgs = args.filter((arg) => arg !== "--local");
    if (tauriArgs.includes("--no-bundle") || option(tauriArgs, "--config")) {
      throw new Error("Widget builds require the generated bundle configuration; --no-bundle and additional --config options are not supported.");
    }
    run("npm", ["run", "tauri", "--", "build", "--config", configPath, ...tauriArgs]);
    const target = option(tauriArgs, "--target");
    verifyBundle(join(
      root, "src-tauri/target", target ?? "",
      tauriArgs.includes("--debug") ? "debug" : "release",
      "bundle/macos", `${profile.productName}.app`,
    ), profile);
  } else {
    throw new Error("Usage: macos-widget.mjs build [--local] [tauri build options] | configure | prepare | verify <app> | check");
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
