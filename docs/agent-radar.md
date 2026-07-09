# Agent Radar

Quay auto-discovers **interactive terminal AI-agent sessions** — Claude Code
(`claude`), Codex CLI (`codex`), OpenCode (`opencode`), and Pi (`pi`) — and
shows them in an **Agents** section of the popover (between Favorites and
More). Each session shows: activity dot, agent brand icon, project name,
CPU % / memory / uptime, and the session's working directory. Two or more
sessions in the same folder club into one **project folder row** with a stack
of overlapping agent badges on the right — which agents are in there, dimmed
when idle — that expands into the member rows.

## How detection works

- **Interactive session = agent process + attached tty.** Each scan pass runs
  one `ps -axo pid=,ppid=,tty=,comm=`; only PIDs with a `ttys…` terminal are considered
  (sysinfo does not expose the controlling tty on macOS). This naturally
  excludes the Claude Code daemon, its `--bg-pty-host`/`--bg-spare` helpers,
  and the Claude desktop app — argv-based excludes back this up. Sessions
  inside tmux/screen still get a tty and are detected. This is "detected
  terminal sessions", not every conceivable agent runtime.
- **Identity** is the argv[0] basename (`claude`, `codex`, `opencode`, `pi`),
  with position-specific exclusions (`claude daemon …`, `codex mcp-server`,
  …). `pi` is extra-guarded because the name is collision-prone.
- **Project name & stack** come from the session's cwd, exactly like the port
  radar: manifest name (`package.json` / `Cargo.toml`) with the folder
  basename as fallback, plus the detected stack icon (Vite, Rails, …).
- The scan shares the port radar's loop: every 5 s **while the popover is
  open**, nothing while it's hidden.

## Session names (hover)

Hovering a session row's name (or a folder row's agent badge) shows a
best-effort session label:

- **Claude Code**: the first user prompt of the newest session log in
  `~/.claude/projects/<cwd-slug>/`.
- **Codex**: the `thread_name` indexed in `~/.codex/session_index.jsonl` for
  the newest rollout file whose recorded cwd matches.
- **Pi / OpenCode**: none — nothing name-like is stored on disk.

Labels are **cwd-keyed**: two sessions in the same folder show the same
(newest) label.

## What "active" means

The dot is a **recent-activity signal, not proof of work**:

- **Claude Code**: newest `.jsonl` mtime in the session's
  `~/.claude/projects/<cwd-slug>/` directory < 20 s ago, **or** the process is
  using > 10 % CPU.
- **Codex**: same rule via the newest matching
  `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` (selected by mtime across all
  date dirs, so long-running sessions started days ago still match).
- **Pi**: same idea via `~/.pi/agent/sessions/<cwd-slug>/` (slug format
  inferred from one machine — best effort).
- **OpenCode**: CPU heuristic only (sessions live in sqlite).

Anything else shows **idle** — which includes "waiting at a permission
prompt", *unless* the waiting-state hooks are installed (below).

## "Waiting on you" (Claude Code, via hooks)

`ps` can't tell "blocked at a permission prompt" from plain idle — both are
low CPU and no log writes. With the optional hook helper installed, Claude
Code itself reports its state and the dot gains a third color: **pulsing
amber = waiting on you**. In a clubbed folder row, one waiting member turns
the whole folder's pill amber.

How it works: `quay-hook` (bundled with the repo, `src-tauri/src/bin/`) is
invoked by Claude Code on lifecycle events and writes one small JSON file per
session under `~/Library/Application Support/am.abhi.quay/agent-state/`. The
radar's 5 s poll reads them. Event → state mapping:

| Hook event | State written |
|---|---|
| `UserPromptSubmit`, `PostToolUse` | working |
| `Notification` (permission needed / waiting for input) | waiting |
| `Stop` | idle |
| `SessionEnd` | file deleted |

The radar only *trusts* the hook's **waiting** — working/idle still come from
the mtime/CPU heuristic. A session-log write newer than the waiting event
overrides it (the session resumed), and files for dead sessions are pruned
after a 10-minute grace. Corrupt/partial files are dropped on read; writes
are atomic (temp + rename), so a mid-write poll can't read half a file.

### Manual install

1. Build and place the helper somewhere stable:

   ```sh
   cargo build --release --manifest-path src-tauri/Cargo.toml --bin quay-hook
   cp src-tauri/target/release/quay-hook ~/.local/bin/quay-hook
   ```

