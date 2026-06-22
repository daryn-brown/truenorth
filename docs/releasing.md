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

Versions live in three files and should stay in sync:

- `src-tauri/tauri.conf.json` → `version`
- `package.json` → `version`
- `src-tauri/Cargo.toml` → `package.version`

### Option A — tag (recommended)

```bash
# 1) Bump the version in the three files above, then commit (authored by you — no bot trailers).
git commit -am "Release v0.2.0"

# 2) Tag and push. The tag drives the release name.
git tag v0.2.0
git push origin main --tags
```

The pushed tag (`v0.2.0`) triggers the Release workflow and the draft release uses that tag.

### Option B — manual run

In the **Actions** tab, open **Release → Run workflow**. The release tag is taken from
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

- **Auto-publish instead of draft:** add a final job that flips the release with
  `github.rest.repos.updateRelease({ ..., draft: false })`, depending on `build-tauri`. (Until then,
  remember the updater needs the release **published** — see Auto-update above.)
- **Linux:** intentionally out of scope (macOS + Windows only). Add an `ubuntu-22.04` matrix
  entry with the usual `libwebkit2gtk` system deps if you want Linux bundles later.
