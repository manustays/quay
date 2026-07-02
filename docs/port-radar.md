# Port radar (Detected servers)

Quay discovers dev servers you started outside the app — a `vite` left running in a terminal, a forgotten `next dev` — and shows them in the popover's **DETECTED** section, where they can be adopted as managed services, killed, or ignored.

## How it works

The scan loop (`src-tauri/src/scanner.rs`) mirrors the metrics loop: it idles while the popover is hidden and runs one pass every **5 seconds** while it's open.

Each pass:

1. **List listeners** — one `lsof -iTCP -sTCP:LISTEN -P -n -a -u <uid> -Fpn` call returns `(port, pid)` for every TCP listener owned by **your user**. Restricting to the current uid cuts system noise and avoids the Full Disk Access prompt entirely (foreign-user processes couldn't be resolved or signalled anyway).
2. **Filter** — drops Quay's own pid, pids already tracked in the running map, ports < 1024, ports in `settings.ignoredPorts`, and a small denylist of well-known non-dev processes (`rapportd`, `ControlCenter`, `sharingd`, `Spotify`, `Dropbox`).
3. **Resolve new pids** — a targeted `sysinfo` refresh reads each new pid's **argv** and **cwd** (no subprocess, no Full Disk Access for same-uid processes). Resolutions are cached per pid, so a steady set of listeners costs one `lsof` per pass and nothing else.
4. **Identify** — display name is the cwd's folder name (falling back to the process name); the tech stack comes from `detect::stack_from_argv` (launcher fingerprints like `next dev`, `manage.py runserver`) or, failing that, `detect::stack_from_dir` manifest markers in the cwd.
5. **Emit** — the full snapshot is pushed on the `ports_discovered` event; the frontend replaces its list wholesale (same contract as `metrics_changed`).

## Actions

- **Adopt** — opens the add-service form prefilled with the listener's folder, command (manifest script when the folder is recognizable, shell-quoted argv otherwise), port, and stack. Saving creates a normal `project` item; pressing **Start** then attaches to the live listener via the existing `adopt_if_listening` path — the process is not restarted. Review the prefilled command before saving: argv is what the process was *started with*, which may not be the command you'd use to start it fresh.
- **Kill** — SIGTERM (⌥-click: SIGKILL) the pid. The backend re-checks that the pid still owns the port immediately before signalling, so a stale row can't hit a reused pid. Only the pid is signalled, never its process group.
- **Ignore** — appends the port to `settings.ignoredPorts` (persisted). Ignored ports show as removable chips in Settings.

## Port collisions

Listeners on a port that belongs to a **registered item** are not shown under DETECTED — instead the item's row gets an amber ⚠ badge (when the item is stopped) with the holder's name and pid in the tooltip. Starting the item adopts the listener via `adopt_if_listening` as before.

## Limits

- **Docker published ports** surface as Docker Desktop's proxy (`com.docker*`/`vpnkit`), tagged with the docker icon and not adoptable — manage containers with a `docker`-kind item instead. Mapping a host port back to a specific container/compose project is out of scope for now.
- Same-uid processes only; TCP LISTEN only.
- A hardened/odd binary whose cwd can't be read falls back to its process name, and Adopt opens with the folder empty for you to pick.
