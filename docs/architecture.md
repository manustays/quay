# Architecture

A contributor-facing overview of how Quay is built.

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
| `model` | Shared types: `ManagedItem`, `Settings`, `AppConfig`, `ItemKind`, `RunMode`, `Status`, `ItemStatus`, `AppError`. The serde representations are the contract with the frontend. `ItemKind::Cli` carries `#[serde(alias = "agent")]` so configs written before the `agent`→`cli` rename still deserialize (and re-serialize back to the frontend as the canonical `"cli"`). |
| `store` | Load/save `config.json` with an atomic temp-write + rename, and corrupt-file recovery (`config.bad.json` + defaults). Also persists the volatile `id → pid` map in `pids.json` for reattachment after an app restart. |
| `detect` | Inspect a chosen folder (`package.json`, `requirements.txt`/`pyproject.toml`, `.env`) and suggest a name, start command, port, and kind. |
| `brew` | Wrap `brew services start/stop/list` and parse the list output into per-formula statuses. |
| `docker` | Wrap the Docker CLI for container items: daemon lifecycle (`daemon_running`/`start_daemon`/`wait_for_daemon`), image listing for autocomplete (`list_images`), run/reuse a named container (`docker_start`) and stop it (`docker_stop`), container status via `docker ps` (`docker_status`/`parse_docker_ps`), and resource stats via `docker stats` (`stats_raw`/`parse_docker_stats`). See [Docker services](docker-services.md). |
| `supervisor` | Spawn a background item via `zsh -lc "<cmd>"` in its **own process group** (`setsid`), redirect stdout/stderr to `logs/<id>.log`, and stop it by signalling the group (SIGTERM, escalating to SIGKILL). Also **adopts** orphaned services across app restarts: `adopt` (handle-less, PID-only), `pids_listening`/`parse_lsof_pids` (find listeners via `lsof`), and `stop_port` (free a port by killing its listeners). See [process reattachment](process-reattach.md). |
| `health` | The pure `decide_status` function (PID liveness × port/HTTP reachability → status), the TCP/HTTP probes, and the background poll loop that emits `status_changed`. |
| `metrics` | Per-process CPU%/memory sampling via `sysinfo`. A gated loop (`AppState::wait_active`) samples only while the popover is open and a screen is lit, aggregates each item's whole process tree (pure `aggregate_tree`), and emits `metrics_changed`. See [metrics](metrics.md). |
| `terminal` | Build the shell line and drive Terminal.app / iTerm2 via `osascript` (open a folder, or run a `terminal`-mode item). |
| `state` | `AppState` — shared mutable state behind `Mutex`es: the loaded config, the map of running children, the status map, and the error map, plus a `suppress_hide` flag and the data dir. Also `Wake`, the condvar gate all three background loops park on (`awake` = a screen is lit and unlocked; `active` = that plus the popover open). |
| `mac_power` | macOS only. The app's one observer of system state: `NSWorkspace` screen sleep/wake and `NSDistributedNotificationCenter` lock/unlock, plus the startup reads (`CGDisplayIsAsleep`, `CGSessionCopyCurrentDictionary`) that notifications alone can't supply. Drives `Wake`. See *Power gating*. |
| `commands` | All `#[tauri::command]` handlers, plus `init_state`. |
| `lib` | The Tauri builder: registers plugins, sets up the tray + popover + hide-on-blur, manages `AppState`, registers commands, spawns the poll loop and the power observers, auto-starts flagged items, and installs the quit/exit handler. |

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
   - **docker** → ensure the daemon is up (`docker::start_daemon`/`wait_for_daemon` if needed), then `docker::docker_start` runs or reuses the named container; status `starting`.
   - **background** → `supervisor::spawn_background` spawns the child (own process group, logs to file), inserts it into the running map, status `starting`.
   - **terminal** → `terminal::run_in_terminal` opens a Terminal/iTerm window; status `running`.
3. The poll loop picks it up on the next cycle and emits `status_changed` as the real state settles (e.g. `starting` → `running` once the port opens).

### Status polling

A single background thread runs every `pollIntervalSec`. This is the app's only
always-on loop — it keeps running with the popover closed, because it drives the tray
icon and the waiting-agent badge — so its per-pass cost is the app's energy floor.

That floor is **conditional on someone being able to see the tray**. The loop parks on
the same condvar the other two use (`AppState::wait_awake`) whenever every display is
asleep or the Mac is locked, because the icon and badge it maintains are on no screen
in either state. See *Power gating* below.
`brew services list` and `docker ps -a` are therefore spawned **once per pass**, not
once per item, and parsed into a map the item loop looks up (the same batching
`metrics::collect` does for `launchctl`/`lsof`). Each pass:

