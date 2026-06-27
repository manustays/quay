# Usage

This guide walks through the day-to-day use of Quay.

## The popover

Left-click the menubar icon to open the popover. It has:

- A **search box** and a **Stop all** button at the top.
- A **FAVORITES** section (items you've starred).
- A collapsible **More (n)** section for everything else.
- A footer with **+ Add** and **⚙ Settings**.

Right-click the menubar icon for the **Quit** menu. Quitting stops every background service the app started.

### Status indicators

Each row shows a colored dot:

| Dot | State | Meaning |
|-----|-------|---------|
| ● green | `running` | Process alive and (if it has a port) the port is reachable / health check passes |
| ◐ yellow | `starting` | Process alive but the port isn't accepting connections yet |
| ○ grey | `stopped` | Not running |
| ✖ red | `error` | Process exited unexpectedly, or a start/stop failed (hover the dot for the message) |

Status is refreshed automatically by a background poll (every few seconds, configurable). You never need to manually refresh.

Running rows also show live **CPU% · memory** next to the port — sampled only while the popover is open, every 10s by default. See [metrics](metrics.md).

### Row actions

| Button | Action | Shown when |
|--------|--------|------------|
| ▶ / ■ | Start / Stop | always |
| ↗ | Open `http://localhost:<port>` in your browser | the item has a port |
| >_ | Open a terminal `cd`'d into the item's folder | the item has a folder |

Click the **body** of a row to expand it — you'll see the tail of its log file, plus **Edit**, **Delete**, and toggles for **Favorite** and **Auto-start**.

## The three kinds of items

### 1. Project servers (`kind: project`)

Your Node/Python/etc. apps that serve on a localhost port.

- **dir** — the project folder.
- **startCmd** — e.g. `npm run dev`, `python main.py`, `pnpm start`.
- **port** — the port the dev server listens on (used for status + the browser button).
- **runMode** — usually `background` (headless; output goes to a log file).

### 2. Homebrew services (`kind: brew`)

Background services managed by `brew services`.

- **brewFormula** — e.g. `mysql`, `mongodb-community`, `redis`.
- No folder or start command needed.
- Start/stop call `brew services start|stop <formula>`; status comes from `brew services list`.

### 3. CLI tools (`kind: cli`)

Standalone command-line tools and binaries — interactive long-running tools run in a real terminal (e.g. Claude Code or a custom agent), or headless commands run in the background.

- **dir** + **startCmd** — e.g. `claude` in a project folder.
- **runMode** — `terminal` (opens a Terminal/iTerm window you can type into).

## Run modes

| Mode | Behavior | Use for |
|------|----------|---------|
| `background` | App spawns the command as a hidden child process; stdout/stderr go to a log file. The app owns and can stop it. | Servers, brew |
| `terminal` | App opens a Terminal/iTerm window running the command, so you can interact with it. The app does **not** own this process; stop is best-effort. | Interactive CLI tools |

## Adding an item

1. **+ Add**.
2. For a project or CLI tool: **Pick…** a folder. The app inspects it and pre-fills:
   - `package.json` with a `dev`/`start`/`serve` script → `npm run <script>` and a port from `.env` if present.
   - `requirements.txt` / `pyproject.toml` → `python main.py`.
3. For a brew service: set **kind = brew** and choose a formula (the field suggests installed formulae).
4. Adjust any field:
   - **Env** — one `KEY=VALUE` per line, merged into the process environment. _Use for dev variables only — don't store real secrets here; the config file is plain text._
   - **Health path** — optional. If set (e.g. `/health`), status uses an HTTP `GET` to `http://localhost:<port><healthPath>` and treats a 2xx as healthy. If empty, status uses a plain TCP port check.
   - **Favorite** — pin to the top section.
   - **Auto-start** — start this item automatically when the app launches.
5. **Save.**

## Editing & deleting

Expand a row → **Edit** to reopen the form pre-filled, or **Delete** (with confirmation) to remove it. Favorites and auto-start can also be toggled from the expanded row.

## Stop all

The header **Stop all** button (with confirmation) stops every running background service and every app-started brew service in one go. Terminal-mode items are best-effort.

## Settings

**⚙ Settings** lets you set:

- **Terminal app** — `Terminal` (default) or `iTerm`. Used for the "open terminal" action and `terminal`-mode items.
- **Poll interval (sec)** — how often status is checked (default 3).
- **Metrics interval (sec)** — how often per-process CPU%/memory are sampled while the popover is open (default 10). See [metrics](metrics.md).
- **Launch at login** — register/unregister the app as a macOS login item.

## Where things live

- **Config:** `~/Library/Application Support/am.abhi.quay/config.json`
- **Logs:** `~/Library/Application Support/am.abhi.quay/logs/<id>.log` (one per item)

See [Configuration](configuration.md) for the full file format.
