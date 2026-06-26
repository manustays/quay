# Architecture

A contributor-facing overview of how Menubar Service Manager is built. For the original decisions and rationale, see the [design spec](specs/2026-06-26-menubar-service-manager-design.md).

## Big picture

```
+---------------------------+      commands (invoke)       +--------------------------+
|   Frontend (webview)      | ---------------------------> |       Rust core          |
|   vanilla TS popover UI   | <--------------------------- |  supervisor, health,     |
|   list / row / form /     |     events (status push)     |  store, brew, terminal,  |
|   settings                |                              |  detect, state, commands |
+---------------------------+                              +--------------------------+
                                                                      |
                                   spawn / signal · osascript · brew · fs · TCP/HTTP
                                                                      v
                       child processes · Terminal.app/iTerm · brew services · config & logs
```

It's a **Tauri v2** app. The **Rust core** owns all process supervision and state; the **frontend** is a small vanilla-TypeScript single-page app rendered in a frameless, always-on-top webview window anchored under the menubar tray icon. The two communicate only through Tauri's IPC:

- **Commands** — frontend calls Rust (`invoke('start_item', { id })`), request/response.
- **Events** — Rust pushes to the frontend (`status_changed`), so the UI never polls.

## Rust modules

Each module has a single responsibility:

| Module | Responsibility |
|--------|----------------|
| `model` | Shared types: `ManagedItem`, `Settings`, `AppConfig`, `ItemKind`, `RunMode`, `Status`, `ItemStatus`, `AppError`. The serde representations are the contract with the frontend. |
| `store` | Load/save `config.json` with an atomic temp-write + rename, and corrupt-file recovery (`config.bad.json` + defaults). |
| `detect` | Inspect a chosen folder (`package.json`, `requirements.txt`/`pyproject.toml`, `.env`) and suggest a name, start command, port, and kind. |
| `brew` | Wrap `brew services start/stop/list` and parse the list output into per-formula statuses. |
| `supervisor` | Spawn a background item via `zsh -lc "<cmd>"` in its **own process group** (`setsid`), redirect stdout/stderr to `logs/<id>.log`, and stop it by signalling the group (SIGTERM, escalating to SIGKILL). |
| `health` | The pure `decide_status` function (PID liveness × port/HTTP reachability → status), the TCP/HTTP probes, and the background poll loop that emits `status_changed`. |
| `terminal` | Build the shell line and drive Terminal.app / iTerm2 via `osascript` (open a folder, or run a `terminal`-mode item). |
| `state` | `AppState` — shared mutable state behind `Mutex`es: the loaded config, the map of running children, the status map, and the error map, plus a `suppress_hide` flag and the data dir. |
| `commands` | All `#[tauri::command]` handlers, plus `init_state`. |
| `lib` | The Tauri builder: registers plugins, sets up the tray + popover + hide-on-blur, manages `AppState`, registers commands, spawns the poll loop, auto-starts flagged items, and installs the quit/exit handler. |

## Frontend units

| File | Responsibility |
|------|----------------|
| `ipc.ts` | Typed wrappers over `invoke`/`listen`. The single place IPC names live on the frontend. |
| `model.ts` | TypeScript mirrors of the Rust types (camelCase) + pure helpers (`statusDot`, `matchesSearch`, `splitFavorites`), unit-tested. |
| `list.ts` | Renders the two-tier list (favorites + collapsible "More"), the search box, and Stop-all. |
| `row.ts` | Renders one item: status dot, action buttons, and the expand panel (log tail, edit, delete, favorite, auto-start). |
| `form.ts` | The add/edit modal: folder picker → detect prefill → all fields → save. |
| `settings.ts` | The settings modal + launch-at-login toggle. |
| `main.ts` | Bootstraps: loads items, renders, subscribes to `status_changed`, and re-renders on updates. |

## Key flows

### Starting a service

1. UI calls `invoke('start_item', { id })`.
2. `commands::start_item` dispatches by kind:
   - **brew** → `brew services start <formula>`, status set to `running`.
   - **background** → `supervisor::spawn_background` spawns the child (own process group, logs to file), inserts it into the running map, status `starting`.
   - **terminal** → `terminal::run_in_terminal` opens a Terminal/iTerm window; status `running`.
3. The poll loop picks it up on the next cycle and emits `status_changed` as the real state settles (e.g. `starting` → `running` once the port opens).

### Status polling

A single background thread runs every `pollIntervalSec`:

- For each non-stopped item, it computes status:
  - **background:** is the child PID alive? then a port/HTTP check → `decide_status`. A dead PID records an error and yields `error`.
  - **brew:** parse `brew services list`.
  - **terminal:** a port check if the item has a port.
- It calls `set_status`, which emits `status_changed` **only when the status actually changed** (so the UI isn't spammed).

The poll loop deliberately releases the `running` lock before doing the (blocking) port/HTTP probe, so a slow probe never stalls command handlers.

### Shutdown

Quitting via the tray's **Quit** menu item drains the running map and stops each owned child **before** exiting (a `RunEvent::ExitRequested` handler is also installed as a backstop). Background children are in their own session, so they don't get a stray SIGHUP — explicit cleanup is what guarantees "services die with the app". Terminal-mode and brew items are intentionally not owned and are left running.

## Concurrency & locking

- `AppState` holds several `Mutex`es (config, running, statuses, errors). No code path holds two of them at once, so there is no lock-order inversion.
- Mutating command handlers take the `config` lock in a scoped block and drop it **before** calling `persist` (which re-locks `config`) — important because `std::sync::Mutex` is not reentrant.
- No mutex is held across blocking I/O (the port/HTTP probe, or `child.wait` during stop).

## Security posture

- `osascript` command lines are built with correct shell single-quote escaping and AppleScript string escaping, so real folders/commands with spaces or quotes work. These run **your own** configured commands, not untrusted input.
- The frontend sets all process/user-supplied text via `textContent`, never `innerHTML` interpolation.
- Tauri capabilities (`src-tauri/capabilities/default.json`) grant only `dialog:allow-open` and the three `autostart` permissions beyond `core:default`.
- `config.json`'s `env` map is plain text and meant for dev variables, not secrets.

## Deferred / not implemented

Honest gaps (see the spec's non-goals and the README's limitations):

- Drag-to-reorder in the UI (the `reorder` command exists; the gesture isn't wired).
- Aggregate tray-icon color tint.
- Streaming logs (the UI tails the log file on expand instead).
- Richer error surfacing beyond the dot tooltip + log tail.
