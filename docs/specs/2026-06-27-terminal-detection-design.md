# Multi-terminal detection & launch

Date: 2026-06-27
Status: Approved (design)

## Context

Quay's "Open terminal" / Terminal run-mode only supports **Terminal.app** and **iTerm**,
hardcoded in `SettingsDialog.tsx` and dispatched via AppleScript in `terminal.rs`. Users on
modern GPU terminals (Ghostty, Kitty, WezTerm, Alacritty) can't pick their terminal. Goal:
**detect** the terminals actually installed on the machine and launch the selected one
correctly. AppleScript `do script` / `write text` only works for Terminal.app and iTerm; the
others need their own CLI invocation.

Outcome: settings dropdown lists only installed terminals; running a service in / opening a
folder in any of them works.

## Supported terminals

| Display    | Strategy    | Detect (`.app` bundle)        | Detect (CLI fallback) |
|------------|-------------|-------------------------------|-----------------------|
| Terminal   | AppleScript | always present (system)       | —                     |
| iTerm      | AppleScript | `iTerm.app`                   | —                     |
| Ghostty    | CLI         | `Ghostty.app`                 | `ghostty`             |
| Kitty      | CLI         | `kitty.app`                   | `kitty`               |
| WezTerm    | CLI         | `WezTerm.app`                 | `wezterm`             |
| Alacritty  | CLI         | `Alacritty.app`               | `alacritty`           |

## Components

### 1. Terminal registry (`src-tauri/src/terminal.rs`)

A static table (pure fn, unit-testable) describing each terminal: stable `id`/display name,
`.app` bundle name(s), optional CLI binary name, and launch strategy enum
(`AppleScript { kind }` vs `Cli`). Single source of truth for both detection and dispatch.

### 2. Detection

- `fn installed_terminals() -> Vec<String>` (display names), driven by the registry:
  - **`.app` bundle:** check `/Applications/<App>` and `~/Applications/<App>`.
  - **CLI fallback:** `command -v <bin>` via login shell (`/bin/zsh -lc`), same pattern as
    `brew::login_shell_brew` / `docker::login_shell_docker`. Covers brew-installed CLIs with
    no `.app` (e.g. `wezterm` headless).
  - Terminal.app is always included.
- New Tauri command `get_terminals() -> Vec<String>` in `commands.rs`, registered in
  `lib.rs` `invoke_handler`, exposed in `ipc.ts` as `getTerminals()`.

### 3. Launch (`run_in_terminal`)

Reuse existing `build_command_line(dir, env, cmd)` (cd + env exports + cmd). Then dispatch by
the registry strategy for `app_name`:

- **AppleScript** — unchanged: Terminal.app `do script`, iTerm `write text`.
- **CLI** — `open -na "<App>" --args <argv…>` where the terminal runs an interactive login
  shell that executes the line and then stays open:
  - inner command: `/bin/zsh -ilc "<line>; exec /bin/zsh -il"`
    (`exec` keeps the window alive after the command, matching `do script`; interactive login
    shell sources `~/.zshrc`, so PATH resolves natively — no `interactive_path` needed here).
  - per-terminal argv prefix before the shell command:
    - Ghostty: `-e /bin/zsh -ilc "…"`
    - Kitty: `/bin/zsh -ilc "…"` (kitty runs the given program)
    - WezTerm: `start -- /bin/zsh -ilc "…"`
    - Alacritty: `-e /bin/zsh -ilc "…"`
  - If the `.app` is absent but the CLI is on PATH, invoke the resolved binary directly
    instead of `open -na`.
- Unknown / uninstalled `app_name` → `AppError`.

### 4. Frontend (`src/components/SettingsDialog.tsx`)

On dialog open, call `getTerminals()`; render the returned names as `<SelectItem>`s instead of
the two hardcoded entries. If the saved `terminalApp` is no longer installed, still show it
(so the user sees their stored choice) but it errors on launch.

## Error handling

- Detection failures (missing dirs, login-shell probe error) degrade gracefully: that terminal
  is simply omitted; Terminal.app remains.
- Launch errors return `AppError` with the underlying stderr (as today).

## Testing

- Rust unit tests: registry lookup by id; `build_command_line` unchanged; CLI argv builder
  produces expected tokens per terminal (pure fn, no process spawn).
- Manual (real device, by user): each installed terminal opens, runs a service, stays open;
  `get_terminals` reflects actually-installed apps.

## Caveat

Exact CLI arg contracts for Ghostty/Kitty/WezTerm/Alacritty (esp. Ghostty `-e`) can't be
verified in this environment — require real-device verification.

## Out of scope

- Warp (no scriptable launch-with-command CLI), tmux, SSH/remote terminals.
- Per-service terminal override (uses the global `settings.terminalApp`).
