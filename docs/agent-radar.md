# Agent Radar

Quay auto-discovers **interactive terminal AI-agent sessions** — Claude Code
(`claude`), Codex CLI (`codex`), OpenCode (`opencode`), and Pi (`pi`) — and
shows them in an **Agents** section of the popover (between Favorites and
More). Each session shows: activity dot, agent brand icon, project name,
CPU % / memory / uptime, and the session's working directory. Two or more
sessions in the same folder club into one **project folder row** with a stack
of overlapping agent badges on the right — which agents are in there, dimmed
when idle — that expands into the member rows.

## Switching it off

**Settings → Track AI coding agents** (`trackAgents`, on by default) is a master
switch, not a display filter. With it off:

- the scan loop skips the radar pass outright, so the `ps` + sysinfo work below
  never runs — turning it off actually costs nothing per tick;
- `refresh_waiting_badge` forces the count to zero, so the tray's waiting glyph
  and the `waitingTitleBadge` title clear immediately rather than freezing at
  their last value. The hook-driven poll loop keeps calling it, so this holds
  even though hooks keep firing;
- the popover drops its AGENTS section, and any rows already on screen are
  cleared when the setting is saved;
- **Show waiting count in menubar** is disabled, since it has nothing to count.

Hooks already installed are left alone — installing them is a deliberate,
separate action that writes to your Claude/Codex config, so a settings flip does
not undo it. Their state files are simply ignored while tracking is off, and the
hooks list in Settings greys out. Turn tracking back on and the radar resumes on
the next scan pass with no reinstall.

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
- The scan shares the port radar's loop but keeps its own cadence:
  `agentIntervalSec` (default 5 s) **while the popover is open**, nothing while
  it's hidden — the loop blocks on a condvar rather than idle-ticking, so a
  closed popover costs no wakeups at all. The port radar stays on its own 5 s
  tick; whichever is due first wakes the loop.
- Per-pass reads are cached where the source can't change behind us: Claude
  session names by log path (successes only — a log exists before its first
  prompt is written), codex rollout metas by path, and
  `~/.codex/session_index.jsonl` by `(len, mtime)`. Manifest name/stack lookups
  are deduped per pass, not across passes, so editing a manifest still shows up.

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

## The three states: working / idle / waiting

Each session's dot is one of:

- **pulsing green — working**: a turn is in progress.
- **hollow — idle**: waiting for you to type the next prompt, or done.
- **pulsing amber — waiting on you**: blocked at a permission prompt / an
  input request. In a clubbed folder row, one waiting member turns the whole
  folder's pill amber.

**Menubar signal (works while the popover is closed).** A waiting agent also
switches the *tray icon* to the amber submerged-buoy glyph (`tray-waiting`), and —
when the `waitingTitleBadge` setting is on — shows the waiting count in the menubar
title (e.g. `●2`). Unlike the in-popover dots, this is driven by the always-on poll
loop reading the hook-state files (`refresh_waiting_badge` → `waiting_count`), so it
pulls attention even when the popover is shut. Icon precedence puts a service `Error`
above waiting; waiting above `Starting`. The count dedups by `(agent, cwd)` and honors
`ignored_agents`.

**Keeping the badge honest.** The badge and the in-popover dots read the same files
but the dots apply corrections the raw count used to miss, so a stale `waiting` file
could badge the tray with no matching row. Two mechanisms now keep them in step:

- **Resume reconcile.** When a popover-open scan sees a session the resume backstop
  downgraded off `waiting` (its log advanced past the waiting event), it *deletes*
  that stale `waiting` file (`clear_resumed_waiting`), so the badge stops counting
  what the rows already hide. It only removes a `waiting` event whose own `ts`
  predates the resume evidence, so a sibling session still genuinely waiting in the
  same folder (newer event) is preserved.
- **PID liveness.** Each scan stamps the live PIDs it resolved per `(agent, cwd)`
  into `last_agent_pids`; `waiting_count` drops a waiting key whose every stamped PID
  is dead (`kill(pid, 0)`) — a *crashed-while-waiting* session clears without waiting
  for the 10-minute dead-session prune.
- **Orphan sweep.** A `waiting` file whose session died *unscanned* has no stamped
  PID to test, so it is swept by comparing the state dir against live sessions
  (`prune_orphan_hook_states` + `live_agent_keys`). That enumeration forks `ps`, and
  "an agent is waiting on you" is a steady state, not a rare one — so on the always-on
  poll loop it is rate-limited to once a minute (`PRUNE_INTERVAL_SECS`), while the
  popover-open path runs it unconditionally on every open and every agent pass. A
  phantom badge therefore clears within ~a minute in the background, or instantly the
  moment you open Quay.

Remaining ceilings: PIDs are keyed by `(agent, cwd)`, so a dead waiting session
sharing a folder with a live sibling still badges until the next popover-open scan;
`last_agent_pids` is empty after an app restart until the first scan, so a crash then
reverts to prune-only behavior; and a recycled PID can read alive. All self-heal on a
popover-open scan.