2. Merge into `~/.claude/settings.json` (applies to all projects, no session
   restart needed):

   ```json
   {
     "hooks": {
       "UserPromptSubmit": [
         { "hooks": [{ "type": "command", "command": "~/.local/bin/quay-hook working", "timeout": 5 }] }
       ],
       "PostToolUse": [
         { "matcher": "", "hooks": [{ "type": "command", "command": "~/.local/bin/quay-hook working", "timeout": 5 }] }
       ],
       "Notification": [
         { "hooks": [{ "type": "command", "command": "~/.local/bin/quay-hook waiting", "timeout": 5 }] }
       ],
       "Stop": [
         { "hooks": [{ "type": "command", "command": "~/.local/bin/quay-hook idle", "timeout": 5 }] }
       ],
       "SessionEnd": [
         { "hooks": [{ "type": "command", "command": "~/.local/bin/quay-hook ended", "timeout": 5 }] }
       ]
     }
   }
   ```

`PostToolUse → working` is what clears amber after you approve a permission —
no `UserPromptSubmit` fires on approval, only the tool runs.

Without the hooks nothing changes: claude sessions keep the plain
active/idle heuristic. Codex/OpenCode/Pi always use the heuristic (no
equivalent hook system is wired).

## Row actions

- **Jump to session** — focuses the exact terminal window/tab/pane hosting
  the session. The host is resolved from the session's own environment
  (multiplexers and env-labelled terminals win) or, failing that, from its
  process ancestry:

  | Host | Detected by | Focused via |
  |---|---|---|
  | Herdr | `HERDR_ENV` + `HERDR_PANE_ID` + `HERDR_SOCKET_PATH` | `herdr agent focus <pane>` on its socket, then a best-effort raise of the terminal hosting the attached herdr client |
  | Supacode | `SUPACODE_WORKTREE_ID`/`TAB_ID`/`SURFACE_ID` | supacode CLI: `tab focus` → `surface focus` → `open` |
  | Kitty | `KITTY_LISTEN_ON` + `KITTY_WINDOW_ID` | `kitten @ --to <socket> focus-window --match id:<id>` |
  | WezTerm | `WEZTERM_UNIX_SOCKET` + `WEZTERM_PANE` | `wezterm cli activate-pane --pane-id <id>` + `open -a WezTerm` |
  | Ghostty | ancestry (`/Ghostty.app/`) | AppleScript (Ghostty ≥ 1.3): match terminal by `tty` (1.4+) falling back to `working directory` (1.3) |
  | Terminal.app / iTerm2 | ancestry | AppleScript window lookup by the session's tty |

  **Kitty prerequisite:** remote control is off by default — the action only
  appears for kitty sessions when `kitty.conf` has `allow_remote_control yes`
  and `listen_on unix:/tmp/mykitty` (kitty appends `-<pid>`; the session's
  `KITTY_LISTEN_ON` env carries the exact socket).

  Bare tmux/screen/zellij panes are unsupported (ancestry dead-ends at the
  mux server). Before any focus, the PID is revalidated against agent + cwd,
  the host is re-resolved from the live process, and the tty paths recheck
  the current tty — so a recycled PID can't focus someone else's window.
- **Reveal in Finder** — opens the session's project folder.
- **Kill** — SIGTERM (lets the TUI restore your terminal); hold **⌥** for
  SIGKILL. The PID is revalidated against the agent + cwd right before
  signalling, so a stale row can't kill an unrelated process.
- **Ignore** — hides **all sessions of that agent in that folder**,
  persistently. Un-ignore via the chips in Settings.

## Known ceilings (v2)

- CPU/memory are **per-PID** — child processes (MCP servers, spawned tools)
  are not summed in.
- Activity and session names are **cwd-keyed**: two sessions in the same
  folder share the newest session's signal and label.
- OpenCode activity is CPU-only; Pi and OpenCode have no session names.
- Slugs are built from the raw process cwd; symlinked or `/private/var`
  canonicalized paths may miss the session dir — the state then degrades to
  the CPU signal. (Hook-state cwds are canonicalized by `quay-hook`, so the
  waiting signal is immune to this.)
- Hook state is cwd-keyed too: two claude sessions in one folder share it,
  waiting winning over working/idle. Per-session attribution needs a PID in
  the hook payload, which Claude Code doesn't provide.
- Jump to session covers Herdr, supacode, Kitty, WezTerm, Ghostty,
  Terminal.app, and iTerm2. Bare tmux/screen/zellij are the remaining gap.
  Ghostty 1.3 falls back to cwd matching (ambiguous when two terminals share
  a folder); exact tty matching engages on 1.4+. Herdr's host-terminal raise
  picks the first attached client when several herdr clients run at once.