- For each non-stopped item, it computes status:
  - **background:** is the child PID alive? then a port/HTTP check → `decide_status`. A dead PID records an error and yields `error`.
  - **brew:** parse `brew services list`.
  - **docker:** `docker ps` for the named container (`docker::docker_status`), plus a port/HTTP check if the item has a port.
  - **terminal:** a port check if the item has a port.
- It calls `set_status`, which emits `status_changed` **only when the status actually changed** (so the UI isn't spammed).

The poll loop deliberately releases the `running` lock before doing the (blocking) port/HTTP probe, so a slow probe never stalls command handlers.

### Metrics sampling

A second background thread (`metrics::spawn_metrics_loop`) samples per-process CPU% and memory, but **only while the popover is visible** — gated on `AppState::wait_active` — the popover open **and** a screen to show it on. `lib.rs` flips visibility on successful window show/hide (and on genuine hide-on-blur, but not while a native dialog suppresses hiding). While hidden it blocks on a condvar rather than idle-ticking, so a closed popover costs zero wakeups and an open is picked up immediately. The awake term matters independently: display sleep does not defocus a window, so without it a popover left open would keep sampling at a dark screen. The interval between samples is a condvar wait too, carrying the visibility *generation* the pass started with — a hide→show that happens during a collection would otherwise notify a condvar nobody was waiting on and be slept through.

Each pass resolves root PIDs per running item (the tracked child PID, plus any port listeners via `pids_listening` — covering terminal/brew items and reparented servers), takes two `sysinfo` refreshes 200 ms apart so CPU% is a valid delta (cpu+memory only — the tree walk needs every process's parent, but not its argv, environ, cwd or disk I/O), then sums each item's whole process tree with the pure `aggregate_tree` helper. Docker items are the exception: their CPU/memory come from `docker stats` (`docker::collect_docker`) rather than the host process tree, since the container runs under the Docker VM. The result is pushed as a full snapshot via `metrics_changed`; the frontend replaces its map wholesale so stopped items drop out. See [metrics](metrics.md).

### Popover placement

### Power gating

`mac_power` (macOS only) is the app's only observer of system state. It watches four
notifications and ands them into `Wake`:

| Centre | Notification | Flag |
|---|---|---|
| `NSWorkspace` notification centre | `NSWorkspaceScreensDidSleep` / `…DidWake` | `screens_asleep` |
| `NSDistributedNotificationCenter` | `com.apple.screenIsLocked` / `…IsUnlocked` | `locked` |

`Wake::awake()` is `!screens_asleep && !locked`; `Wake::active()` adds `visible`. The
health loop waits on the first, metrics and radar on the second.

Three details that are easy to get wrong:

- **`ScreensDidSleep` fires only when *every* attached display sleeps**, which is the
  question being asked. A closed lid with an external display still lit does not fire
  it — correctly, since the menubar is visible over there.
- **Notifications only report changes**, so the current state has to be read once at
  startup (`CGDisplayIsAsleep`, `CGSessionCopyCurrentDictionary`). Without that, a
  launch or an updater relaunch into a locked Mac would poll at a lock screen forever.
  `show_popover` re-reads them too, so a missed notification can't wedge the app idle.
- **Gating the next iteration does not cancel a pass already in flight.** What this
  buys is eventual quiescence, not an instant stop.

System sleep needs no observer: the CPU is stopped and the threads are parked anyway,
and `ScreensDidWake` covers the resume.

Set `QUAY_TRACE=1` to print each transition — the gate parks the app precisely when
nobody is looking, so there is otherwise no way to watch it work.

On macOS the popover is positioned entirely in **AppKit coordinates** (`lib.rs`: `pin_under_tray`, `mac_screen_geometry`). Tauri's physical monitor origins/sizes (and the positioner plugin's `TrayCenter`, which builds on them) are inconsistent on mixed-DPI desktops — vertically stacked 1x + 2x displays in particular — and mixing them with AppKit window coordinates picked the wrong display or stranded the window off-screen (issue #5; upstream tauri-apps/tauri#7890, plugins-workspace#724).

1. **Anchor.** On a left-click, the tray handler records `NSEvent::mouseLocation()` in `TrayAnchorState`. The cursor is over the status item at that moment, so this is the icon's position in the same space as `NSScreen`/`NSWindow`.
2. **Screen.** `mac_screen_geometry` finds the `NSScreen` whose `frame` contains the anchor via the pure `cocoa_frame_contains` (unit-tested). AppKit is y-up and the cursor's top pixel row reports `y == maxY`, so y is matched on `(origin.y, maxY]` and x on `[origin.x, maxX)`. A half-open y range would miss clicks with the cursor pushed against the top edge — the usual way to hit the menubar — and on stacked displays would match the screen above.
3. **Place.** The window's top-left is set with `setFrameTopLeftPoint`: x centered on the anchor and clamped inside the screen's `visibleFrame`, top edge at the top of `visibleFrame` (just under the menubar). The chosen screen's origin is stored in `PopoverMonitorState`.
4. **Toggle vs move.** `toggle_popover` on a visible popover compares the clicked screen with `PopoverMonitorState`: same screen → hide; different screen → re-pin there and refocus instead of closing.
5. **Resize.** `resize_popover` (driven by the frontend's content height) re-pins from the last anchor, keeping the top edge fixed so the window grows downward.

If there is no anchor yet, or no screen contains it, or the call isn't on the main thread (`MainThreadMarker`), placement is skipped rather than guessed. Non-macOS builds keep the positioner's `TrayCenter`, guarded by `current_monitor()` so an off-screen window can't panic it.

Known limit: the frontend's height cap (`POPOVER_MAX` in `Popup.tsx`) is read once from `window.screen.availHeight`, so after moving to a shorter display a very tall popover can extend past that display's bottom edge.

### Tray menu attachment (macOS 27 workaround)

macOS 27 stopped forwarding status-item mouse events to the `NSView` that `tray-icon`
installs on the `NSStatusBarButton` whenever an `NSMenu` is attached to the `NSStatusItem`:
AppKit's menu tracking swallows the click instead. The view's `mouseDown:`/`mouseUp:` never
fire, so no `TrayIconEvent::Click` is emitted, so `on_tray_icon_event` never runs — the
popover becomes unreachable and *every* click, left or right, just opens the context menu.
`show_menu_on_left_click(false)` cannot help: the suppression it relies on lives in the
callback AppKit no longer invokes.

So on macOS the builder deliberately does **not** call `.menu(&tray_menu)`. Instead
`show_tray_menu` (`lib.rs`) attaches the menu, presents it, and detaches it again:

```
set_menu(Some(menu)) → with_inner_tray_icon(|t| t.show_menu()) → set_menu(None)
```

Two things make this safe, and both are worth knowing before touching it:

- **It must stay synchronous.** Tray events arrive on the main thread, and Tauri's
  main-thread dispatch (`run_item_main_thread!` → `send_user_message`) calls straight
  through when it is already on that thread. `show_menu` is an `NSStatusBarButton
  ::performClick`, which runs AppKit's nested menu-tracking loop and returns only once the
  menu is dismissed — which is exactly when the detach should happen. Moving this onto a
  worker thread would widen the window in which the menu sits attached and clicks are dead.
- **It is bound to right-button *Down*, not Up**, matching how every other macOS menubar
  item opens its menu and preserving press-drag-release selection.

#### The click also races hide-on-blur

Detaching the menu exposes a second macOS 27 change: the status-item button now takes key
focus on mouse-down, so the popover resigns key and `WindowEvent::Focused(false)` hides it
**before** the click is delivered. Measured on this machine, the blur handler runs ~80 ms
ahead of `TrayIconEvent::Click`. A toggle that only asks `win.is_visible()` therefore sees
a hidden window and reopens it — the click flickers instead of closing.

So the blur handler records *when* it hid the popover (`note_blur_hide`), and the tray
handler latches the verdict on the click's **press** (`press_closes_popover`) while that
timestamp is still fresh, within `BLUR_CLICK_GRACE` (250 ms). The **release** consumes the
latch and passes it to `toggle_popover` as `already_closed`, which then treats the popover
as open despite the hidden window. Latching on the press rather than testing the clock at
release is what makes a press held longer than the grace window behave the same as a quick
one. The cross-display branch still wins over the close, so clicking the icon on another
display moves the popover there instead of swallowing the click.

This mirrors the upstream fix, [tauri-apps/tray-icon#365](https://github.com/tauri-apps/tray-icon/pull/365),
which landed in tray-icon 0.25.1. Tauri 2.11 still requires `tray-icon ^0.24`, so the fix
is unreachable from here — 0.25.1 is semver-incompatible, and `[patch.crates-io]` cannot
bridge that. **Removal condition:** once Tauri depends on `tray-icon >= 0.25.1`, delete
`show_tray_menu` and its event arm and restore the plain `.menu(&tray_menu)` on the
builder. Non-macOS builds already take that path and are unaffected.

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
