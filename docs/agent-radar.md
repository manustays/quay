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
  one `ps -axo pid=,tty=`; only PIDs with a `ttys…` terminal are considered
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
prompt".

## Row actions

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
  the CPU signal.
