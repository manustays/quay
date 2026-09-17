# Per-process metrics (CPU% + memory)

Each running service shows live **CPU%** and **memory** next to its port/descriptor in
the popover. Metrics are sampled cheaply: only while the popover is open, on a
configurable interval (default 10s).

## Why visibility-gated

The status poll (`health`) runs continuously in the background because status drives the
UI dots and reattachment. Metrics are different — they're only worth anything while the
user is looking at the list. Sampling them continuously would burn CPU (and run `lsof`)
every few seconds forever, for data nobody sees.

So the metrics loop is gated on `AppState.visible`:

- `lib.rs::toggle_popover` sets `visible = true` after a successful `show()`, `false` after
  a successful `hide()`.
- The hide-on-blur handler sets `visible = false` **only when the window is actually
  hidden** — during a native dialog (`suppress_hide`) the popover stays open, so sampling
  must continue.
- The flag mirrors the *actual* outcome of each window op, never just the intended branch.

While hidden the loop **blocks on the visibility condvar** — no idle tick, no wakeups — and
is released the moment the popover opens. It also re-checks `visible` after a collection
(which takes ~200 ms + `lsof`) and skips the emit if the popover closed meanwhile. The
interval between samples is a condvar wait, not a sleep, so a hide (or a hide→show) that
happens *during* a collection is acted on at once instead of being slept through.

The two process-table refreshes ask for cpu+memory only. `ProcessesToUpdate::All` is still
required — the descendant walk needs every process's parent — but the default refresh kind
would also read cmd, environ, cwd, disk and user for every process on the machine, twice.

## How a sample is taken

For each item whose status is `running`/`starting`, `metrics::collect`:

1. **Resolves root PIDs.** The tracked child PID (background items), **plus** any port
   listeners via `supervisor::pids_listening(port)`. The union covers terminal/brew items
   (no tracked PID) and background servers whose launcher shell was reparented away. Each
   port is resolved at most once per pass (cached).
2. **Samples with `sysinfo`.** Two `refresh_processes` calls 200 ms apart
   (`MINIMUM_CPU_UPDATE_INTERVAL`) so `cpu_usage()` has a valid delta to measure; memory is
   read as resident bytes.
3. **Aggregates the process tree.** `aggregate_tree` (pure, unit-tested) walks each root's
   descendants via parent links, deduping shared PIDs, summing CPU% and memory. This is why
   an `npm → node → worker` service reports the real total, not the near-zero wrapper.

The result — one `ItemMetrics { id, cpuPercent, memoryBytes }` per item with live PIDs — is
emitted as a **full snapshot** on the `metrics_changed` event. The frontend rebuilds its
metrics map wholesale per event, so a service that stopped between ticks drops out and no
stale numbers linger.

## Notes & limits

- **`cpuPercent` is summed across cores** — it can exceed 100% for a busy multi-threaded
  tree (same convention as Activity Monitor's per-process "%CPU").
- **Portless brew services show nothing** — there's no tracked PID and no port to resolve
  them by. (A brew service with a configured port does get metrics.)
- **Cost is bounded to "while open."** Closed popover = no `sysinfo`, no `lsof`.

## Configuration

`metricsIntervalSec` (default `10`, min `1`) controls the sampling cadence — set via
**Settings → Metrics interval (sec)**. Read fresh each tick, so changes take effect on the
next cycle. See [configuration](configuration.md).
