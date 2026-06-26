# Installation

Quay is **macOS only**. There are no pre-built signed releases yet, so you install it by building from source (or by building your own `.dmg` and installing that).

## Prerequisites

| Tool | Why | Install |
|------|-----|---------|
| **Rust** (stable) | Builds the app's Rust core | [rustup.rs](https://rustup.rs) — `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| **Node.js 18+** + npm | Builds the webview frontend | [nodejs.org](https://nodejs.org) or `brew install node` |
| **Xcode Command Line Tools** | Compiles native macOS code | `xcode-select --install` |
| **Homebrew** (optional) | Only needed for managing `brew services` items | [brew.sh](https://brew.sh) |

After installing Rust, confirm `cargo` is on your `PATH`:

```bash
cargo --version    # should print a version, not "command not found"
```

If it's missing, restart your shell or `source "$HOME/.cargo/env"`. See [Troubleshooting](troubleshooting.md#cargo-not-found) if it still isn't found.

## Option A — Build and run from source (quickest to try)

```bash
git clone https://github.com/manustays/quay.git
cd quay
npm install
npm run tauri dev
```

The first run compiles all Rust dependencies and may take a couple of minutes. When it's ready, a tray icon appears in your menubar — click it to open the popover.

`npm run tauri dev` runs an unoptimized debug build with hot-reload for the frontend. Use it while trying the app or developing.

## Option B — Build a release app and install it

```bash
git clone https://github.com/manustays/quay.git
cd quay
npm install
npm run tauri build
```

This produces:

- The app bundle: `src-tauri/target/release/bundle/macos/Quay.app`
- A disk image: `src-tauri/target/release/bundle/dmg/Quay_0.1.0_<arch>.dmg`

Install it by either:

- Opening the `.dmg` and dragging **Quay** to `/Applications`, or
- Copying the `.app` straight to `/Applications`:
  ```bash
  cp -R "src-tauri/target/release/bundle/macos/Quay.app" /Applications/
  ```

### "App can't be opened because it is from an unidentified developer"

A locally-built app is unsigned, so Gatekeeper will warn on first launch. To open it anyway:

- **Right-click** the app in `/Applications` → **Open** → **Open** in the dialog, **or**
- Run once: `xattr -dr com.apple.quarantine "/Applications/Quay.app"`

To produce a properly **signed and notarized** build that opens without warnings (recommended if you distribute it to others), see [Packaging & distribution](packaging.md).

## First launch

- The app lives in the **menubar**, not the Dock. Look for its tray icon at the top-right.
- Left-click the icon → popover. Right-click the icon → **Quit**.
- On first run there are no items — click **+ Add** to register your first service. See the [Usage guide](usage.md).
- Configuration and logs are written to
  `~/Library/Application Support/com.abhi.quay/`.

## Updating

Pull the latest source and rebuild:

```bash
git pull
npm install
npm run tauri build   # or: npm run tauri dev
```

Your `config.json` is stored outside the project directory, so it is preserved across rebuilds.

## Uninstalling

1. Quit the app (tray → **Quit**).
2. Delete the app: `rm -rf "/Applications/Quay.app"`.
3. (Optional) Remove its data:
   `rm -rf "~/Library/Application Support/com.abhi.quay"`.
4. (Optional) If you enabled **Launch at login**, disable it first in Settings, or remove the login item under **System Settings → General → Login Items**.
