# Menubar Service Manager — Design Spec

**Date:** 2026-06-26
**Status:** Approved design, ready for implementation planning
**Author:** abhi (manustays@gmail.com)

## Problem

I build many local services — Node.js / Python apps running on `localhost:<port>`. Keeping several running means remembering folder names, `cd`-ing in, starting the server, switching to a browser. I also run Homebrew background services (MySQL, MongoDB) and long-running terminal tools/agents (Claude Code, HERMES). I want one macOS menubar app to register these, start/stop them, see their status at a glance, open their web UI, and drop into a terminal in their folder only when needed.

## Goals

- Register application folders (and brew formulae, and terminal agents) as managed items.
- Start/stop each item from the menubar; track live status without a foreground terminal.
- One click to open a running service's web UI in the browser.
- One click to open a terminal already `cd`'d into the item's folder (for live logs / interaction).
- A single, efficient, native-feeling app — low RAM/CPU, small binary.

## Non-Goals (v1)

- Services surviving app restart (services are owned children; quitting the app stops them).
- Remote/SSH service management — local only.
- Secret management / encrypted credential storage.
- Cross-platform (macOS only).
- Auto-detecting/importing all projects on disk — items are added explicitly.

## Key Decisions (resolved during brainstorming)

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Tech stack | **Tauri** (Rust core + system-webview web UI) | Near-native efficiency, tiny binary (~3-10 MB), web frontend matches author's JS skills, hackable. |
| Process lifecycle | **Services die with the app** (owned child processes) | Simplest, cleanest. No orphan/PID-reconnect complexity. On relaunch all items show `stopped`. |
| Managed item model | **Unified** — one `ManagedItem` abstraction covers project servers, brew services, terminal agents | One data model; kind-specific behavior is thin. |
| Run mode | **Per-item**: `background` (headless child, logs to file) or `terminal` (opens Terminal/iTerm window) | Servers run headless; interactive agents need a real TTY. |
| Status detection | **Process + port check** (with optional per-item HTTP health path); brew via `brew services list` | "PID alive" alone is misleading for a still-starting server. |
| Add flow | **Auto-detect + confirm** — inspect folder, prefill form | Best UX, modest logic. |
| Terminal emulator | **Configurable, default Terminal.app**, support iTerm2 | Works out of the box; extensible. |
| Menubar UI | **Webview popover** anchored under the tray icon | Rich UI: status dots, inline buttons, search, favorites — beyond a native NSMenu. |

## Architecture

Tauri app. The **Rust core** owns process supervision and all state. The **web frontend** is the popover UI. They communicate via Tauri **commands** (frontend → Rust, request/response) and **events** (Rust → frontend, status/log push).

```
+---------------------------+        commands (invoke)        +--------------------------+
|   Frontend (webview)      | ------------------------------> |     Rust core            |
|   popover UI: list, form, | <------------------------------ |  supervisor, health,     |
|   settings, log tail      |     events (status/log push)    |  store, brew, terminal,  |
+---------------------------+                                 |  detect, commands        |
                                                              +--------------------------+
                                                                        |
                                          spawns / signals / osascript / brew / fs
                                                                        v
                              child processes · Terminal.app/iTerm · brew services · config & logs
```

### Rust core modules (each one responsibility)

| Module | Responsibility | Depends on |
|--------|----------------|------------|
| `store` | Load/save item configs + settings to `config.json` (atomic write: temp + rename). | serde, fs |
| `model` | Types: `ManagedItem`, `ItemKind`, `RunMode`, `Status`, `Settings`, `AppError`. | — |
| `supervisor` | Spawn/kill background child processes; hold PID + log handle per item; own process groups. | std::process, model |
| `brew` | Wrap `brew services start/stop/list`; parse list output to statuses. | model |
| `terminal` | Drive Terminal.app / iTerm2 via `osascript` (open folder, run terminal-mode item). | settings |
| `health` | Single poll loop: PID alive? port open / HTTP 200? brew status? → emit `status_changed` on change. | supervisor, brew |
| `detect` | Inspect a picked folder (package.json, pyproject/requirements, .env) → suggest cmd + port + kind. | fs |
| `commands` | Tauri command handlers wiring frontend → modules; all return `Result<_, AppError>`. | all |

### Frontend units

| Unit | Responsibility |
|------|----------------|
| `ItemList` | Render two-tier list (favorites up front, others under collapsible "More"); search filter; "Stop all". |
| `ItemRow` | One item: status dot + action buttons (start/stop, open-browser, open-terminal); expandable for log tail / edit / delete / favorite / autoStart. |
| `AddEditForm` | Folder/formula pick → detect prefill → edit all fields → save. Also used for editing existing items. |
| `Settings` | terminalApp, pollIntervalSec, browser, launch-at-login. |
| `ipc` | Thin wrapper over Tauri `invoke` / `listen`. |

### Data flow (start a service)

