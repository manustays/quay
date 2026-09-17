# Configuration

All state is stored in a single JSON file. You normally edit it through the app's UI, but the format is documented here for reference and manual tweaking.

## Location

```
~/Library/Application Support/am.abhi.quay/config.json
```

Per-item logs live alongside it:

```
~/Library/Application Support/am.abhi.quay/logs/<id>.log
```

The file is written atomically (temp file + rename) on every change. If it ever becomes unreadable or corrupt, the app backs it up to `config.bad.json` and starts fresh with defaults — so a bad edit won't crash the app.

> **Edit while the app is not running.** The app loads the config at launch and overwrites the file on changes, so hand-edits made while it's running may be lost.

## Top-level shape

```jsonc
{
  "settings": { /* app-wide settings */ },
  "items":    [ /* registered services */ ]
}
```

## `settings`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `terminalApp` | `"Terminal"` \| `"iTerm"` | `"Terminal"` | Which terminal emulator the "open terminal" action and `terminal`-mode items use. |
| `pollIntervalSec` | number | `3` | How often (seconds) the background loop re-checks each item's status. Minimum 1. |
| `metricsIntervalSec` | number | `10` | How often (seconds) per-process CPU%/memory are sampled **while the popover is open**. No sampling happens while it's closed. Minimum 1. See [metrics](metrics.md). |
| `browser` | string | `"default"` | Reserved; the browser action currently always uses the system default browser. |
| `launchAtLogin` | boolean | `false` | Whether the app is registered as a macOS login item. Toggle via Settings (it also calls the OS API). |
| `ignoredPorts` | number[] | `[]` | Ports hidden from the popover's DETECTED (port radar) section. Add via a detected row's **Ignore** action; remove via the chips in Settings. |
| `ignoredAgents` | `{agent, cwd}[]` | `[]` | Agent sessions hidden from the AGENTS section. Ignoring hides **all** sessions of that agent in that cwd. See [agent radar](agent-radar.md). |
| `radarDevOnly` | boolean | `true` | When true, DETECTED hides listeners with no recognized dev stack (databases, caches, system services). |
| `trackAgents` | boolean | `true` | Master switch for the [agent radar](agent-radar.md). When false the scan pass is skipped entirely, the menubar waiting signal stays clear, and the AGENTS section disappears. Installed hooks stay installed but are ignored. |
| `waitingTitleBadge` | boolean | `false` | When true, the menubar title shows the count of agents waiting on you (e.g. `●2`) beside the tray icon. Requires `trackAgents`. |

## `items[]`

Each registered service:

```jsonc
{
  "id": "9f1c…",              // stable UUID; also the log filename
  "name": "myapp",
  "kind": "project",          // "project" | "brew" | "cli" | "docker" | "command" ("agent" still accepted as a legacy alias for "cli")
  "dir": "/Users/me/dev/myapp", // null for brew items
  "startCmd": "npm run dev",   // null for brew items
  "stopCmd": null,             // command kind: shell command to stop the service; else null (SIGTERM the owned child group)
  "port": 5173,                // null if the service has no port
  "runMode": "background",     // "background" | "terminal"
  "brewFormula": null,         // e.g. "mysql" when kind = "brew"
  "dockerImage": null,         // e.g. "postgres:16" when kind = "docker"
  "containerName": null,       // container name when kind = "docker"
  "stack": "vite",             // detected tech stack (row icon); null if unknown
  "group": null,               // optional group label; grouped items cluster + start/stop together
  "order": 0,                  // sort position in the menu
  "favorite": false,           // shown in the FAVORITES section when true
  "env": { "NODE_ENV": "development" }, // merged into the process env at spawn
  "healthPath": null,          // null = TCP port check; e.g. "/health" = HTTP 2xx check
  "browserUrl": null,          // null = http://localhost:<port>; e.g. "http://127.0.0.1:{port}/index.html"
  "autoStart": false           // start automatically when the app launches
}
```

### Field reference