There are two sources for the state, and the better one wins:

1. **Hooks (authoritative).** With the radar hooks installed for an agent (one
   click in Settings — below), the agent reports its own lifecycle, so
   working/waiting/idle are exact — including "waiting at a permission prompt",
   which `ps` alone cannot see.
2. **Heuristic (fallback).** Without hooks, "working" is a **recent-activity
   signal, not proof of work**: a session-log write < 20 s ago **or** the
   process using > 10 % CPU; anything else is idle. There is no "waiting"
   without hooks — a permission prompt reads as idle. Per agent:
   - **Claude Code**: newest `.jsonl` mtime in `~/.claude/projects/<cwd-slug>/`.
   - **Codex**: newest matching `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`
     (by mtime across all date dirs, so sessions started days ago still match).
   - **Pi**: newest `.jsonl` under `~/.pi/agent/sessions/<cwd-slug>/` (slug
     inferred from one machine — best effort).
   - **OpenCode**: CPU only (sessions live in sqlite).

## Installing the hooks (Settings → Agent radar hooks)

Each agent has an **Install / Remove** button. Install does everything: it
copies the bundled `quay-hook` helper to an app-managed stable path
(`~/Library/Application Support/am.abhi.quay/bin/quay-hook`, kept current across
app updates) and writes that agent's hook config to reference it. No terminal,
no restart of running sessions. Remove strips only the entries Quay added,
leaving your own hooks untouched.

Each agent's config and event mapping:

| Agent | Config Quay writes | Events → state | Waiting? |
|---|---|---|---|
| **Claude Code** | `~/.claude/settings.json` | SessionStart *(startup/resume/clear/fork)* → idle · UserPromptSubmit/PostToolUse/PostToolUseFailure → working · Notification *(filtered)* → waiting · Stop → idle · SessionEnd → ended | yes |
| **Codex** | `~/.codex/hooks.json` | SessionStart *(startup/resume/clear)* → idle · UserPromptSubmit/PostToolUse → working · PermissionRequest → waiting · Stop → idle · SessionEnd → ended | yes |
| **OpenCode** | `~/.config/opencode/plugin/quay.js` | session.created → idle · tool.execute.before/permission.replied → working · permission.asked → waiting · session.idle → idle · session.deleted → ended | yes |
| **Pi** | `~/.pi/agent/extensions/quay.ts` | session_start → idle · agent_start → working · ui_prompt_start → waiting · agent_settled → idle · session_shutdown → ended | yes (any blocking UI prompt) |

**The `Notification` matcher is not optional.** `Notification` is a catch-all that also
fires for `auth_success`, `quota_auto_resume_fired` and the `elicitation_complete`
/`elicitation_response` pair. Subscribing to it unfiltered made every one of those a
waiting agent — an amber row and a tray badge for a session that wanted nothing.
`idle_prompt` is excluded too: it nudges about a session `Stop` has already marked
idle, so counting it would badge every finished session a minute after it finished.

**Both agents' `SessionStart` excludes `compact`.** Codex runs `SessionStart` hooks
matching `source: "compact"` after auto-compaction, *before the next model request* —
mid-turn — so matching it would blank a working row exactly when the agent is busiest.
Claude's docs are ambiguous on the same point, and the asymmetry settles it: wrongly
including `compact` is a visible wrong state, while wrongly excluding it only delays
discovery until the session's next event.

`clear` (and `fork`, on Claude) *are* included, because either may hand the session a
new id — and a session id the radar has never seen is one it does not know exists.

`PostToolUseFailure` carries the same `working` signal as `PostToolUse`, which fires
only on success; without it a session whose tool call errored looked stale until its
next successful call.

**Pi does have a waiting state after all.** It has no built-in permission prompt — that
much of the old note was right — but `ui_prompt_start`/`ui_prompt_end` bracket any
blocking prompt an extension raises (`confirm`, `select`, `input`, `editor`), which is
exactly "blocked on a question". Where `ui_prompt_end` returns to is decided by
`ctx.isIdle()`, since a prompt can be raised mid-run or at rest.

**OpenCode's working signal is `tool.execute.before`.** It is bounded — once per tool
call, like Claude's `PostToolUse` — where `message.updated` fires on every stream chunk
and would spawn the helper continuously. It does not cover a turn that only thinks and
never calls a tool, so a short reply can still read idle until `session.idle` confirms
it. That narrows the CPU-heuristic dependency rather than removing it outright. Its `SessionEnd` carries no matcher: `reason` is always `other` today, and
omitting it keeps catching whatever Codex adds later. Note that Codex also fires
`SessionEnd` after 30 minutes of inactivity, so a still-running CLI can lose its hook
state and fall back to the radar's own view of the process.

Pi reports idle on **`agent_settled`, not `agent_end`** — a run can end and then be
continued by pi's own retries, compaction or follow-ups, so `agent_end` flashed the row
idle in the middle of work.