`invoke('start_item', id)` → `commands` → `supervisor` spawns child (`zsh -lc "<startCmd>"`, cwd=`dir`, env merged), redirects stdout+stderr to `logs/<id>.log` → `health` poll detects PID + port → emits `status_changed` event → frontend updates the row's dot. Frontend never polls; Rust pushes.

## Data Model & Persistence

**Config file:** `~/Library/Application Support/<app>/config.json`. Written by `store` on every change via atomic temp-write + rename. Loaded at launch.

```jsonc
{
  "settings": {
    "terminalApp": "Terminal",   // "Terminal" | "iTerm"
    "pollIntervalSec": 3,
    "browser": "default",        // open port in default browser
    "launchAtLogin": false
  },
  "items": [
    {
      "id": "uuid",              // stable; used for log filename + event routing + reorder-safety
      "name": "myapp",
      "kind": "project",         // "project" | "brew" | "agent"
      "dir": "/Users/abhi/dev/myapp",  // null for brew
      "startCmd": "npm run dev", // null for brew (brew uses brewFormula)
      "stopCmd": null,           // null = SIGTERM the owned child group; brew manages its own
      "port": 5173,              // null if none
      "runMode": "background",   // "background" | "terminal"
      "brewFormula": null,       // e.g. "mysql" when kind=brew
      "order": 0,                // menu sort position
      "favorite": false,         // favorites shown up front; others under "More"
      "env": { "NODE_ENV": "development" },  // merged onto shell env at spawn; {} if none
      "healthPath": null,        // null = TCP port check; e.g. "/health" = HTTP GET, 200 -> running
      "autoStart": false         // true = start this item when the app launches
    }
  ]
}
```

### Persistence rules

- **Runtime state is NOT persisted.** PID, current status, child handles, log paths live in memory only (matches "services die with app"). On launch every item = `stopped`.
- **No secrets in config.** `env` is plaintext, intended for dev variables only. The spec and the Add/Edit form both warn against storing real credentials. The app inherits the user's shell environment for anything sensitive.

### Kind-specific invariants

- `kind=project`: has `dir` + `startCmd` (+ usually `port`); `runMode=background`.
- `kind=brew`: `dir`/`port`/`startCmd` null, `brewFormula` set; status via `brew services list`; always `background`; ignores `healthPath`.
- `kind=agent`: has `dir` + `startCmd`; typically `runMode=terminal`; usually no `port`.

### In-memory runtime (separate from config)

```
RuntimeState: id -> { status, pid?, childHandle?, logPath, lastError? }
```

## Process Supervision & Status

### Spawn — background mode

- Run via login shell so PATH / nvm / pyenv resolve: `zsh -lc "<startCmd>"`, `cwd = dir`, env = shell ∪ `item.env`.
- Redirect stdout + stderr → append to `logs/<id>.log` with a session marker line per start.
- Store `Child` handle + PID in `RuntimeState`; status → `starting`.
- **Process group:** spawn in its own group (setsid-style) so stop kills the whole tree (e.g. `npm` → `node`). Stop = SIGTERM the group, escalate to SIGKILL after a 5s timeout.

### Spawn — terminal mode

- `terminal` builds `cd <dir> && <env exports> && <startCmd>` and hands it to Terminal.app / iTerm2 via `osascript`.
- The app does **not** own a child handle. Status: if `port` set, use the port check; otherwise best-effort (assume running until the window/session is gone).
- **Stop is best-effort** (accepted): signal the port-owning PID if discoverable, else the user closes the window. Documented as a known limitation.

### Brew mode

- Start/stop = `brew services start|stop <formula>`.
- Status from a cached `brew services list` parse each poll cycle.

### Health poll loop

Single timer, interval from settings (default 3s). For each item whose status != `stopped`:

```
background:
  PID alive? no  -> error (unexpected exit; capture tail of log)
            yes  -> port set? -> healthPath set? HTTP GET 200 -> running, else -> starting
                                  else TCP connect: open -> running, closed -> starting
                  -> no port  -> running (PID alive is enough)
brew:     parse `brew services list` -> started -> running, stopped -> stopped
terminal: port set? port check : assume running until window gone
emit status_changed(id, status) only when it differs from last (diffed)
```

