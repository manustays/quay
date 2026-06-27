# Docker service management

Quay manages Docker containers alongside Project / Brew / CLI services. A Docker
service is the fourth `ItemKind` and — like Brew — bypasses `RunMode`, routing to
a dedicated module instead of the background/terminal supervisor.

## Model

`ManagedItem` (Rust `src-tauri/src/model.rs`, TS `src/model.ts`) gains two
`Option`/nullable fields, both `#[serde(default)]` so pre-existing configs load:

| Field | Role |
|-------|------|
| `dockerImage` | "repo:tag" — **autofill only**, not operational |
| `containerName` | **the join key** for status, stop, and metrics — required |
| `startCmd` (existing) | full `docker run …` command — source of truth for a fresh run |

Containers run inside the Docker Desktop VM, so there is **no host PID** to
supervise. Docker items never enter `AppState.running`: status comes from
`docker ps`, metrics from `docker stats`. This mirrors Brew (tracked via
`launchctl`, not the running map).

## Backend — `src-tauri/src/docker.rs`

Parallel to `brew.rs`. The `docker` binary is resolved to an absolute path
(`/usr/local/bin/docker`, `/opt/homebrew/bin/docker`,
`/Applications/Docker.app/Contents/Resources/bin/docker`, then a login-shell
lookup) and cached in a `OnceLock`, so a bundled `.app` launched from Finder
(minimal PATH) still finds it.

- **Daemon lifecycle**: `daemon_running()` (`docker info` exit status),
  `start_daemon()` (`open -a Docker`), `wait_for_daemon(timeout)` (polls ~1s).
- **Images**: `list_images()` / `parse_docker_images` — `docker images`,
  deduped, `<none>` dropped.
- **Status**: `docker_status(name)` / `parse_docker_ps` — `docker ps -a`.
  `running`→Running, `restarting`/`created`→Starting, `exited`/`paused`→Stopped,
  `dead`/`removing`/unknown→**Error** (unknown states surface, never silently Stopped).
- **Lifecycle**: `docker_start(item)` decides via the pure `plan_start`:
  - container exists (stopped *or* running) → **reuse** `docker start <name>`,
    preserving volumes/data;
  - absent → run the configured `start_cmd` **through a login shell**
    (`zsh -lc`) so quoting in `-e "A=B C"` is honoured and `docker` is on PATH.
  - `docker_stop(name)` → `docker stop <name>`.
- **Metrics**: `stats_raw()` feeds `metrics::parse_docker_stats` /
  `parse_mem_size`.

## Backend wiring

- `commands.rs`: `list_docker_images`, `docker_daemon_running`,
  `start_docker_daemon` (launch + wait 60s). `start_item`/`stop_item` Docker
  branches; `start_item` returns the `DOCKER_DAEMON_DOWN` sentinel when the daemon
  is down so callers without UI (auto-start) fail safe. `stop_all` includes Docker
  items **only when currently Running/Starting** — never tears down a container it
  did not bring up.
- `health.rs`: Docker polled even when Stopped (state lives in `docker ps`).
- `metrics.rs`: Docker items excluded from the host-PID tree; `collect_docker`
  runs a single `docker stats --no-stream` (only when ≥1 Docker item is active and
  the popover is visible), matched to items by `containerName`, merged into the
  emitted `ItemMetrics` vec.
- `lib.rs`: module + commands registered; Docker excluded from the launch-time
  port-sweep; auto-start brings Docker Desktop up once if a Docker item is flagged
  `autoStart` and the daemon is down.

## Frontend

- `ServiceForm.tsx`: "Docker container" kind. Image input with a `<datalist>`
  of installed images; picking one autofills name / container name / a
  `docker run --name <cn> -d <img>` start command (port `-p` left for the user).
  Container name required on save. Adding while the daemon is down prompts to
  start Docker. Run mode / folder hidden for Docker.
- `ServiceRow.tsx`: `descriptor` shows `docker`; the Start action ensures the
  daemon (prompt-then-launch) before starting, with the sentinel as a fallback.
  CPU·MEM row is unchanged (works for any running item with metrics).
- `src/lib/docker.ts`: `ensureDockerDaemon()` — shared prompt-then-auto-start
  helper, suppressing hide-on-blur around the native confirm.

## Design decisions

- **Daemon down → prompt, then auto-start** (`open -a Docker`, wait, proceed).
- **Existing stopped container → reuse** via `docker start` (preserves data);
  fresh `docker run` only when the container is absent.

## Tests

Pure functions are unit-tested per the existing `#[cfg(test)]` convention:
`docker.rs` (`parse_docker_images`, `parse_docker_ps`, `plan_start`,
`candidate_order`, `is_executable`), `metrics.rs` (`parse_docker_stats`,
`parse_mem_size`), `model.rs` (Docker serialization + back-compat deserialization).

## Verify

- `cargo test --manifest-path src-tauri/Cargo.toml docker` / `metrics` / `model`.
- Manual (Docker Desktop installed): add a Docker service → pick image → autofill
  → save (daemon-down prompt) → start (Starting→Running, `docker ps` shows it) →
  open popover (CPU·MEM) → stop → start again (reuses container, data preserved).
