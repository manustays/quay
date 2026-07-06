# Agent Radar

Quay auto-discovers **interactive terminal AI-agent sessions** — Claude Code
(`claude`), Codex CLI (`codex`), OpenCode (`opencode`), and Pi (`pi`) — and
shows them in an **Agents** section of the popover, one row per session:
activity dot, agent brand icon, project folder name, CPU % / memory / uptime,
and the session's working directory.

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
- The scan shares the port radar's loop: every 5 s **while the popover is
  open**, nothing while it's hidden.

## What "active" means

The dot is a **recent-activity signal, not proof of work**:

- **Claude Code**: newest `.jsonl` mtime in the session's
  `~/.claude/projects/<cwd-slug>/` directory < 20 s ago, **or** the process is
  using > 10 % CPU.
- **Pi**: same idea via `~/.pi/agent/sessions/<cwd-slug>/` (slug format
  inferred from one machine — best effort).
- **Codex / OpenCode**: CPU heuristic only (their session stores don't map
  back to a cwd cheaply).

Anything else shows **idle** — which includes "waiting at a permission
prompt".

## Row actions

- **Reveal in Finder** — opens the session's project folder.
- **Kill** — SIGTERM (lets the TUI restore your terminal); hold **⌥** for
  SIGKILL. The PID is revalidated against the agent + cwd right before
  signalling, so a stale row can't kill an unrelated process.
- **Ignore** — hides **all sessions of that agent in that folder**,
  persistently. Un-ignore via the chips in Settings.

## Known ceilings (v1)

- CPU/memory are **per-PID** — child processes (MCP servers, spawned tools)
  are not summed in.
- The activity mtime is **cwd-keyed**: two sessions in the same folder both
  read active when either writes.
- Codex/OpenCode activity is CPU-only.
- Slugs are built from the raw process cwd; symlinked or `/private/var`
  canonicalized paths may miss the session dir — the state then degrades to
  the CPU signal.
