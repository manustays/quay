# Quay

> **Quay** _(pronounced "key")_ — **Where your ports come in.**
>
> A native macOS menubar app to start, stop, and monitor your local dev services — Node/Python servers, Homebrew services, and long-running terminal agents — from one place.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Platform: macOS](https://img.shields.io/badge/platform-macOS-black.svg)](#requirements)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB.svg)](https://tauri.app)

---

## The problem

If you build a lot of local services, you know the dance: remember which folder, `cd` into it, run the start command, switch to the browser, and repeat for every project. Keeping several running at once means juggling terminal tabs and trying to remember what's up and on which port.

**Quay** puts all of that one click away. Register an app folder once; then start it, see its live status, open its web UI, or drop into a terminal in its folder — straight from the menubar. It also manages Homebrew services (MySQL, MongoDB, Redis…) and long-running terminal agents.

## What it looks like

Click the menubar icon and a popover opens:

```
┌────────────────────────────────────——─┐
│ 🔍 search…              [ ■ Stop all] │
├──────────────────────────────────——───┤
│ ★ FAVORITES                           │
│  ● myapp     :5173  [■][↗][>_]        │
│  ● claude    term   [■]    [>_]       │
├────────────────────────────────────——─┤
│  ▸ More (4)                           │
├────────────────────────────────────——─┤
│  [+ Add]                  [⚙ Settings]│
└───────────────────────────────────——──┘
```

Status at a glance: ● running (green) · ◐ starting (yellow) · ○ stopped (grey) · ✖ error (red).

> 📸 _Screenshots/GIF: add `docs/assets/popover.png` and a short capture once you run it locally (`npm run tauri dev`)._

## Features

- **One unified list** for three kinds of long-running things:
  - **Project servers** — Node/Python apps on `localhost:<port>` (`npm run dev`, `python main.py`, …)
  - **Homebrew services** — `brew services` formulae like `mysql`, `mongodb-community`, `redis`
  - **Terminal agents** — interactive tools you run in a terminal (e.g. Claude Code, custom agents)
- **Start / stop** each item from the menubar. Background services run headless (no foreground terminal); their output is logged to a file.
- **Live status** — process liveness **plus** a port/HTTP health check, polled in the background and pushed to the UI (no manual refresh).
- **Open in browser** — one click opens `http://localhost:<port>`.
- **Open a terminal** already `cd`'d into the service's folder, when you actually need to watch logs.
- **Auto-detect on add** — pick a folder and the app reads `package.json` / `requirements.txt` / `.env` to pre-fill the start command and port.
- **Favorites + search** — pin the services you use most; the rest tuck under a collapsible "More".
- **Per-item env vars, custom health path, and auto-start-on-launch.**
- **Configurable terminal** (Terminal.app or iTerm2) and **launch-at-login**.
- **Native & light** — built with Tauri v2 (Rust core + system webview), no bundled Chromium.

## Requirements

- **macOS** (Apple Silicon or Intel). This app is macOS-only — it uses `osascript`, `open`, and `brew`.
- For Homebrew items: [Homebrew](https://brew.sh) installed.
- To build from source: see [Development](docs/development.md).

## Install

There are no pre-built signed releases yet, so the supported path today is **build from source** (or build your own `.dmg`).

```bash
git clone https://github.com/manustays/quay.git
cd quay
npm install
npm run tauri build      # produces a .app and .dmg under src-tauri/target/release/bundle/
```

Open the generated `.dmg` and drag the app to `/Applications`, or run it in dev mode while you try it out:

```bash
npm run tauri dev
```

Full details, including the Rust/Node prerequisites and how to package, sign, and notarize a distributable build:

- **[Installation guide](docs/installation.md)**
- **[Packaging & distribution (macOS)](docs/packaging.md)**

## Usage

1. Click the menubar icon → **+ Add**.
2. **Pick a folder** (for a project or agent) — the app pre-fills name, start command, and port. Or choose **kind = brew** and pick a formula.
3. Tweak fields if needed (run mode, env vars, health path, favorite, auto-start) → **Save**.
4. Hit **▶** to start. Watch the dot go yellow → green. Use **↗** to open the browser, **>_** to open a terminal, **■** to stop.

See the **[Usage guide](docs/usage.md)** for the full walkthrough of item kinds, run modes, and status semantics.

## Documentation

| Doc | What's in it |
|-----|--------------|
| [Installation](docs/installation.md) | Prerequisites, build from source, install the `.app` |
| [Usage](docs/usage.md) | Adding items, run modes, status, browser/terminal actions, favorites |
| [Configuration](docs/configuration.md) | `config.json` location + full field reference |
| [Packaging & distribution](docs/packaging.md) | Build a `.dmg`, code-sign, and notarize for macOS |
| [Development](docs/development.md) | Dev setup, project layout, running tests |
| [Architecture](docs/architecture.md) | How the Rust core and webview fit together |
| [Troubleshooting](docs/troubleshooting.md) | Common issues and fixes |
| [Design spec](docs/specs/2026-06-26-menubar-service-manager-design.md) | The original design document |

## How it works (in one paragraph)

A Rust core owns all process supervision and state; a small vanilla-TypeScript webview is the popover UI. They talk over Tauri commands (UI → Rust) and events (Rust → UI). Background services are spawned as child processes in their own process group (so the whole tree can be stopped cleanly), with stdout/stderr written to a per-item log file. A background poll loop checks each item's process and port and pushes status changes to the UI. Everything dies with the app — quit from the tray's right-click **Quit** and owned children are terminated. See [Architecture](docs/architecture.md).

## Known limitations

- **macOS only.**
- **Services don't survive an app restart** by design — quitting the app stops everything it started; on relaunch all items show `stopped`.
- **Terminal-mode items are best-effort** — the app opens a Terminal/iTerm window but doesn't own that process; "stop" for those is best-effort, and a terminal item with a configured port can sit at `starting` if its window is closed externally.
- No pre-built signed release yet — build from source or roll your own `.dmg`.

See the design spec's non-goals for the full list.

## Roadmap

- Drag-to-reorder in the UI (the backend `reorder` command already exists)
- Aggregate tray-icon color reflecting overall state
- Richer error surfacing (exit code + log tail in tooltips)
- Pre-built, notarized releases

## Contributing

Contributions welcome — see **[CONTRIBUTING.md](CONTRIBUTING.md)**. In short: open an issue to discuss, work on a `feature/`, `bugfix/`, or `chore/` branch, use conventional commits, run the tests (`cargo test` + `npm test`) and `npx tsc --noEmit` before opening a PR.

## License

[MIT](LICENSE) © 2026 Kumar Abhishek
