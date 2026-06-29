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

## 5. Releasing (fully automated)

Releases are driven by [semantic-release](https://semantic-release.gitbook.io/) — **you never bump a version or create a tag by hand.** Land a [Conventional Commit](https://www.conventionalcommits.org/) on `main` and CI does the rest.

| Commit type | Example | Result |
|-------------|---------|--------|
| `fix:` | `fix: detect closed terminal window` | patch release (`0.5.2` → `0.5.3`) |
| `feat:` | `feat: add Docker autostart` | minor release (`0.5.2` → `0.6.0`) |
| `feat!:` / `BREAKING CHANGE:` footer | | major release (`0.5.2` → `1.0.0`) |
| `docs:` / `chore:` / `test:` / `refactor:` | | **no release** (CI runs, decides nothing to ship, exits) |

On a release-worthy commit, the [`Release` workflow](../.github/workflows/release.yml) (on `macos-latest`):

1. Computes the next version from the commits since the last tag.
2. Writes it into `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml` via [`scripts/set-version.mjs`](../scripts/set-version.mjs) (one source of truth — no hand-syncing). `Cargo.lock` is refreshed by the build.
3. Builds the universal `.dmg` ([`scripts/release-build.sh`](../scripts/release-build.sh)).
4. Updates `CHANGELOG.md`, creates the `vX.Y.Z` tag, and **publishes** a GitHub Release with the `.dmg` attached.
5. Commits the bumped files + changelog back to `main` as `chore(release): … [skip ci]` (which does not re-trigger the workflow).

The published release is what the README / marketing **Download** link resolves to (`releases/latest`). There is no draft step — a release-worthy commit ships to users automatically. (To gate that, do feature work on branches and merge deliberately.)

The semantic-release plugin chain lives in [`.releaserc.json`](../.releaserc.json).

## 6. Code signing in CI

The build is **unsigned by default** and automatically signs + notarizes only when a **complete** set of Apple credentials is present as encrypted repository secrets (a partial set is treated as unsigned — see [`scripts/release-build.sh`](../scripts/release-build.sh)). Secrets (Settings → Secrets and variables → Actions):

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

Then, in the GitHub repo: **Settings → Secrets and variables → Actions → New repository secret**. Add each row from the table above as its own secret (name = the `APPLE_*` key, value = the corresponding value; paste the clipboard for `APPLE_CERTIFICATE`). Once all six exist, the next release builds a signed + notarized `.dmg` automatically — no workflow change needed.

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