- **Immediate exit detection:** also catch child exit as it happens (don't wait for the next poll) → `error` if unexpected, `stopped` if user-initiated.
- **Status states:** `stopped` · `starting` · `running` · `error`.

### On app quit

Exit handler SIGTERMs all owned background children. Terminal-mode and brew items are **not** owned and survive — this nuance is documented.

## UI & Interactions

**Tray icon:** glyph with a color cue reflecting the aggregate state (any `error` → red; any `running` → normal/active; all stopped → dim). Click opens the popover webview anchored under the icon.

**Popover — two-tier list:**

```
+-------------------------------------+
| (search)                [ Stop all ]|
+-------------------------------------+
| FAVORITES                           |
|  * myapp    :5173  [>/#][open][term]|
|  * claude   term   [>/#]      [term]|
+-------------------------------------+
|  More (4)            (collapsed)     |
+-------------------------------------+
| [+ Add]                  [Settings] |
+-------------------------------------+
```

- **Status dot:** running (green) · starting (yellow) · stopped (grey) · error (red).
- **Row buttons:** start/stop toggle · open-browser (shown if `port`; runs `open http://localhost:<port>`) · open-terminal (shown if `dir`). Brew rows show only start/stop.
- **Row expand** (click body): live log tail (last N lines via `log_appended` events) + edit + delete + toggle-favorite + toggle-autoStart.
- **Favorites** (`favorite=true`) shown up front; non-favorites under a collapsible **"More (n)"** section.
- **Search box** filters all items (favorites + others) by name / kind / port; matches appear regardless of the More-collapsed state. Empty search restores favorites + collapsed More.
- **Stop all** — header button (with running-count). Confirms, then stops every owned background child + app-started brew items; terminal-mode best-effort/skipped.
- **Drag to reorder** updates `order`, persisted.

**Add/Edit form:** folder picker → `detect` prefills name / startCmd / port / kind → user edits all fields (incl. env, healthPath, runMode, favorite, autoStart) → save. Editing an existing item opens the same form preloaded with its config. Brew item add = pick a formula instead of a folder.

**Settings:** terminalApp (Terminal/iTerm2), pollIntervalSec, browser, launch-at-login (macOS login item).

**Live updates:** frontend subscribes to `status_changed` + `log_appended`; no manual refresh.

### Tauri commands

`get_items` · `add_item` · `update_item` · `delete_item` · `reorder` · `start_item` · `stop_item` · `stop_all` · `open_browser` · `open_terminal` · `toggle_favorite` · `tail_log` · `detect_folder` · `get_settings` · `update_settings`.

Search and the favorites/More split are pure frontend (no Rust).

## Error Handling

| Failure | Detection | Response |
|---------|-----------|----------|
| Bad start command (spawn fails / exits fast) | spawn error, or PID dead < ~2s | row → error; tooltip = exit code + last log line; "View log" in expand |
| Server starts but port never opens | `starting` persists past ~30s | stays yellow; note "port :X not up — check log"; no auto-kill |
| Crash while running | child exit caught (not user-initiated) | → error; capture log tail; keep entry; one-click restart |
| Port already in use | port open but owned PID dead, or EADDRINUSE in log | error + hint "port :X already in use" |
| `brew` / `osascript` / shell missing | spawn error | error toast naming the missing tool |
| Brew formula not installed | `brew services` stderr | error on that row with stderr |
| Terminal app not found | osascript failure | toast: pick a different terminal in Settings |
| Config file corrupt/unreadable | parse failure at launch | load empty, back up bad file to `config.bad.json`, toast; never crash |
| Stop times out | still alive after 5s | escalate SIGKILL group; if still alive, error "couldn't stop, PID X" |
| Log file unwritable | fs error at spawn | start anyway, warn "logs unavailable" |

**Principles:** never crash the menubar app over a child failure — isolation is per item. Every error is per-row with the log reachable. Destructive actions (delete, stop all) confirm first. All Rust command results are `Result<_, AppError>`; the frontend surfaces errors inline / as a toast, never silently.

## Testing

**Rust unit (isolated per module):**

- `detect` — sample folders (package.json variants, pyproject, .env) → assert suggested cmd/port/kind. Pure, table-driven.
- `store` — config serialize/load round-trip; corrupt file → backup path. Temp dirs.
- `model` — defaults and kind/runMode invariants (brew has formula, project has port).
- `brew` — parse `brew services list` fixtures → statuses. Pure parser.
- `health` — PID×port → status as a pure function, table-driven; mocked probes.
- `supervisor` — spawn a trivial real command (`sleep`, `python -m http.server`) in a temp dir → assert PID alive, port opens, stop kills the group. Fast integration test.

**Frontend:** component tests for `ItemRow` rendering per status, search filter, favorites split, and form prefill from a `detect` payload (mocked ipc).

**Manual smoke checklist:** add a real Node app → start → green → open browser → open terminal → stop; brew MySQL start/stop; an agent in terminal mode; quit app kills children; corrupt-config recovery.

**TDD where it pays:** `detect`, `brew` parse, `health` state function, `store` round-trip — pure logic, written test-first. Supervisor and UI are thin; lighter coverage.

## Build Order (suggested)

1. Tauri scaffold + tray icon + empty popover; `model` + `store` (load/save config).
2. `supervisor` (background spawn/stop, log files) + `start_item`/`stop_item` + a hardcoded item.
3. `health` poll loop + `status_changed` events + status dots in the UI.
4. `AddEditForm` + `detect` (auto-detect + confirm) + add/edit/delete/reorder.
5. `open_browser` + `terminal` module (open-terminal, terminal run-mode) + Settings.
6. `brew` module + brew item kind.
7. Favorites + two-tier list + search + Stop all + autoStart + launch-at-login.
8. Error-handling polish + manual smoke pass.
