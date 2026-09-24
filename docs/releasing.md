# Releasing

Desktop installers for **macOS (universal)** and **Windows (x64)** are built by GitHub Actions
and attached to a **GitHub Release**. This document explains how to cut a release, what gets
produced, and how to turn on code signing later.

## Pipeline overview

| Workflow | File | Trigger | What it does |
| --- | --- | --- | --- |
| **CI** | [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) | PRs, pushes to `main` | Type-checks + builds the frontend and runs `cargo check` on macOS and Windows. No artifacts. |
| **Release** | [`.github/workflows/release.yml`](../.github/workflows/release.yml) | Tag `v*` or manual run | Builds installers on macOS + Windows and uploads them to a **draft** GitHub Release. |

The Release workflow:

1. **`create-release`** — resolves the tag and creates one **draft** GitHub Release.
2. **`build-tauri`** — a matrix that builds on `macos-latest` (universal: Apple Silicon + Intel)
   and `windows-latest` (x64) using [`tauri-action`](https://github.com/tauri-apps/tauri-action),
   and uploads each platform's bundles to the draft release.

> The release is left as a **draft** so you stay in control — review the assets, then click
> **Publish release**. The workflow never pushes commits back to the repo, so nothing
> bot-authored appears in history or on the contributors list (see [`AGENTS.md`](../AGENTS.md)).

## Cut a release

Versions live in these files and must stay in sync:

- `src-tauri/tauri.conf.json` → `version`
- `package.json` → `version`
- `src-tauri/Cargo.toml` → `package.version`
- `src-tauri/Cargo.lock` → the `truenorth` package version
- `package-lock.json` → the root package versions

The native Mac widget derives its bundle version from `tauri.conf.json`.

### Option A — tag (recommended)

```bash
# After bumping all versions, testing, and building locally:
# commit as the owner, push the feature branch, and open a PR into main.
# Wait for required checks, merge the PR, then verify the version on origin/main.
git fetch origin main
git show origin/main:src-tauri/tauri.conf.json

# Create the tag from merged main, never the unmerged feature branch.
git tag v1.12.0 origin/main
git push origin v1.12.0
```

The pushed tag (`v1.12.0` in this example) triggers the Release workflow and the draft release uses that tag.

### Option B — manual run

Only after the release PR is merged and its version is verified on `origin/main`, open
**Actions → Release → Run workflow** and select **main**. The release tag is taken from
`src-tauri/tauri.conf.json` as `v<version>`. Optionally tick **prerelease**.

When the runs finish, open the draft release, verify the attached installers, and **Publish**.

## What gets built

| Platform | Artifacts |
| --- | --- |
| macOS (universal) | `.dmg` installer and a `.app` bundle (runs on both Apple Silicon and Intel) |
| Windows (x64) | `.msi` (WiX) and `.exe` (NSIS) installers |

## Code signing (optional, signing-ready)

Builds currently ship **unsigned**. They install and run, but users will see OS warnings:

- **macOS** — Gatekeeper blocks the first launch. Right‑click the app → **Open**, or run
  `xattr -dr com.apple.quarantine "/Applications/TrueNorth.app"`.
- **Windows** — SmartScreen shows "Windows protected your PC". Click **More info → Run anyway**.

The pipeline is wired to sign automatically once you add the secrets — no workflow changes needed.

### macOS — sign & notarize

Add these repository secrets (**Settings → Secrets and variables → Actions**):

| Secret | Description |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64 of your **Developer ID Application** `.p12` (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERTIFICATE_PASSWORD` | Password for the `.p12` |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | Your Apple ID email (for notarization) |
| `APPLE_PASSWORD` | An **app-specific password** for that Apple ID |
| `APPLE_TEAM_ID` | Your 10-character Apple Developer Team ID |

Once present, `tauri-action` signs and notarizes the macOS build automatically.

Signed macOS builds also include the [native net-worth widget](mac-widget.md).
The workflow first imports the certificate for the separately signed WidgetKit extension,
then generates a Tauri overlay containing the shared app-group entitlement and native bundle
files. Both binaries use the real `APPLE_TEAM_ID`; no registered app group or provisioning
profile is required for this macOS-only team-prefixed group. Unsigned builds omit the widget
and continue to work normally. An Apple Development identity is suitable for local widget
builds, but releases need the Developer ID identity and notarization credentials above.

### Windows — sign

Tauri reads signing config from `src-tauri/tauri.conf.json` under `bundle.windows`. The common
options are a code-signing certificate thumbprint (`certificateThumbprint` + `digestAlgorithm` +
`timestampUrl`) with the cert available on the runner, or [Azure Trusted Signing](https://v2.tauri.app/distribute/sign/windows/).
See the Tauri Windows signing guide and add the relevant secrets/config when you obtain a cert.

## Auto-update (in-app updater)

The app ships with the Tauri **updater** enabled. On launch it checks the latest **published**
GitHub release for a newer signed build and prompts the user to download + install it
(`src/components/UpdatePrompt.tsx`). The update manifest is read from
`https://github.com/daryn-brown/truenorth/releases/latest/download/latest.json`, which
`tauri-action` generates and uploads when `bundle.createUpdaterArtifacts` is `true`.

**One-time setup — add the updater signing key as a secret:**

A signing keypair was generated with `npm run tauri signer generate`. The **public** key is
committed in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`); the **private** key must be
added to CI so releases can be signed:

| Secret | Description |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | Contents of the generated private key file (keep it secret — never commit it) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The key's password, or leave **empty** if it was generated without one |

To generate a fresh keypair (e.g. to rotate it):

```bash
npm run tauri signer generate -- -w ~/.tauri/truenorth-updater.key
# Paste the printed public key into src-tauri/tauri.conf.json -> plugins.updater.pubkey,
# and add the private key file contents to the TAURI_SIGNING_PRIVATE_KEY secret.
```

> Losing the private key means you can't sign updates the installed app will accept — back it up.
> The public key in `tauri.conf.json` and the private key in CI must always be a matching pair.

**Important — publish, don't leave as draft.** The updater only sees **published** releases, and
the new release's `version` must be **higher** than what's installed. So the flow is: bump the
version → let CI build the draft → review → **Publish release**. Installed apps then pick it up on
their next launch.

## Notes

### README screenshots

The committed screenshots use the real React UI with typed fictional fixtures and Tauri's
official IPC mock. The preview never starts the Rust app or opens a finance database.

```bash
npm ci
npx playwright install chromium
npm run screenshots:readme
```

This captures the overview, planning studio, and widget settings into `docs/screenshots/`.
It also exercises the sharing switch and dialog close/reopen behavior. Browser requests are
restricted to the local preview server. For interactive previewing, run `npm run preview:readme`
and open `http://127.0.0.1:1422/scripts/readme-preview/`.

Keep screenshots fictional; do not replace them with captures of a populated personal app.
The README's download button points to `/releases/latest`, so it does not need a new URL for
each version.

### Other release notes

- **Auto-publish instead of draft:** add a final job that flips the release with
  `github.rest.repos.updateRelease({ ..., draft: false })`, depending on `build-tauri`. (Until then,
  remember the updater needs the release **published** — see Auto-update above.)
- **Linux:** intentionally out of scope (macOS + Windows only). Add an `ubuntu-22.04` matrix
  entry with the usual `libwebkit2gtk` system deps if you want Linux bundles later.
