# Auto-Update

Quay updates itself in place. On launch (and on demand from the tray menu) it asks
GitHub whether a newer release exists; if so, it prompts the user, downloads the new
build, installs it, and restarts.

This is built on [`tauri-plugin-updater`](https://v2.tauri.app/plugin/updater/) (Tauri
v2). Update packages are signed with Tauri's own **minisign** key pair — this is
**separate from Apple code-signing/notarization**, so auto-update works even on the
current unsigned builds.

## How it works

1. Each release publishes three assets (see [`.releaserc.json`](../.releaserc.json)):
   the `.dmg` (manual download), the `.app.tar.gz` **updater bundle**, and
   `latest.json` (the **manifest**).
2. The app's [`tauri.conf.json`](../src-tauri/tauri.conf.json) holds the updater's
   public key and a single endpoint:
   `https://github.com/manustays/quay/releases/latest/download/latest.json` — always
   the newest release's manifest.
3. `latest.json` lists the new `version`, the download `url` for the `.app.tar.gz`,
   and its `signature`. The client verifies the signature against the embedded public
   key before installing, then swaps the bundle and relaunches.

### Check triggers (`src-tauri/src/lib.rs`)

- **On launch:** a silent check fires ~3s after startup (after the tray + popover are
  settled). If a newer version is found, a native dialog offers *Install & Restart* /
  *Later*. If the app is current or the network is down, nothing is shown.
- **Manual:** the tray menu's **Check for Updates…** runs the same flow but also
  reports "you're on the latest version" and surfaces check errors.
- Both paths share an `update_in_flight` guard (`state.rs`) so a launch check and a
  manual check can't overlap (no duplicate dialogs or racing downloads).

The whole flow lives in Rust (`check_for_updates` / `run_update_check`) — the menubar
app has no frontend update UI, so there is **no** `capabilities/` permission or JS
`@tauri-apps/plugin-updater` dependency to maintain.

## Signing keys (one-time setup)

Auto-update requires a minisign key pair. Generate one and keep the private key safe:

```bash
npm run tauri signer generate -w ~/.tauri/quay_updater.key
```

This prints a **public key** (committed in `tauri.conf.json` → `plugins.updater.pubkey`)
and writes the **private key** to the path you chose. If you lose the private key or
its password you can no longer sign updates that existing installs will accept —
shipping a new key only works for fresh installs.

## CI secrets (required)

The release build emits the `.app.tar.gz.sig` signature **only** when the private key
is available. Add these repository secrets
(**Settings → Secrets and variables → Actions**):

| Secret | Value |
|--------|-------|
| `TAURI_SIGNING_PRIVATE_KEY` | contents of the private key file (`~/.tauri/quay_updater.key`) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the password chosen at `signer generate` |

The [`Release` workflow](../.github/workflows/release.yml) passes these to the build.
[`scripts/make-latest-json.mjs`](../scripts/make-latest-json.mjs) then builds the
manifest from the signed bundle and **aborts the release** if the signature is missing
or empty — a missing key fails loudly rather than publishing a broken `latest.json`.

> If the secrets are absent, the release fails at the manifest step. Auto-update is a
> hard requirement of the pipeline once enabled, unlike the optional Apple signing.

## Verifying

- **Manifest reachable** after a release:
  ```bash
  curl -L https://github.com/manustays/quay/releases/latest/download/latest.json
  ```
  Should return JSON whose `version` matches the latest release and whose
  `platforms.*.signature` is non-empty.
- **End-to-end:** install an older release, push a `fix:`/`feat:` commit so CI ships a
  higher version, then launch the old app → expect the *Install & Restart* dialog →
  the app relaunches on the new version. Exercise **Check for Updates…** on an
  up-to-date install to see the "latest version" dialog.