Installed configs are **refreshed on launch** for agents already opted in
(`hooks_install::refresh_installed`), because what we install changes between versions;
a corrected matcher would otherwise only reach someone who toggled the hook off and on.
It never installs for an agent that has none.

`PostToolUse → working` is what clears amber after you approve a permission —
on approval only the tool runs, no `UserPromptSubmit` fires. The
OpenCode/Pi files are small plugins that shell out to the same `quay-hook`
helper, so there is one audited state-writer for every agent.

How the radar consumes it: `quay-hook` writes one small JSON file per session
(`{ agent, cwd, state, ts }`) under
`~/Library/Application Support/am.abhi.quay/agent-state/`, keyed by session and
tagged with the agent so two agents sharing a folder don't collide. The always-on poll
reads them (`pollIntervalSec`, independent of `agentIntervalSec`). It trusts **waiting** outright and a **fresh working** (event
within 5 min); a session-log write newer than a waiting event overrides it (the
session resumed), a stale working falls back to the heuristic, dead-session
files are pruned after a 10-minute grace, and corrupt/partial files are dropped
on read. Writes are atomic (temp + rename), so a mid-write poll never reads
half a file.

### Manual install (appendix)

The Settings button is the supported path; this is the equivalent by hand, e.g.
for Claude Code. `quay-hook` takes `<state> [agent]` — the agent defaults to
`claude` when omitted, so a pre-existing single-arg install keeps working.

```sh
cargo build --release --manifest-path src-tauri/Cargo.toml -p quay-hook
cp src-tauri/target/release/quay-hook ~/.local/bin/quay-hook
```

```json
{
  "hooks": {
    "UserPromptSubmit": [
      { "hooks": [{ "type": "command", "command": "~/.local/bin/quay-hook working claude", "timeout": 5 }] }
    ],
    "PostToolUse": [
      { "matcher": "", "hooks": [{ "type": "command", "command": "~/.local/bin/quay-hook working claude", "timeout": 5 }] }
    ],
    "Notification": [
      {
        "matcher": "permission_prompt|agent_needs_input|elicitation_dialog|elicitation_url_dialog",
        "hooks": [{ "type": "command", "command": "~/.local/bin/quay-hook waiting claude", "timeout": 5 }]
      }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "~/.local/bin/quay-hook idle claude", "timeout": 5 }] }
    ],
    "SessionEnd": [
      { "hooks": [{ "type": "command", "command": "~/.local/bin/quay-hook ended claude", "timeout": 5 }] }
    ]
  }
}
```

## Row actions

- **Jump to session** — focuses the exact terminal window/tab/pane hosting
  the session. The host is resolved from the session's own environment
  (multiplexers and env-labelled terminals win) or, failing that, from its
  process ancestry:

  | Host | Detected by | Focused via |
  |---|---|---|
  | Herdr | `HERDR_ENV` + `HERDR_PANE_ID` + `HERDR_SOCKET_PATH` | `herdr agent focus <pane>` on its socket, then a best-effort raise of the terminal hosting the attached herdr client |
  | Supacode | `SUPACODE_WORKTREE_ID`/`TAB_ID`/`SURFACE_ID` | supacode CLI: `tab focus` → `surface focus` → `open` |
  | Cmux | `CMUX_WORKSPACE_ID` + `CMUX_SURFACE_ID` (legacy `CMUX_TAB_ID`/`CMUX_PANEL_ID`) | deep link `open cmux://workspace/<ws>/surface/<sf>` — raises + focuses in one step |
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
- Without hooks, OpenCode activity is CPU-only; Pi and OpenCode have no
  session names.
- Slugs are built from the raw process cwd; symlinked or `/private/var`
  canonicalized paths may miss the session dir — the state then degrades to
  the CPU signal. (Hook-state cwds are canonicalized by `quay-hook`, so the
  hook signal is immune to this.)
- Hook state is keyed by (agent, cwd): different agents in one folder stay
  distinct, but **two sessions of the same agent in one folder share it**
  (waiting > working > idle). Per-session attribution needs a PID in the hook
  payload, which the agents don't provide. Pi reports working/idle only (no
  permission event); OpenCode "working" leans on the heuristic + its
  permission-replied event (no clean "turn started" event).
- The stable helper path (`…/am.abhi.quay/bin/quay-hook`) is written into each
  agent's config verbatim; if you install with hooks pointing at an old
  location, reinstall from Settings to re-point them.
- Jump to session covers Herdr, supacode, Cmux, Kitty, WezTerm, Ghostty,
  Terminal.app, and iTerm2. Bare tmux/screen/zellij are the remaining gap.
  Ghostty 1.3 falls back to cwd matching (ambiguous when two terminals share
  a folder); exact tty matching engages on 1.4+. Herdr's host-terminal raise
  picks the first attached client when several herdr clients run at once.
