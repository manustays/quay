# Configuration

All state is stored in a single JSON file. You normally edit it through the app's UI, but the format is documented here for reference and manual tweaking.

## Location

```
~/Library/Application Support/com.abhi.menubar-service-manager/config.json
```

Per-item logs live alongside it:

```
~/Library/Application Support/com.abhi.menubar-service-manager/logs/<id>.log
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

## `items[]`

Each registered service:

```jsonc
{
  "id": "9f1c…",              // stable UUID; also the log filename
  "name": "myapp",
  "kind": "project",          // "project" | "brew" | "agent"
  "dir": "/Users/me/dev/myapp", // null for brew items
  "startCmd": "npm run dev",   // null for brew items
  "stopCmd": null,             // null = SIGTERM the owned child group; brew manages its own
  "port": 5173,                // null if the service has no port
  "runMode": "background",     // "background" | "terminal"
  "brewFormula": null,         // e.g. "mysql" when kind = "brew"
  "order": 0,                  // sort position in the menu
  "favorite": false,           // shown in the FAVORITES section when true
  "env": { "NODE_ENV": "development" }, // merged into the process env at spawn
  "healthPath": null,          // null = TCP port check; e.g. "/health" = HTTP 2xx check
  "autoStart": false           // start automatically when the app launches
}
```

### Field reference

| Field | Type | Notes |
|-------|------|-------|
| `id` | string | UUID v4. Assigned automatically; don't reuse. Names the item's log file (`logs/<id>.log`). |
| `name` | string | Display name. |
| `kind` | `"project"` \| `"brew"` \| `"agent"` | Determines how the item is started and how status is read. |
| `dir` | string \| null | Working directory. Required for `project`/`agent`; `null` for `brew`. |
| `startCmd` | string \| null | Shell command run via `zsh -lc "<cmd>"` (so your PATH / nvm / pyenv resolve). `null` for `brew`. |
| `stopCmd` | string \| null | Reserved. `null` means a background item is stopped by signalling its process group. |
| `port` | number \| null | TCP port. Enables the browser button and the port-based status check. |
| `runMode` | `"background"` \| `"terminal"` | `background` = headless child + log file; `terminal` = opens a Terminal/iTerm window. Brew items are always treated as background. |
| `brewFormula` | string \| null | Homebrew formula name when `kind = "brew"`. |
| `order` | number | Menu sort order. |
| `favorite` | boolean | Pin to the FAVORITES section. |
| `env` | object (string→string) | Extra environment variables merged onto your shell env at spawn. **Plain text — dev variables only, not secrets.** |
| `healthPath` | string \| null | If set, status uses an HTTP `GET http://localhost:<port><healthPath>` and treats a 2xx as healthy (requires a `port`). If `null`, a plain TCP connect is used. Ignored for brew items. |
| `autoStart` | boolean | Start this item when the app launches (respecting its run mode). |

## Status model (for reference)

Status is computed, not stored. The four states are:

- `stopped` — no running process.
- `starting` — process alive, port not yet accepting connections.
- `running` — process alive and (if it has a port) reachable / health check passes; for brew, `brew services list` reports it started.
- `error` — process exited unexpectedly, or a start/stop operation failed.

Runtime state (PIDs, current status, log handles) is **not** persisted — it lives in memory only. On relaunch every item starts as `stopped`.

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
    }
  ]
}
```
