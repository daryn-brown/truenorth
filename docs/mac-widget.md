# Mac net-worth widget

TrueNorth includes a native **WidgetKit** net-worth widget for **macOS 14 or later**,
in small and medium sizes. Both sizes show USD and CAD totals. Clicking the widget
opens its containing app.

## Add it to your desktop

1. Install and open a **signed widget build** of TrueNorth.
2. Choose **Widgets** at the bottom of the app's navigation rail.
3. Enable **Share net worth with macOS**.
4. Right-click your desktop, choose **Edit Widgets**, search for **TrueNorth**, and add
   **Net worth**. The widget is also available in Notification Center.

Unsigned releases and `tauri dev` do not include a working native widget. Their Widgets
panel explains the requirement without preventing normal use of the finance app.
Windows is unchanged and does not show the Widgets control.

## Privacy and freshness

Sharing is **off by default**, with consent stored in the encrypted app database.
Enabling it publishes one **unencrypted, local** JSON snapshot containing only the
USD/CAD totals, a status, schema version, export time, and the oldest contributing
balance or exchange-rate date. It contains no account names, institution names,
transactions, holdings, database keys, or connector credentials.

The sandboxed widget reads only this snapshot from the app's macOS app-group
container. It never opens the finance database or connects to a bank. The app creates
the snapshot directory with owner-only access and makes the snapshot owner-readable
and writable. Widget amounts are marked privacy-sensitive, but they are still visible
to people who can see your desktop; do not treat the widget as a secret display.

Snapshots refresh when the dashboard loads after opening the app, syncing, importing,
editing balances/currencies, deleting accounts, or refreshing FX. The app requests a
WidgetKit timeline reload after each export. The widget also requests a reread every
15 minutes, subject to macOS scheduling. With the app closed, it retains the last local
snapshot; after 24 hours it asks you to reopen the app. The displayed data date is the
**oldest underlying balance or FX date**, not a claim that opening the app synced banks.
Missing balances or exchange rates show an explanatory state instead of partial totals.

Turning sharing off persists the opt-out, removes the shared file, and requests a
timeline reload. Cleanup failures are shown in the app and can be retried. macOS can
retain a rendered widget until its next refresh; remove the widget to hide it immediately.

## Build locally

Requirements: macOS, Rust/Node dependencies, Apple's Command Line Tools with a macOS 14+
SDK and Swift compiler, and a real **Apple Development** or **Developer ID Application**
identity installed in the keychain.

```bash
security find-identity -v -p codesigning
export APPLE_SIGNING_IDENTITY="Apple Development: Your Name (IDENTITY)"
export APPLE_TEAM_ID="YOURTEAMID"
npm ci
npm run build:desktop:widget -- --local --bundles app
```

Use the **team ID from the certificate**, which is not necessarily the identifier in
parentheses in its display name. The script verifies the actual code-signature team.
`--local` skips updater artifacts, so this build does not need the updater private key.
The app is created at `src-tauri/target/release/bundle/macos/TrueNorth.app`; the build
does not install or launch it, or replace `/Applications/TrueNorth.app`. When you choose
to open or install it, it uses the existing TrueNorth app identity and local data.

For a release build, omit `--local`; supply the usual updater signing key when
building updater artifacts. Add `--target universal-apple-darwin` to build both Mac
architectures (and install the corresponding Rust targets). The generated configuration
and native outputs stay under the ignored `src-tauri/target/macos-widget/` directory.

The build uses `<actual-team-id>.<app-identifier>.shared`, Apple's macOS-only app-group
format. These groups do not require registration or provisioning profiles for the
unrestricted macOS App Groups entitlement. Both the app and widget must be signed by
that team. Do not substitute ad-hoc signing or an invented team ID.

## Packaging and release

`scripts/macos-widget.mjs configure` generates a Tauri configuration overlay. Its
`beforeBundleCommand` compiles and signs the native bridge and extension before Tauri
copies them into `Contents/Frameworks` and `Contents/PlugIns`. The extension has its own
sandbox/app-group entitlements; the existing Tauri app is **not** newly sandboxed.
Its version is derived from `tauri.conf.json`, including the extension's bundle version.

The signed macOS Release workflow imports the certificate before the native hook,
includes the overlay, and verifies the resulting signatures. Distributable builds
require **Developer ID signing and notarization**; unsigned releases continue to build
without the extension. Modern macOS may require authorization for local development
app-group access. A local Apple Development build is not a substitute for a notarized
release. See [releasing.md](releasing.md).

```bash
npm run test:widget
npm run check:widget:native
cargo test --manifest-path src-tauri/Cargo.toml --lib -- commands::widget::tests commands::net_worth::tests
```

Native checks compile the extension and bridge for both architectures and exercise the
Swift snapshot reader without accessing actual finance data. They do not install a
widget or claim that a particular macOS version has refreshed its gallery.

References: [Apple app groups](https://developer.apple.com/documentation/xcode/configuring-app-groups),
[WidgetKit extensions](https://developer.apple.com/documentation/widgetkit/creating-a-widget-extension),
and [Tauri macOS bundle configuration](https://v2.tauri.app/reference/config/#macconfig).