| Field | Type | Notes |
|-------|------|-------|
| `id` | string | UUID v4. Assigned automatically; don't reuse. Names the item's log file (`logs/<id>.log`). |
| `name` | string | Display name. |
| `kind` | `"project"` \| `"brew"` \| `"cli"` \| `"docker"` \| `"command"` | Determines how the item is started and how status is read. `"agent"` is accepted as a legacy alias for `"cli"`. |
| `dir` | string \| null | Working directory. Required for `project`/`cli`; optional for `command` (defaults to `$HOME`); `null` for `brew`. |
| `startCmd` | string \| null | Shell command run via `zsh -lc "<cmd>"` (so your PATH / nvm / pyenv resolve). `null` for `brew`. Required for `command`. |
| `stopCmd` | string \| null | For `command` items, the shell command that stops the service (e.g. `omlx stop`), run via `zsh -lc`. For other kinds it is unused — `null` means a background item is stopped by signalling its process group. |
| `port` | number \| null | TCP port. Enables the browser button and the port-based status check. |
| `runMode` | `"background"` \| `"terminal"` | `background` = headless child + log file; `terminal` = opens a Terminal/iTerm window. Brew items are always treated as background. |
| `brewFormula` | string \| null | Homebrew formula name when `kind = "brew"`. |
| `dockerImage` | string \| null | Docker image to run when `kind = "docker"`, e.g. `postgres:16`. |
| `containerName` | string \| null | Container name when `kind = "docker"` (required for docker items). Quay reuses an existing container with this name, or runs a new one. |
| `stack` | string \| null | Detected tech stack keyword (e.g. `"vite"`, `"django"`), set at add/adopt time from the folder's manifests; drives the row's brand icon. `null` renders no icon. |
| `group` | string \| null | Optional group label. Items sharing a label cluster under a group header with aggregate status and start-all/stop-all. Trimmed on save; empty = ungrouped. |
| `order` | number | Menu sort order. |
| `favorite` | boolean | Pin to the FAVORITES section. |
| `env` | object (string→string) | Extra environment variables merged onto your shell env at spawn. **Plain text — dev variables only, not secrets.** |
| `healthPath` | string \| null | If set, status uses an HTTP `GET http://localhost:<port><healthPath>` and treats a 2xx as healthy (requires a `port`). If `null`, a plain TCP connect is used. Ignored for brew items. |
| `browserUrl` | string \| null | URL opened by ↗ **Open in browser** and copied by the `:port` label. `{port}` is replaced with `port` (e.g. `http://127.0.0.1:{port}`, `http://192.168.1.42:{port}`, `http://localhost:{port}/index.html`). Must be `http://` or `https://` — anything else is rejected on save. `null` = `http://localhost:<port>`. A fixed URL without `{port}` also works for port-less items. |
| `autoStart` | boolean | Start this item when the app launches (respecting its run mode). |

## Status model (for reference)

Status is computed, not stored. The four states are:

- `stopped` — no running process.
- `starting` — process alive, port not yet accepting connections.
- `running` — process alive and (if it has a port) reachable / health check passes; for brew, `brew services list` reports it started.
- `error` — process exited unexpectedly, or a start/stop operation failed.

Runtime state (PIDs, current status, log handles) is **not** persisted — it lives in memory only. On relaunch every item starts as `stopped`.

## Command services (`kind: "command"`)

For a **detached daemon controlled by its own CLI** — a launchd service, or a tool with `start`/`stop` subcommands (e.g. oMLX on `:9000`, Hermes) — that Quay should *manage* but not *own*:

- **Start** runs `startCmd` (e.g. `omlx start`) via `zsh -lc` and waits for it to return. A nonzero exit surfaces as an error; the daemon it launched keeps running detached.
- **Stop** runs `stopCmd` (e.g. `omlx stop`). Required to stop from Quay — there is no owned process to signal and no port-kill fallback (the CLI is the source of truth).
- **Status** is driven by the configured **`port`** (required), polled even when stopped — so starting or stopping the service *outside* Quay is reflected within one poll. Set `healthPath` to switch the probe from a TCP connect to an HTTP 2xx check (note: an admin root that returns 401/redirects will then read as stopped — leave it unset to use a plain port check).
- Quay never owns the process, so (like `brew`/`terminal` items) it is **left running when Quay quits**.

The ↗ **Open in browser** action (opens `browserUrl`, default `http://localhost:<port>`) appears once the service is running, same as any ported item.

```jsonc
{
  "id": "44444444-4444-4444-8444-444444444444",
  "name": "oMLX",
  "kind": "command",
  "dir": null,
  "startCmd": "omlx start",
  "stopCmd": "omlx stop",
  "port": 9000,
  "runMode": "background",   // ignored for command items
  "order": 3,
  "favorite": false,
  "env": {},
  "healthPath": null,
  "autoStart": false
}
```

## Example

```jsonc
{
  "settings": {
    "terminalApp": "iTerm",
    "pollIntervalSec": 2,
    "metricsIntervalSec": 10,
    "browser": "default",
    "launchAtLogin": true
  },
  "items": [
    {
      "id": "11111111-1111-4111-8111-111111111111",
      "name": "web",
      "kind": "project",
      "dir": "/Users/me/dev/web",
      "startCmd": "npm run dev",
      "stopCmd": null,
      "port": 5173,
      "runMode": "background",
      "brewFormula": null,
      "order": 0,
      "favorite": true,
      "env": { "NODE_ENV": "development" },
      "healthPath": null,
      "autoStart": true
    },
    {
      "id": "22222222-2222-4222-8222-222222222222",
      "name": "mysql",
      "kind": "brew",
      "dir": null,
      "startCmd": null,
      "stopCmd": null,
      "port": null,
      "runMode": "background",
      "brewFormula": "mysql",
      "order": 1,
      "favorite": false,
      "env": {},
      "healthPath": null,
      "autoStart": false
    },
    {
      "id": "33333333-3333-4333-8333-333333333333",
      "name": "postgres",
      "kind": "docker",
      "dir": null,
      "startCmd": null,
      "stopCmd": null,
      "port": 5432,
      "runMode": "background",
      "brewFormula": null,
      "dockerImage": "postgres:16",
      "containerName": "quay-postgres",
      "order": 2,
      "favorite": false,
      "env": { "POSTGRES_PASSWORD": "dev" },
      "healthPath": null,
      "autoStart": false
    }
  ]
}
```
