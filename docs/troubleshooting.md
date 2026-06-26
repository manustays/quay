# Troubleshooting

Common issues and how to fix them. Per-item logs are your best friend — find them at:

```
~/Library/Application Support/com.abhi.menubar-service-manager/logs/<id>.log
```

## `cargo` not found

`npm run tauri dev` fails with something like:

```
failed to run `cargo metadata` ... No such file or directory (os error 2)
```

`cargo`/`rustc` aren't on your shell's `PATH`. Fixes:

- Standard rustup installs put them in `~/.cargo/bin`. Make sure that's on `PATH`:
  ```bash
  source "$HOME/.cargo/env"     # for the current shell
  ```
  and add it permanently to your `~/.zshrc`:
  ```bash
  export PATH="$HOME/.cargo/bin:$PATH"
  ```
- If you use a non-standard toolchain manager, find the toolchain's `bin` directory and prepend it to `PATH` the same way. Verify with `which cargo && cargo --version`.

## "App can't be opened — unidentified developer"

A locally-built app isn't signed, so Gatekeeper blocks the first launch. Either:

- **Right-click** the app → **Open** → **Open**, or
- Clear the quarantine flag:
  ```bash
  xattr -dr com.apple.quarantine "/Applications/Menubar Service Manager.app"
  ```

For a build that opens with no warning, sign and notarize it — see [Packaging](packaging.md).

## I can't find the app / how do I quit it?

- The app lives in the **menubar** (top-right), not the Dock. Look for its tray icon.
- **Left-click** the icon for the popover; **right-click** for the **Quit** menu.

## A service shows `starting` (yellow) and never turns green

`starting` means the process is alive but the port isn't accepting connections yet. If it stays yellow:

- The server may still be compiling/booting — give it a moment.
- The configured **port** may be wrong. Edit the item and fix it.
- The server may have failed after launch — expand the row and read the log tail, or open the full log file.
- If the service has no web port at all, leave **port** empty; it will show `running` on process liveness alone.

## A service shows `error` (red)

The process exited unexpectedly, or a start/stop failed. **Hover the red dot** for the message, and expand the row to read the log tail. Common causes:

- Bad start command — test it manually in a terminal in that folder.
- **Port already in use** — another process holds the port (`lsof -i :<port>`).
- Wrong working directory or missing dependencies (`npm install` not run, wrong Node version, etc.).

## "Open in browser" does nothing

The browser button only appears when the item has a **port**, and it opens `http://localhost:<port>`. If the page doesn't load, the service isn't actually serving on that port yet (check status / logs).

## "Open terminal" opens the wrong app

The terminal action uses the app set in **Settings → Terminal app** (`Terminal` or `iTerm`). Switch it there. If nothing opens, make sure the chosen app is installed.

## Picking a folder hides the popover

Opening the native folder picker makes the popover lose focus. The app suppresses its hide-on-blur while the picker is open, so this should not happen — but if your form ever disappears, just click the menubar icon again; the form is preserved.

## Homebrew items don't work

- Ensure [Homebrew](https://brew.sh) is installed and `brew` is on your `PATH` (`which brew`).
- The formula name must match what `brew services list` shows (e.g. `mongodb-community`, not `mongodb`).
- Some services need `sudo` or extra setup the first time — start them once manually with `brew services start <formula>` to confirm they work, then manage them from the app.

## Background services keep running after I expected them to stop

Services only stop when **you** stop them (per-item, **Stop all**, or **Quit**). If you Force-Quit the app or the machine sleeps/logs out abruptly, the graceful cleanup may not run and a background child can be left running. Find and stop strays with:

```bash
pgrep -fl "<part of your start command>"
kill <pid>
```

Terminal-mode and brew items are intentionally not stopped on quit (they're not owned by the app).

## Launch-at-login didn't take effect

Toggle it in **Settings**. You can verify/remove it under **System Settings → General → Login Items**.

## Resetting everything

Quit the app, then remove its data directory and relaunch with a clean slate:

```bash
rm -rf "~/Library/Application Support/com.abhi.menubar-service-manager"
```

If only the config is bad, the app already auto-recovers: a corrupt `config.json` is moved to `config.bad.json` on launch and replaced with defaults.

## `npm run tauri build` fails at `bundle_dmg.sh`

The app compiles and the `.app` bundles, but the `.dmg` step fails with `failed to run bundle_dmg.sh`. The DMG bundler uses an AppleScript to make **Finder** style the disk-image window, and macOS blocks that until you grant **Automation** consent (the first attempt errors with `-1743`).

Fix: re-run and approve the “control Finder / System Events” prompt, or pre-allow your terminal under **System Settings → Privacy & Security → Automation → Finder**, then re-run. The `.app` was already built under `src-tauri/target/release/bundle/macos/`. Full detail and the CI/headless workaround are in [Packaging → Troubleshooting the DMG build](packaging.md#troubleshooting-the-dmg-build).

## Still stuck?

Open an issue at <https://github.com/manustays/menubar-cli-launcher/issues> with your macOS version + chip, what you did, and the relevant log output.
