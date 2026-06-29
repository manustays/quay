# Packaging & Distribution (macOS)

How to turn the source into a distributable macOS app — a `.app` bundle and a `.dmg`, optionally **code-signed** and **notarized** so it opens cleanly on other people's Macs.

Bundling is handled by the Tauri CLI (`tauri build`), configured in `src-tauri/tauri.conf.json` under `"bundle"`. The current config bundles `"targets": "all"`, which on macOS produces both an `.app` and a `.dmg`.

## 1. Build an unsigned bundle

```bash
npm install
npm run tauri build
```

Outputs:

| Artifact | Path |
|----------|------|
| App bundle | `src-tauri/target/release/bundle/macos/Quay.app` |
| Disk image | `src-tauri/target/release/bundle/dmg/Quay_0.5.0_<arch>.dmg` |

`<arch>` is `aarch64` on Apple Silicon or `x64` on Intel. An unsigned build runs locally but triggers a Gatekeeper warning on other machines (see [Installation](installation.md#app-cant-be-opened-because-it-is-from-an-unidentified-developer)).

## 2. Build a universal (Intel + Apple Silicon) binary

To ship one app that runs natively on both architectures, add the Rust targets once:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

Then build universal:

```bash
npm run tauri build -- --target universal-apple-darwin
```

The bundle lands under `src-tauri/target/universal-apple-darwin/release/bundle/`.

## 3. Code signing

To distribute without the "unidentified developer" warning you need an **Apple Developer ID Application** certificate (a paid Apple Developer account).

1. In Xcode (or via the Developer portal), create/download a **Developer ID Application** certificate into your login keychain.
2. Find its identity name:
   ```bash
   security find-identity -v -p codesigning
   # e.g. "Developer ID Application: Your Name (TEAMID1234)"
   ```
3. Tell Tauri to sign with it. Either set it in `src-tauri/tauri.conf.json`:
   ```jsonc
   "bundle": {
     "macOS": {
       "signingIdentity": "Developer ID Application: Your Name (TEAMID1234)"
     }
   }
   ```
   …or pass it via environment variable at build time:
   ```bash
   export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID1234)"
   npm run tauri build
   ```

Tauri signs the `.app` (and the binaries inside) during the bundle step.

### Entitlements

This app does not require special entitlements for its core behavior (spawning child processes, opening Terminal via `osascript`, hitting localhost). If you later add capabilities that need hardened-runtime entitlements, create an entitlements plist and reference it under `bundle.macOS.entitlements` in `tauri.conf.json`.

> Note: the app uses macOS private API (`macOSPrivateApi: true`, for the transparent/positioned popover). This is fine for **Developer ID** distribution (direct download). It is **not** allowed on the Mac App Store — App Store submission would require removing that flag.

## 4. Notarization

Notarization is Apple scanning your signed app and stapling an approval ticket, so Gatekeeper trusts it on first launch.

You need:

- Your **Apple ID** email.
- An **app-specific password** for that Apple ID (create at [appleid.apple.com](https://appleid.apple.com) → Sign-In and Security → App-Specific Passwords).
- Your **Team ID** (from the Developer portal).

Tauri can notarize automatically during `tauri build` when these environment variables are set:

```bash
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="abcd-efgh-ijkl-mnop"   # the app-specific password
export APPLE_TEAM_ID="TEAMID1234"
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID1234)"

npm run tauri build
```

Tauri will sign, notarize, and staple the ticket to the bundle. The resulting `.dmg` opens without warnings on any Mac.

### Verifying

```bash
# signature
codesign --verify --deep --strict --verbose=2 "src-tauri/target/release/bundle/macos/Quay.app"

# notarization / Gatekeeper assessment
spctl -a -vvv -t install "src-tauri/target/release/bundle/macos/Quay.app"
```

A notarized app reports `source=Notarized Developer ID` and `accepted`.

## 5. Versioning a release

Bump the version in **both**:

- `src-tauri/tauri.conf.json` → `"version"`
- `package.json` → `"version"`

(Keep them in sync — Tauri uses `tauri.conf.json`; npm scripts read `package.json`.) Then build, and tag the release in git:

```bash
git tag v0.5.0
git push origin v0.5.0
```

Pushing the tag triggers the CI pipeline (§6), which builds the universal `.dmg` and drafts a GitHub Release with it attached — just review and publish. (To build and attach a `.dmg` by hand instead, run §1–§4 locally and upload it to a new Release.)

## 6. CI release pipeline

The repo ships a GitHub Actions workflow at [`.github/workflows/release.yml`](../.github/workflows/release.yml) that builds and publishes releases automatically.

**Trigger.** Push a version tag:

```bash
# bump version in both files first (see §5), then:
git tag v0.5.0
git push origin v0.5.0
```

**What it does.** On a `macos-latest` runner it installs the `aarch64`/`x86_64` Rust targets, runs [`tauri-apps/tauri-action`](https://github.com/tauri-apps/tauri-action) with `--target universal-apple-darwin` (one `.dmg` for both architectures), and creates a **draft** GitHub Release with the `.dmg` attached. Review the draft, then publish it — that's what the README's "Download for macOS" link resolves to (`releases/latest`).

**Signing is conditional.** The workflow runs **unsigned by default** and automatically signs + notarizes only when these are set as encrypted repository secrets (Settings → Secrets and variables → Actions):

| Secret | Value |
|--------|-------|
| `APPLE_CERTIFICATE` | base64 of your Developer ID Application `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | password for that `.p12` |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Your Name (TEAMID1234)` |
| `APPLE_ID` | your Apple ID email |
| `APPLE_PASSWORD` | app-specific password (see §4) |
| `APPLE_TEAM_ID` | your Team ID |

With the secrets unset the pipeline still succeeds and produces an **unsigned** universal `.dmg` (users open it via the "Open Anyway" step documented in [Installation](installation.md#opening-an-unsigned-build)). Add the secrets later to flip every subsequent release to signed + notarized — no workflow change needed.

**Adding the secrets (when you're ready to sign).** First base64-encode your exported Developer ID Application certificate so it survives as a single-line secret:

```bash
base64 -i DeveloperID.p12 | pbcopy   # copies the encoded cert to the clipboard
```

Then, in the GitHub repo: **Settings → Secrets and variables → Actions → New repository secret**. Add each row from the table above as its own secret (name = the `APPLE_*` key, value = the corresponding value; paste the clipboard for `APPLE_CERTIFICATE`). Once all six exist, the next tag you push produces a signed + notarized `.dmg` automatically.

## Troubleshooting the DMG build

### `failed to run bundle_dmg.sh` (Automation permission)

`npm run tauri build` compiles the app and bundles the `.app`, then fails when building the `.dmg`:

```
Bundling Quay_0.5.0_aarch64.dmg ...
     Running bundle_dmg.sh
failed to bundle project: error running bundle_dmg.sh
```

**Cause.** Tauri's `bundle_dmg.sh` (a fork of `create-dmg`) runs an **AppleScript** that tells **Finder** to lay out the DMG window (icon positions, background). The first time the build process sends Apple events to Finder, macOS requires **Automation consent**. Until that's granted, `osascript` returns `-1743 (Not authorized to send Apple events)`, and because the script runs with `set -e`, it aborts with the generic message above. The `.app` is already built at this point — only the cosmetic DMG styling failed.

**Fix (interactive build).** Grant the consent, then re-run:

1. Re-run `npm run tauri build` and **approve** the “… wants to control Finder” / “System Events” prompt if it appears, **or**
2. Pre-grant it under **System Settings → Privacy & Security → Automation** — allow your terminal (Terminal/iTerm) to control **Finder** (and **System Events**), then re-run.

Once consent exists the DMG builds every time. (The recompile is skipped on a re-run — only the bundling step repeats.) To confirm the underlying error yourself, run with `--verbose`:

```bash
npm run tauri build -- --verbose
```

and look for the AppleScript / `-1743` line.

**Fix (CI / headless, no Finder).** A machine with no GUI/Finder session can't run the styling AppleScript at all. Options:

- Build only the app and skip the DMG by setting `"bundle": { "targets": ["app"] }` in `src-tauri/tauri.conf.json` (or `--bundles app` on the CLI), then zip the `.app` for distribution, **or**
- Use [`tauri-apps/tauri-action`](https://github.com/tauri-apps/tauri-action) in GitHub Actions on a `macos` runner, which runs in a context where the DMG step works.

### Stale temp image / mounted volume

A previously **failed** DMG run can leave a read-write temp image (`rw.<n>.*.dmg`) under `target/release/bundle/macos/` or a mounted `/Volumes/dmg.*`. These don't normally block a retry (the temp name is randomized), but if a build complains about a busy device, detach any leftover mount and remove the temp image:

```bash
hdiutil info | grep /Volumes/dmg     # find stray mounts
hdiutil detach /Volumes/dmg.XXXXXX   # detach it
rm -f src-tauri/target/release/bundle/macos/rw.*.dmg
```

## Quick reference

| Goal | Command |
|------|---------|
| Local unsigned build | `npm run tauri build` |
| App only (skip DMG) | `npm run tauri build -- --bundles app` |
| Universal binary | `npm run tauri build -- --target universal-apple-darwin` |
| Signed + notarized | set `APPLE_*` env vars, then `npm run tauri build` |
| Verify signature | `codesign --verify --deep --strict -v=2 <app>` |
| Verify notarization | `spctl -a -vvv -t install <app>` |
| Debug a bundle failure | `npm run tauri build -- --verbose` |
