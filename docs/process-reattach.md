# Reattaching to already-running services

## Problem

The app tracks a background service's liveness through an in-memory
`std::process::Child` handle in `AppState.running`. If the app process itself goes
away — a crash, a force-quit, or a relaunch after the laptop sleeps — that map is
rebuilt empty while the real `node`/`python` dev servers keep running, reparented to
`launchd` and still holding their ports.

Before this feature the consequences were:

- The poll loop saw no handle and reported the service **Stopped**, even though it was
  serving traffic.
- Pressing **Start** spawned a *second* process that couldn't bind the taken port,
  leaving two processes behind.
- Pressing **Stop** killed only the tracked child, so the orphan survived and there was
  no in-app way to kill it — you had to `lsof`/`kill` from a terminal.

## Model: owned vs adopted processes

`supervisor::Running` now holds `child: Option<Child>`:

- **Owned** (`Some`) — we spawned it via `zsh -lc` as its own `setsid` process-group
  leader. Liveness via `Child::try_wait`; stop signals the whole **process group**
  (`kill(-pgid, …)`) so grandchildren die too.
- **Adopted** (`None`) — reattached to a process we did *not* spawn. Liveness via
  `kill(pid, 0)`; stop signals the **PID only** (`kill(pid, …)`), never its group —
  we didn't create that group and could otherwise hit unrelated processes.

`Running::is_owned()` distinguishes the two.

## How reattachment happens

The configured **port** is the source of truth for "is this service actually up".

1. **Persisted PIDs** — `store::{load_pids,save_pids}` keep a volatile `id → pid` map in
   `pids.json` (separate from `config.json`). `commands::persist_pids` rewrites it
   whenever the `running` map changes.
2. **On launch** (`lib.rs` setup), two passes:
   - *Persisted-PID reattach* — for each PID in `pids.json`, adopt it only if alive
     **and**, when a port is configured, that PID appears in
     `supervisor::pids_listening(port)`. Tying identity to the port guards against PID
     reuse pointing us at an unrelated process. Dead/mismatched entries are dropped.
   - *Port sweep* — for each **background** item not already reattached above, probe
     its configured port (`commands::adopt_if_listening`) and adopt a live listener.
     This catches services started outside the app, or still running after a clean
     quit cleared `pids.json`, so they show Running on launch with no manual Start.
     Each port is claimed once, so two items sharing a port can't both adopt the same
     listener; terminal/brew items are excluded (they must not enter the running map).
3. **On Start** (`start_item`, background) — if the port is already serving
   (`http_ok` when a health path is set, else `port_open`), adopt the listener instead
   of spawning a duplicate, and mark it Running.
4. **On Stop** (`stop_item`) — stop the tracked child (if any), then if the port is
   still open call `supervisor::stop_port(port)`, which SIGTERM→SIGKILLs every listener
   PID on that port. This is the guaranteed "free the port" path for orphans.
5. **On graceful quit/exit** — children are stopped and `pids.json` is cleared, so the
   next launch doesn't try to reattach to processes we just killed.

The poll loop deliberately does **not** auto-adopt arbitrary open ports — that would
risk hijacking an unrelated process on a common port like `:3000`/`:8000`. Adoption
only happens at launch (persisted-PID reattach + a one-time port sweep) or on an
explicit Start. A service started *after* launch needs a Start or an app restart to be
adopted.

## Detecting listeners

`supervisor::pids_listening(port)` runs `lsof -ti tcp:<port> -sTCP:LISTEN` and parses
the output with the pure `parse_lsof_pids` (sorted + de-duplicated, tolerant of IPv4 +
IPv6 multiple listeners). If `lsof` is missing it returns empty and the app degrades to
handle-only behavior.

## Caveats

- `stop_port` kills whatever listens on the configured port. That matches intent on an
  explicit Stop, but a mistyped port could target an unrelated process.
- Reattachment relies on the configured port. A portless background item can only be
  reattached by raw PID liveness on launch (best effort), not adopted on Start.
