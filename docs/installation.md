# Installation

Quay is **macOS only**. The easiest path is to **download the latest release** (Option A below). You can also build from source (Options B and C). Releases are universal — one `.dmg` runs on Apple Silicon and Intel — but are **not yet code-signed/notarized**, so first launch needs one extra click (see [Opening an unsigned build](#opening-an-unsigned-build)).

## Option A — Download a release (easiest)

1. Go to the **[latest release](https://github.com/manustays/quay/releases/latest)** and download `Quay_<version>_universal.dmg`.
2. Open the `.dmg` and drag **Quay** to `/Applications`.
3. Launch it. Because the build is unsigned, macOS will block it the first time — follow [Opening an unsigned build](#opening-an-unsigned-build) to allow it. You only do this once.

## Prerequisites

| Tool | Why | Install |
|------|-----|---------|
| **Rust** (stable) | Builds the app's Rust core | [rustup.rs](https://rustup.rs) — `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| **Node.js 18+** + npm | Builds the webview frontend | [nodejs.org](https://nodejs.org) or `brew install node` |
| **Xcode Command Line Tools** | Compiles native macOS code | `xcode-select --install` |
| **Homebrew** (optional) | Only needed for managing `brew services` items | [brew.sh](https://brew.sh) |
| **Docker Desktop** (optional) | Only needed for managing Docker container items | [docker.com](https://www.docker.com/products/docker-desktop/) |

After installing Rust, confirm `cargo` is on your `PATH`:

```bash
cargo --version    # should print a version, not "command not found"
```

If it's missing, restart your shell or `source "$HOME/.cargo/env"`. See [Troubleshooting](troubleshooting.md#cargo-not-found) if it still isn't found.

## Option B — Build and run from source (quickest to try)

```bash
git clone https://github.com/manustays/quay.git
cd quay
npm install
npm run tauri dev
```

The first run compiles all Rust dependencies and may take a couple of minutes. When it's ready, a tray icon appears in your menubar — click it to open the popover.

`npm run tauri dev` runs an unoptimized debug build with hot-reload for the frontend. Use it while trying the app or developing.

## Option C — Build a release app and install it

```bash
git clone https://github.com/manustays/quay.git
cd quay
npm install
npm run tauri build
```

This produces:

- The app bundle: `src-tauri/target/release/bundle/macos/Quay.app`
- A disk image: `src-tauri/target/release/bundle/dmg/Quay_0.5.0_<arch>.dmg`

Install it by either:

- Opening the `.dmg` and dragging **Quay** to `/Applications`, or
- Copying the `.app` straight to `/Applications`:
  ```bash
  cp -R "src-tauri/target/release/bundle/macos/Quay.app" /Applications/
  ```

### Opening an unsigned build

Downloaded releases and locally-built apps are **unsigned**, so Gatekeeper warns on first launch — *"Quay can't be opened because it is from an unidentified developer."* This is expected, not a sign of anything wrong. Open it once with either method (you only do this the first time):

**Recommended — "Open Anyway":**

1. Double-click Quay; the warning appears. Dismiss it.
2. Open **System Settings → Privacy & Security**, scroll to the **Security** section, and click **Open Anyway** next to the message about Quay.
3. Confirm, and authenticate if prompted. Quay launches and is trusted from then on.

> On **macOS Sequoia (15) and later**, the older right-click → **Open** shortcut no longer reliably bypasses Gatekeeper for unsigned apps — use **Open Anyway** above.

**Alternative — strip the quarantine flag from a terminal:**

```bash
xattr -dr com.apple.quarantine "/Applications/Quay.app"
```

To produce a properly **signed and notarized** build that opens with no warning at all (recommended if you distribute it to others), see [Packaging & distribution](packaging.md). The release CI signs + notarizes automatically once the Apple credentials are configured as repository secrets.

## First launch

- The app lives in the **menubar**, not the Dock. Look for its tray icon at the top-right.
- Left-click the icon → popover. Right-click the icon → **Quit**.
- On first run there are no items — click **+ Add** to register your first service. See the [Usage guide](usage.md).
- Configuration and logs are written to
  `~/Library/Application Support/am.abhi.quay/`.

## Updating

If you installed from a release, download the newer `.dmg` from the [latest release](https://github.com/manustays/quay/releases/latest) and drag it over the old app in `/Applications`. Your `config.json` lives outside the app, so it's preserved.

If you built from source, pull the latest and rebuild:

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
   `rm -rf "~/Library/Application Support/am.abhi.quay"`.
4. (Optional) If you enabled **Launch at login**, disable it first in Settings, or remove the login item under **System Settings → General → Login Items**.
