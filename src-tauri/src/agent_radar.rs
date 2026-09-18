//! Discover interactive terminal AI-agent sessions ("agent radar").
//!
//! An interactive session = an agent binary (claude / codex / opencode / pi)
//! with an attached tty. sysinfo does not expose the controlling tty on macOS,
//! so each pass starts with one `ps -axo pid=,ppid=,tty=,comm=` to collect
//! tty-attached PIDs (plus ancestry for the host-terminal check backing
//! jump-to-session), then resolves argv/cwd/cpu/mem for just those via two targeted
//! sysinfo refreshes (`MINIMUM_CPU_UPDATE_INTERVAL` apart, so CPU% is a valid
//! delta). Runs inside the port-radar loop (scanner.rs): 5 s cadence, only
//! while the popover is visible; snapshots go out on `agents_discovered`.

use crate::detect;
use crate::state::AppState;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::time::SystemTime;
use sysinfo::{
	Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind,
};
use tauri::{AppHandle, Manager};

/// One discovered agent session, pushed to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredAgent {
	pub pid: u32,
	/// Agent kind: "claude" | "codex" | "opencode" | "pi".
	pub agent: &'static str,
	/// Display name: the project's manifest name, falling back to the cwd
	/// basename (same rule as the port radar — `detect::name_from_dir`).
	pub name: String,
	pub cwd: String,
	/// Detected tech stack keyword for the project folder (e.g. "vite").
	pub stack: Option<String>,
	/// Best-effort session label: claude = first user prompt of the newest
	/// session log, codex = the indexed thread name. cwd-keyed, so two
	/// sessions in one folder share it.
	// ponytail: per-session attribution needs lsof pid→open-file mapping.
	#[serde(rename = "sessionName")]
	pub session_name: Option<String>,
	pub uptime_sec: u64,
	pub memory_bytes: u64,
	/// "waiting" (hook-reported: session blocked on the user), "working"
	/// (hook-reported turn in progress, or — with no hooks — a recent
	/// session-log write / busy CPU), or "idle". Without hooks installed,
	/// working/idle is a recent-activity signal, not proof of work.
	pub state: &'static str,
	/// Controlling tty (e.g. "ttys002") — jump-to-session's window lookup key.
	pub tty: String,
	/// True when the configured/known terminal can be focused by tty via
	/// AppleScript (Terminal.app / iTerm). GPU terminals (Kitty, Ghostty…)
	/// have a tty too but no scriptable lookup, so the UI hides Jump for them.
	pub jump_supported: bool,
}


struct AgentDef {
	kind: &'static str,
	bin: &'static str,
}

const AGENTS: &[AgentDef] = &[
	AgentDef { kind: "claude", bin: "claude" },
	AgentDef { kind: "codex", bin: "codex" },
	AgentDef { kind: "opencode", bin: "opencode" },
	AgentDef { kind: "pi", bin: "pi" },
];

/// Identify an agent session from its argv. Pure.
///
/// Matches the argv\[0\] basename against the agent table, then applies
/// position/flag-specific exclusions (never substring-anywhere, so a project
/// path merely containing "daemon" can't be excluded):
/// - claude: drop `--bg-pty-host` / `--bg-spare` helpers (whole-element flag
///   match) and the `claude daemon …` subcommand. Belt-and-braces — those all
///   run without a tty anyway.
/// - codex: drop `codex mcp-server` / `codex app-server`.
/// - pi: bare `pi` (how a shell-launched session reads in ps) is accepted; a
///   pathed argv\[0\] must look like a real pi install (`/.pi/` or an
///   nvm/node bin dir), so any other tool named `pi` doesn't match.
// ponytail: pi disambiguation inferred from one machine; best-effort.
fn agent_from_argv(argv: &[String]) -> Option<&'static AgentDef> {
	let exe = argv.first()?;
	let base = exe.rsplit('/').next().unwrap_or(exe);
	let def = AGENTS.iter().find(|a| a.bin == base)?;
	let arg1 = argv.get(1).map(String::as_str);
	match def.kind {
		"claude" => {
			if argv.iter().any(|a| a == "--bg-pty-host" || a == "--bg-spare")
				|| arg1 == Some("daemon")
			{
				return None;
			}
		}
		"codex" => {
			if matches!(arg1, Some("mcp-server" | "app-server")) {
				return None;
			}
		}
		"pi" => {
			if exe.contains('/') && !exe.contains("/.pi/") && !exe.contains("/node/") {
				return None;
			}
		}
		_ => {}
	}
	Some(def)
}

/// One row of the per-pass `ps` snapshot.
struct PsProc {
	ppid: u32,
	/// Controlling tty ("ttys002"); `None` for `??` (daemons, .app bundles).
	tty: Option<String>,
	/// Executable path/name as reported by `comm` — used for the host-terminal
	/// ancestry walk (e.g. ".../iTerm.app/Contents/MacOS/iTerm2").
	comm: String,
}

/// Parse `ps -axo pid=,ppid=,tty=,comm=` output. Pure.
fn parse_ps_snapshot(out: &str) -> HashMap<u32, PsProc> {
	out.lines()
		.filter_map(|line| {
			let mut cols = line.split_whitespace();
			let pid = cols.next()?.parse().ok()?;
			let ppid = cols.next()?.parse().ok()?;
			let tty = cols.next()?;
			let tty = tty.starts_with("tty").then(|| tty.to_string());
			// comm may contain spaces (paths); rejoin the remainder.
			let comm = cols.collect::<Vec<_>>().join(" ");
			Some((pid, PsProc { ppid, tty, comm }))
		})
		.collect()
}

/// Full process snapshot — one `ps` per scan pass.
fn ps_snapshot() -> HashMap<u32, PsProc> {
	std::process::Command::new("ps")
		.args(["-axo", "pid=,ppid=,tty=,comm="])
		.output()
		.map(|o| parse_ps_snapshot(&String::from_utf8_lossy(&o.stdout)))
		.unwrap_or_default()
}

/// How to focus the window/tab/pane hosting a session — resolved from the
/// session's own environment (multiplexers and env-labelled terminals) or,
/// failing that, from its process ancestry (AppleScript-able terminal apps).
#[derive(Debug, Clone, PartialEq)]
pub enum JumpTarget {
	/// Herdr multiplexer pane: `herdr agent focus <pane>` against its socket,
	/// falling back to workspace+tab focus when herdr doesn't recognize the
	/// pane as an agent. The host terminal window is raised separately (see
	/// `jump_to_session`).
	Herdr { socket: String, pane: String, workspace: String, tab: String },
	/// Supacode surface: exact CLI coordinates.
	Supacode { worktree: String, tab: String, surface: String },
	/// Cmux surface: `cmux://workspace/<ws>/surface/<sf>` deep link (raises
	/// the window and focuses the surface in one step). IDs are UUIDs.
	Cmux { workspace: String, surface: String },
	/// Kitty window: remote-control socket + window id. Both env vars only
	/// exist when the user enabled `allow_remote_control` + `listen_on`.
	Kitty { socket: String, window_id: String },
	/// WezTerm pane: `wezterm cli activate-pane` against its socket.
	WezTerm { socket: String, pane: String },
	/// Ghostty ≥1.3: AppleScript match by tty (1.4+) or cwd (1.3 fallback).
	/// No per-surface env id exists, hence no fields.
	Ghostty,
	/// Terminal.app / iTerm2: AppleScript window lookup by controlling tty.
	ScriptableTty,
}

/// Resolve a jump target from an environ list of `K=V` strings. Pure.
///
/// Priority: Herdr first — a multiplexer pane's env can carry stale host
/// terminal vars (panes survive detach/reattach into a different terminal),
/// so the innermost layer must win. Kitty/WezTerm need both their vars: an id
/// without its socket is unreachable.
fn jump_target_from_env<'a>(env: impl Iterator<Item = &'a str>) -> Option<JumpTarget> {
	let mut vars: HashMap<&str, &str> = HashMap::new();
	for kv in env {
		if let Some((k, v)) = kv.split_once('=') {
			if k.starts_with("HERDR_") || k.starts_with("SUPACODE_") || k.starts_with("KITTY_")
				|| k.starts_with("WEZTERM_") || k.starts_with("CMUX_")
			{
				vars.insert(k, v);
			}
		}
	}
	let get = |k: &str| vars.get(k).map(|v| v.to_string());
	if vars.get("HERDR_ENV") == Some(&"1") {
		if let (Some(socket), Some(pane), Some(workspace), Some(tab)) = (
			get("HERDR_SOCKET_PATH"),
			get("HERDR_PANE_ID"),
			get("HERDR_WORKSPACE_ID"),
			get("HERDR_TAB_ID"),
		) {
			return Some(JumpTarget::Herdr { socket, pane, workspace, tab });
		}
	}
	if let (Some(worktree), Some(tab), Some(surface)) =
		(get("SUPACODE_WORKTREE_ID"), get("SUPACODE_TAB_ID"), get("SUPACODE_SURFACE_ID"))
	{
		return Some(JumpTarget::Supacode { worktree, tab, surface });
	}
	// CMUX_TAB_ID / CMUX_PANEL_ID are cmux's own legacy aliases of the two.
	if let (Some(workspace), Some(surface)) = (
		get("CMUX_WORKSPACE_ID").or_else(|| get("CMUX_TAB_ID")),
		get("CMUX_SURFACE_ID").or_else(|| get("CMUX_PANEL_ID")),
	) {
		return Some(JumpTarget::Cmux { workspace, surface });
	}
	if let (Some(socket), Some(window_id)) = (get("KITTY_LISTEN_ON"), get("KITTY_WINDOW_ID")) {
		return Some(JumpTarget::Kitty { socket, window_id });
	}
	if let (Some(socket), Some(pane)) = (get("WEZTERM_UNIX_SOCKET"), get("WEZTERM_PANE")) {
		return Some(JumpTarget::WezTerm { socket, pane });
	}
	None
}

/// Hosts focusable via process ancestry, matched by a path marker in an
/// ancestor's `comm`: AppleScript by tty (Terminal.app/iTerm) or Ghostty's
/// AppleScript dictionary.
// ponytail: bare tmux/screen/zellij panes stay unsupported — their ancestry
// dead-ends at the mux server; per-mux adapters are the upgrade path.
const ANCESTRY_HOSTS: &[(&str, JumpTarget)] = &[
	("/iTerm.app/", JumpTarget::ScriptableTty),
	("/Terminal.app/", JumpTarget::ScriptableTty),
	("/Ghostty.app/", JumpTarget::Ghostty),
];

/// Walk `pid`'s parent chain looking for a focusable host terminal.
/// Bounded so a cyclic/garbled table can't loop. Pure.
fn ancestry_target(pid: u32, table: &HashMap<u32, PsProc>) -> Option<JumpTarget> {
	let mut cur = pid;
	for _ in 0..20 {
		let p = table.get(&cur)?;
		if let Some((_, t)) = ANCESTRY_HOSTS.iter().find(|(m, _)| p.comm.contains(m)) {
			return Some(t.clone());
		}
		if p.ppid <= 1 {
			return None;
		}
		cur = p.ppid;
	}
	None
}

/// Jump target for a live PID, resolved fresh (env via sysinfo, ancestry via
/// one `ps` snapshot) — jump-to-session's click-time lookup, so a stale radar
/// row can't dispatch against recycled coordinates.
pub fn jump_target_for(pid: u32) -> Option<JumpTarget> {
	let mut sys = System::new();
	let sys_pid = Pid::from_u32(pid);
	sys.refresh_processes_specifics(
		ProcessesToUpdate::Some(&[sys_pid]),
		true,
		ProcessRefreshKind::nothing().with_environ(UpdateKind::Always),
	);
	let proc_ = sys.process(sys_pid)?;
	jump_target_from_env(proc_.environ().iter().filter_map(|s| s.to_str()))
		.or_else(|| ancestry_target(pid, &ps_snapshot()))
}

/// The attached herdr *client* process (tty-attached `herdr` binary) — a herdr
/// pane's own ancestry leads to the detachable herdr server, never the host
/// terminal, so raising the host means finding the client and focusing *it*.
// ponytail: first tty-attached herdr wins; multiple concurrent herdr clients
// would need an env socket match to disambiguate.
pub fn herdr_client_pid() -> Option<u32> {
	let ps = ps_snapshot();
	let mut pids: Vec<u32> = ps
		.iter()
		.filter(|(_, p)| {
			p.tty.is_some() && p.comm.rsplit('/').next().unwrap_or(&p.comm) == "herdr"
		})
		.map(|(&pid, _)| pid)
		.collect();
	pids.sort_unstable();
	pids.first().copied()
}
















/// Grace before a hook-state file whose (agent, cwd) has no live session is
/// deleted — long enough to ride out a transient cwd-resolution miss, short
/// enough that crashed-while-waiting sessions don't leave junk.
const HOOK_PRUNE_GRACE_SECS: u64 = 600;

/// A hook-reported "working" older than this (with no session-log write since)
/// is treated as stale and falls back to the mtime/CPU heuristic — guards a
/// missed Stop/idle hook from pinning a row green forever.
const WORKING_STALE_SECS: u64 = 300;


/// Three-way session state from what the hook last reported. Pure.
///
/// The hook is authoritative: it is now also what discovers the session, so there is
/// no second opinion to weigh it against. The one correction left is staleness — a
/// `working` that has gone quiet for `WORKING_STALE_SECS` with no clearing event is a
/// missed hook rather than a turn that has genuinely run that long, and pinning a row
/// green forever is worse than showing it idle a little early.
fn resolve_state(state: &str, ts: SystemTime, now: SystemTime) -> &'static str {
	match state {
		"waiting" => "waiting",
		"working" if now.duration_since(ts).unwrap_or_default().as_secs() <= WORKING_STALE_SECS => {
			"working"
		}
		_ => "idle",
	}
}


/// Count of distinct waiting agent `(agent, cwd)` pairs from the hook-state files —
/// the always-on menubar signal. Unlike [`hook_states`] this does **no** `ps` scan;
/// instead it consults `pids` (the live PIDs `scan` last stamped per key) to drop a
/// crashed-while-waiting session. Runs from the always-on poll loop so the menubar
/// reflects waiting agents while the popover is closed.
///
/// - Dedups by `(agent, cwd)`: two waiting sessions in one folder count once, matching
///   the folder-rollup UX (one amber member makes the folder need you).
/// - Honors `ignored`: a hidden agent+cwd pair never badges the menubar (same match as
///   [`scan`]).
/// - Liveness: a waiting key present in `pids` whose every seen PID is dead is skipped
///   (the session exited without a clearing hook). A key **absent** from `pids` was
///   never scanned, so it still counts — the popover-open scan will reconcile it.
/// - Skips corrupt JSON and files missing `cwd`/`state`; a missing `agent` defaults to
///   claude (older helpers wrote no agent field), mirroring [`hook_states`].
///
// ponytail: liveness is coarse — pids are keyed by (agent, cwd), so a dead waiting
// session sharing a folder with a live sibling still badges (can't attribute the
// waiting file to a pid without the lsof/identity work this cheap path skips). It
// self-heals on the next popover-open scan; upgrade path is a pid in the hook payload.
pub fn waiting_count(dir: &Path, ignored: &[crate::model::IgnoredAgent]) -> usize {
	let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
	let mut keys: HashSet<(String, String)> = HashSet::new();
	for entry in rd.flatten() {
		let path = entry.path();
		if path.extension().is_none_or(|x| x != "json") {
			continue;
		}
		let parsed = std::fs::read_to_string(&path).ok().and_then(|text| {
			let v: serde_json::Value = serde_json::from_str(&text).ok()?;
			let agent = v["agent"].as_str().unwrap_or("claude").to_string();
			Some((
				agent,
				v["cwd"].as_str()?.to_string(),
				v["state"].as_str()?.to_string(),
				recorded_identity(&v),
			))
		});
		let Some((agent, cwd, state, identity)) = parsed else { continue };
		if state != "waiting" {
			continue;
		}
		if ignored.iter().any(|i| i.agent == agent && i.cwd == cwd) {
			continue;
		}
		match identity {
			// The file names its own process, so liveness is exact and per-session:
			// no `ps`, and a dead sibling in a shared folder no longer badges.
			Some((pid, started_at)) => {
				if !quay_hook::proc_info::is_same_process(pid, started_at) {
					continue;
				}
			}
			// Written by an older helper, so there is nothing to check it against —
			// count it. It is rewritten with an identity on the session's next event,
			// and `prune_orphan_hook_states` still sweeps it on the legacy path.
			None => {}
		}
		keys.insert((agent, cwd));
	}
	keys.len()
}

/// The `(pid, startedAt)` a hook state file names, when it has one.
///
/// Both or neither: a pid without its start time is exactly the ambiguous liveness
/// check this pair exists to replace, so a half-written file is treated as legacy.
fn recorded_identity(v: &serde_json::Value) -> Option<(u32, u64)> {
	let pid = v["pid"].as_u64()?.try_into().ok()?;
	Some((pid, v["startedAt"].as_u64()?))
}

/// Live `(agent, cwd)` keys right now — tty-attached agent processes with a
/// resolvable cwd, minus ignored pairs. A lean cousin of [`scan`]'s candidate
/// pass for the always-on badge: refreshes only cwd+cmd (no cpu delta, no second
/// pass, no sleep) because the sweep needs identity, not metrics. Kept separate
/// from `scan` so the popover's hot path is untouched.
pub fn live_agent_keys(ignored: &[crate::model::IgnoredAgent]) -> HashSet<(String, String)> {
	let ps = ps_snapshot();
	let tty_pids: Vec<Pid> = ps
		.iter()
		.filter_map(|(&pid, p)| p.tty.as_deref().map(|_| Pid::from_u32(pid)))
		.collect();
	if tty_pids.is_empty() {
		return HashSet::new();
	}
	let mut sys = System::new();
	let refresh = ProcessRefreshKind::nothing()
		.with_cmd(UpdateKind::Always)
		.with_cwd(UpdateKind::Always);
	sys.refresh_processes_specifics(ProcessesToUpdate::Some(&tty_pids), true, refresh);
	tty_pids
		.iter()
		.filter_map(|&pid| {
			let proc_ = sys.process(pid)?;
			let argv: Vec<String> = proc_.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect();
			let def = agent_from_argv(&argv)?;
			let cwd = proc_.cwd()?.to_string_lossy().into_owned();
			if ignored.iter().any(|i| i.agent == def.kind && i.cwd == cwd) {
				return None;
			}
			Some((def.kind.to_string(), cwd))
		})
		.collect()
}

/// Delete `waiting` hook-state files whose `(agent, cwd)` has no live session and
/// whose event predates the grace window — the always-on cousin of the prune
/// baked into [`hook_states`], so the menubar badge stops counting a crashed or
/// exited waiting session without waiting for a popover scan. Scoped on purpose:
/// only `waiting` files are touched (`working`/`idle`/corrupt are left to
/// [`hook_states`] / [`waiting_count`], so the always-on path doesn't change their
/// handling), and the grace rides out a transient cwd-resolution miss the same way
/// `hook_states` does. Pure — no process enumeration — so it unit-tests headlessly.
pub fn prune_orphan_hook_states(
	dir: &Path,
	live: impl Fn() -> HashSet<(String, String)>,
	now: SystemTime,
) {
	let Ok(rd) = std::fs::read_dir(dir) else { return };
	// Only materialised if a legacy file turns up, because producing it forks `ps`.
	let mut legacy_live: Option<HashSet<(String, String)>> = None;
	for entry in rd.flatten() {
		let path = entry.path();
		if path.extension().is_none_or(|x| x != "json") {
			continue;
		}
		let parsed = std::fs::read_to_string(&path).ok().and_then(|text| {
			let v: serde_json::Value = serde_json::from_str(&text).ok()?;
			let agent = v["agent"].as_str().unwrap_or("claude").to_string();
			Some((
				agent,
				v["cwd"].as_str()?.to_string(),
				v["state"].as_str()?.to_string(),
				v["ts"].as_u64()?,
				recorded_identity(&v),
			))
		});
		let Some((agent, cwd, state, ts, identity)) = parsed else { continue };
		if state != "waiting" {
			continue;
		}
		// A file that names its own process needs no grace and no enumeration: the
		// (pid, start-time) pair either still exists or it does not.
		if let Some((pid, started_at)) = identity {
			if !quay_hook::proc_info::is_same_process(pid, started_at) {
				let _ = std::fs::remove_file(&path);
			}
			continue;
		}
		let live = legacy_live.get_or_insert_with(&live);
		if live.contains(&(agent.clone(), cwd.clone())) {
			continue;
		}
		let ts = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(ts);
		if now.duration_since(ts).unwrap_or_default().as_secs() > HOOK_PRUNE_GRACE_SECS {
			let _ = std::fs::remove_file(&path);
		}
	}
}

/// `pid`'s controlling tty right now (e.g. "ttys002") — jump-to-session's
/// click-time revalidation, so a recycled PID can't focus someone else's window.
pub fn current_tty(pid: u32) -> Option<String> {
	let out = std::process::Command::new("ps")
		.args(["-o", "tty=", "-p", &pid.to_string()])
		.output()
		.ok()?;
	let tty = String::from_utf8_lossy(&out.stdout).trim().to_string();
	tty.starts_with("tty").then_some(tty)
}

/// True when `pid` currently is an `agent` session in `cwd` — the kill
/// command's revalidation guard, so a stale row (session exited, PID reused —
/// even by the same binary in another folder) can't kill an unrelated process.
pub fn matches_identity(pid: u32, agent: &str, cwd: &str) -> bool {
	let mut sys = System::new();
	let sys_pid = Pid::from_u32(pid);
	sys.refresh_processes_specifics(
		ProcessesToUpdate::Some(&[sys_pid]),
		true,
		ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always).with_cwd(UpdateKind::Always),
	);
	let Some(proc_) = sys.process(sys_pid) else { return false };
	let argv: Vec<String> = proc_.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect();
	agent_from_argv(&argv).is_some_and(|d| d.kind == agent)
		&& proc_.cwd().is_some_and(|c| c.to_string_lossy() == cwd)
}


/// The `&'static str` agent kind for a name out of a state file, or `None` if the
/// file names an agent this build does not know.
fn agent_kind(name: &str) -> Option<&'static str> {
	AGENTS.iter().find(|a| a.kind == name).map(|a| a.kind)
}

/// A focusable host terminal among `pid`'s ancestors, walked without forking.
///
/// Terminal.app, iTerm and Ghostty publish no environment variable identifying
/// themselves, so ancestry is the only way to recognise them — see [`ANCESTRY_HOSTS`].
fn ancestry_host(pid: u32) -> Option<JumpTarget> {
	quay_hook::proc_info::ancestor_exes(pid, 20).iter().find_map(|exe| {
		ANCESTRY_HOSTS.iter().find(|(marker, _)| exe.contains(marker)).map(|(_, t)| t.clone())
	})
}

/// One live session, as its hook state file describes it.
///
/// Discovery is the set of these files, not a process scan: the agents report their
/// own sessions, so the radar no longer infers them from tty-attached processes.
struct Session {
	agent: String,
	cwd: String,
	state: String,
	ts: SystemTime,
	name: Option<String>,
	pid: u32,
	tty: String,
}

/// Every hook state file whose process is still the one that wrote it.
///
/// Deletes the files that fail that check. A file with no recorded `(pid, startedAt)`
/// was written by an older helper and is skipped rather than deleted: it will be
/// rewritten with one on the session's next event, and the always-on badge path still
/// honours it in the meantime.
fn live_sessions(
	dir: &Path,
	ignored: &[crate::model::IgnoredAgent],
) -> Vec<Session> {
	let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
	let mut out = Vec::new();
	for entry in rd.flatten() {
		let path = entry.path();
		if path.extension().is_none_or(|x| x != "json") {
			continue;
		}
		let parsed = std::fs::read_to_string(&path).ok().and_then(|text| {
			let v: serde_json::Value = serde_json::from_str(&text).ok()?;
			Some((
				v["agent"].as_str().unwrap_or("claude").to_string(),
				v["cwd"].as_str()?.to_string(),
				v["state"].as_str()?.to_string(),
				v["ts"].as_u64()?,
				v["name"].as_str().map(str::to_string),
				v["tty"].as_str().unwrap_or_default().to_string(),
				recorded_identity(&v),
			))
		});
		let Some((agent, cwd, state, ts, name, tty, identity)) = parsed else {
			let _ = std::fs::remove_file(&path); // corrupt/partial: drop it
			continue;
		};
		let Some((pid, started_at)) = identity else { continue };
		if !quay_hook::proc_info::is_same_process(pid, started_at) {
			// The session that wrote this is gone, and the pair says so exactly —
			// no grace window, no process enumeration.
			let _ = std::fs::remove_file(&path);
			continue;
		}
		if ignored.iter().any(|i| i.agent == agent && i.cwd == cwd) {
			continue;
		}
		out.push(Session {
			agent,
			cwd,
			state,
			ts: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(ts),
			name,
			pid,
			tty,
		});
	}
	out
}

pub fn scan(app: &AppHandle) -> Vec<DiscoveredAgent> {
	let (ignored, state_dir) = {
		let state = app.state::<AppState>();
		let cfg = state.config.lock().unwrap();
		(cfg.settings.ignored_agents.clone(), state.dir.join("agent-state"))
	};
	let sessions = live_sessions(&state_dir, &ignored);
	if sessions.is_empty() {
		return Vec::new();
	}

	// One refresh, over exactly the sessions we already know about — no `ps`, no
	// sweep of every tty-attached process, and no second pass. `cpu_usage()` was the
	// only thing that needed a second pass (and the 200 ms sleep between them, to
	// have a delta to measure); memory and run time are readable from one.
	let mut sys = System::new();
	let sys_pids: Vec<Pid> = sessions.iter().map(|s| Pid::from_u32(s.pid)).collect();
	sys.refresh_processes_specifics(
		ProcessesToUpdate::Some(&sys_pids),
		true,
		ProcessRefreshKind::nothing().with_memory().with_environ(UpdateKind::Always),
	);

	let now = SystemTime::now();
	// Manifest name + stack, deduped per pass: rows commonly share a cwd. Not cached
	// across passes — a manifest can be edited or created while Quay is open, and the
	// per-pass dedup already removes the repeated reads.
	let mut manifests: HashMap<String, (String, Option<String>)> = HashMap::new();

	let mut out: Vec<DiscoveredAgent> = sessions
		.iter()
		.filter_map(|session| {
			let proc_ = sys.process(Pid::from_u32(session.pid))?;
			let (name, stack) = manifests
				.entry(session.cwd.clone())
				.or_insert_with(|| {
					(
						detect::name_from_dir(Path::new(&session.cwd)),
						detect::stack_from_dir(Path::new(&session.cwd)).map(str::to_string),
					)
				})
				.clone();
			Some(DiscoveredAgent {
				pid: session.pid,
				agent: agent_kind(&session.agent)?,
				name,
				cwd: session.cwd.clone(),
				stack,
				// Straight from the hook. No fallback to the agent's own session logs:
				// `resolve_agent` deliberately never read them for a hooked session, to
				// avoid the macOS "access data from other apps" prompt, and every
				// session here is hooked by definition.
				session_name: session.name.clone(),
				uptime_sec: proc_.run_time(),
				memory_bytes: proc_.memory(),
				state: resolve_state(&session.state, session.ts, now),
				tty: session.tty.clone(),
				jump_supported: jump_target_from_env(
					proc_.environ().iter().filter_map(|s| s.to_str()),
				)
				.is_some()
					|| ancestry_host(session.pid).is_some(),
			})
		})
		.collect();
	out.sort_by_key(|a| a.pid);

	out
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Build an owned argv from string literals.
	fn argv(parts: &[&str]) -> Vec<String> {
		parts.iter().map(|s| s.to_string()).collect()
	}

	fn kind(parts: &[&str]) -> Option<&'static str> {
		agent_from_argv(&argv(parts)).map(|d| d.kind)
	}

	#[test]
	fn agent_from_argv_matches_interactive_sessions() {
		assert_eq!(kind(&["claude"]), Some("claude"));
		assert_eq!(kind(&["/Users/x/.local/bin/claude", "--resume", "abc"]), Some("claude"));
		assert_eq!(kind(&["codex"]), Some("codex"));
		assert_eq!(kind(&["opencode"]), Some("opencode"));
		assert_eq!(kind(&["pi"]), Some("pi"));
		assert_eq!(kind(&["/Users/x/.nvm/versions/node/v24.16.0/bin/pi"]), Some("pi"));
	}

	#[test]
	fn agent_from_argv_excludes_helpers_and_daemons() {
		assert_eq!(kind(&["claude", "daemon", "run"]), None);
		assert_eq!(
			kind(&["/Users/x/.local/share/claude/versions/2.1.195", "--bg-pty-host", "s.sock"]),
			None
		);
		assert_eq!(kind(&["claude", "--bg-spare", "s.sock"]), None);
		assert_eq!(kind(&["codex", "mcp-server"]), None);
		assert_eq!(kind(&["codex", "app-server"]), None);
		// `daemon` must only exclude in subcommand position, never as substring.
		assert_eq!(kind(&["claude", "run-my-daemon-thing"]), Some("claude"));
	}

	#[test]
	fn agent_from_argv_rejects_pi_lookalikes() {
		assert_eq!(kind(&["/usr/local/bin/pi"]), None);
		assert_eq!(kind(&["/x/pinocchio/serve"]), None);
		assert_eq!(kind(&[]), None);
	}


	#[test]
	fn parse_ps_snapshot_keeps_tty_and_ancestry() {
		let out = concat!(
			"1 0 ?? /sbin/launchd\n",
			"123 90 ttys002 /Users/x/.local/bin/claude\n",
			"456 1 ?? /usr/libexec/somehelper\n",
			"90 80 ttys002 -zsh\n",
			"80 1 ?? /Applications/iTerm.app/Contents/MacOS/iTerm2\n",
			"garbage line\n",
		);
		let t = parse_ps_snapshot(out);
		assert_eq!(t.len(), 5);
		assert_eq!(t[&123].tty.as_deref(), Some("ttys002"));
		assert_eq!(t[&123].ppid, 90);
		assert_eq!(t[&456].tty, None);
		assert!(t[&80].comm.contains("/iTerm.app/"));
	}



	#[test]
	fn ancestry_target_walks_parent_chain() {
		let out = concat!(
			"123 90 ttys002 claude\n",
			"90 80 ttys002 -zsh\n",
			"80 1 ?? /Applications/iTerm.app/Contents/MacOS/iTerm2\n",
			"223 190 ttys003 claude\n",
			"190 180 ttys003 -zsh\n",
			"180 1 ?? /Applications/Ghostty.app/Contents/MacOS/ghostty\n",
			"323 1 ttys004 claude\n", // orphaned (tmux-style): no terminal ancestor
		);
		let t = parse_ps_snapshot(out);
		assert_eq!(ancestry_target(123, &t), Some(JumpTarget::ScriptableTty)); // iTerm
		assert_eq!(ancestry_target(223, &t), Some(JumpTarget::Ghostty));
		assert_eq!(ancestry_target(323, &t), None); // dead-ends at launchd
		assert_eq!(ancestry_target(999, &t), None); // unknown pid
	}


	#[test]
	fn waiting_count_dedups_filters_and_skips_junk() {
		use crate::model::IgnoredAgent;
		let d = std::env::temp_dir().join(format!("msm-wc-{}", uuid::Uuid::new_v4()));
		// Missing dir → 0.
		assert_eq!(waiting_count(&d, &[]), 0);
		std::fs::create_dir_all(&d).unwrap();
		let write = |name: &str, body: &str| std::fs::write(d.join(name), body).unwrap();
		// Two waiting sessions in the SAME (agent, cwd) → dedup to one.
		write("s1.json", r#"{"agent":"claude","cwd":"/a","state":"waiting","ts":1}"#);
		write("s2.json", r#"{"agent":"claude","cwd":"/a","state":"waiting","ts":2}"#);
		// A distinct waiting pair → +1.
		write("s3.json", r#"{"agent":"codex","cwd":"/b","state":"waiting","ts":3}"#);
		// Non-waiting states don't count.
		write("s4.json", r#"{"agent":"claude","cwd":"/c","state":"working","ts":4}"#);
		write("s5.json", r#"{"agent":"claude","cwd":"/d","state":"idle","ts":5}"#);
		// Corrupt / missing-field / non-json files are skipped.
		write("bad.json", "not json");
		write("nofield.json", r#"{"agent":"claude","ts":6}"#);
		write("note.txt", r#"{"agent":"claude","cwd":"/e","state":"waiting","ts":7}"#);
		assert_eq!(waiting_count(&d, &[]), 2); // (claude,/a) + (codex,/b)
		// Ignoring (codex,/b) drops it.
		let ignored = vec![IgnoredAgent { agent: "codex".into(), cwd: "/b".into() }];
		assert_eq!(waiting_count(&d, &ignored), 1);
		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn waiting_count_trusts_a_recorded_identity_over_the_scanned_pids() {
		// Per-session liveness, which the coarse (agent, cwd) PID map cannot do: a
		// dead session sharing a folder with a live sibling used to keep the badge lit.
		let dir = std::env::temp_dir().join(format!("quay-wc-id-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&dir).unwrap();
		let me = std::process::id();
		let started = quay_hook::proc_info::info(me).unwrap().started_at;
		let write = |name: &str, cwd: &str, pid: u32, started_at: u64| {
			let body = serde_json::json!({
				"agent": "claude", "cwd": cwd, "state": "waiting",
				"ts": 1_000_000, "pid": pid, "startedAt": started_at,
			});
			std::fs::write(dir.join(name), body.to_string()).unwrap();
		};
		write("live.json", "/shared", me, started);
		write("dead.json", "/shared2", 4_000_000_000, started);
		write("recycled.json", "/shared3", me, started + 1);

		// Deliberately empty: the recorded identity must be used instead, so the map
		// the popover scan stamps is not consulted at all.
		assert_eq!(
			waiting_count(&dir, &[]),
			1,
			"only the session whose process is genuinely still there counts"
		);
		let _ = std::fs::remove_dir_all(&dir);
	}









	#[test]
	fn jump_target_from_env_resolves_each_host() {
		let t = |env: &[&str]| jump_target_from_env(env.iter().copied());
		assert_eq!(
			t(&[
				"PATH=/usr/bin",
				"SUPACODE_WORKTREE_ID=%2Fx%2F",
				"SUPACODE_TAB_ID=TAB",
				"SUPACODE_SURFACE_ID=SUR",
			]),
			Some(JumpTarget::Supacode {
				worktree: "%2Fx%2F".into(),
				tab: "TAB".into(),
				surface: "SUR".into(),
			})
		);
		assert_eq!(
			t(&[
				"HERDR_ENV=1",
				"HERDR_SOCKET_PATH=/s/herdr.sock",
				"HERDR_PANE_ID=w1:p2",
				"HERDR_WORKSPACE_ID=w1",
				"HERDR_TAB_ID=w1:t1",
			]),
			Some(JumpTarget::Herdr {
				socket: "/s/herdr.sock".into(),
				pane: "w1:p2".into(),
				workspace: "w1".into(),
				tab: "w1:t1".into(),
			})
		);
		assert_eq!(
			t(&["KITTY_WINDOW_ID=3", "KITTY_LISTEN_ON=unix:/tmp/mykitty-42"]),
			Some(JumpTarget::Kitty { socket: "unix:/tmp/mykitty-42".into(), window_id: "3".into() })
		);
		assert_eq!(
			t(&["CMUX_WORKSPACE_ID=AB-12", "CMUX_SURFACE_ID=CD-34", "CMUX_SOCKET_PATH=/s"]),
			Some(JumpTarget::Cmux { workspace: "AB-12".into(), surface: "CD-34".into() })
		);
		// Legacy alias names resolve too.
		assert_eq!(
			t(&["CMUX_TAB_ID=AB-12", "CMUX_PANEL_ID=CD-34"]),
			Some(JumpTarget::Cmux { workspace: "AB-12".into(), surface: "CD-34".into() })
		);
		assert_eq!(
			t(&["WEZTERM_PANE=7", "WEZTERM_UNIX_SOCKET=/tmp/wez.sock"]),
			Some(JumpTarget::WezTerm { socket: "/tmp/wez.sock".into(), pane: "7".into() })
		);
	}

	#[test]
	fn jump_target_from_env_priority_and_missing_vars() {
		let t = |env: &[&str]| jump_target_from_env(env.iter().copied());
		// A herdr pane created inside kitty carries both — innermost (herdr) wins.
		assert!(matches!(
			t(&[
				"KITTY_WINDOW_ID=3",
				"KITTY_LISTEN_ON=unix:/tmp/k",
				"HERDR_ENV=1",
				"HERDR_SOCKET_PATH=/s.sock",
				"HERDR_PANE_ID=w1:p1",
				"HERDR_WORKSPACE_ID=w1",
				"HERDR_TAB_ID=w1:t1",
			]),
			Some(JumpTarget::Herdr { .. })
		));
		// Kitty id without a remote-control socket is unreachable → None.
		assert_eq!(t(&["KITTY_WINDOW_ID=3"]), None);
		// Herdr marker without coordinates → None (not a half-focus).
		assert_eq!(t(&["HERDR_ENV=1"]), None);
		// Partial supacode → None.
		assert_eq!(t(&["SUPACODE_TAB_ID=x"]), None);
		assert_eq!(t(&[]), None);
	}

	/// A state file as the current helper writes one.
	fn write_state(dir: &Path, file: &str, cwd: &str, state: &str, pid: u32, started: u64, ts: u64) {
		let body = serde_json::json!({
			"agent": "claude", "cwd": cwd, "state": state, "ts": ts,
			"pid": pid, "startedAt": started, "tty": "ttys001",
		});
		std::fs::write(dir.join(file), body.to_string()).unwrap();
	}

	fn tmpdir(tag: &str) -> PathBuf {
		let d = std::env::temp_dir().join(format!("quay-{tag}-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&d).unwrap();
		d
	}

	#[test]
	fn discovery_is_the_set_of_live_state_files() {
		// The radar no longer infers sessions from tty-attached processes: a session
		// exists because its agent said so, and stops existing when its process does.
		let dir = tmpdir("live-sessions");
		let me = std::process::id();
		let started = quay_hook::proc_info::info(me).unwrap().started_at;

		write_state(&dir, "live.json", "/a", "working", me, started, 1_000);
		write_state(&dir, "dead.json", "/b", "waiting", 4_000_000_000, started, 1_000);
		write_state(&dir, "recycled.json", "/c", "waiting", me, started + 1, 1_000);
		// No pid: written by an older helper. Skipped for discovery, but kept on disk
		// so the always-on badge path can still honour it until it is rewritten.
		std::fs::write(
			dir.join("legacy.json"),
			serde_json::json!({"agent":"claude","cwd":"/d","state":"waiting","ts":1_000}).to_string(),
		)
		.unwrap();
		std::fs::write(dir.join("corrupt.json"), "{not json").unwrap();

		let found = live_sessions(&dir, &[]);
		assert_eq!(found.len(), 1, "only the session whose process is still there");
		assert_eq!(found[0].cwd, "/a");
		assert_eq!(found[0].tty, "ttys001", "tty comes from the file, not from ps");

		assert!(!dir.join("dead.json").exists(), "a dead session's file is reclaimed");
		assert!(!dir.join("recycled.json").exists(), "a recycled pid is not the same session");
		assert!(!dir.join("corrupt.json").exists(), "unparseable files are dropped");
		assert!(dir.join("legacy.json").exists(), "a pid-less file is skipped, not deleted");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn an_ignored_pair_is_discovered_but_not_returned() {
		let dir = tmpdir("live-ignored");
		let me = std::process::id();
		let started = quay_hook::proc_info::info(me).unwrap().started_at;
		write_state(&dir, "a.json", "/hidden", "waiting", me, started, 1_000);

		let ignored = vec![crate::model::IgnoredAgent {
			agent: "claude".to_string(),
			cwd: "/hidden".to_string(),
		}];
		assert!(live_sessions(&dir, &ignored).is_empty(), "hidden pairs never surface");
		assert!(dir.join("a.json").exists(), "hiding a row must not delete its state");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn a_working_state_goes_stale_but_waiting_never_does() {
		let now = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
		let ago = |secs: u64| now - std::time::Duration::from_secs(secs);

		assert_eq!(resolve_state("working", ago(10), now), "working");
		// A missed clearing hook must not pin a row green forever.
		assert_eq!(resolve_state("working", ago(WORKING_STALE_SECS + 1), now), "idle");
		// Waiting has no staleness rule: a session blocked on a permission prompt can
		// sit there for hours and is still blocked on you.
		assert_eq!(resolve_state("waiting", ago(86_400), now), "waiting");
		assert_eq!(resolve_state("idle", ago(1), now), "idle");
		assert_eq!(resolve_state("anything-else", ago(1), now), "idle");
	}

	#[test]
	fn prune_uses_identity_when_present_and_the_grace_only_when_not() {
		let dir = tmpdir("prune");
		let me = std::process::id();
		let started = quay_hook::proc_info::info(me).unwrap().started_at;
		let now = SystemTime::now();
		let secs = |t: SystemTime| t.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
		let old = secs(now) - (HOOK_PRUNE_GRACE_SECS + 60);

		// Identity present: exact, and no grace needed either way.
		write_state(&dir, "alive.json", "/a", "waiting", me, started, old);
		write_state(&dir, "gone.json", "/b", "waiting", 4_000_000_000, started, secs(now));
		// Legacy: falls back to the live set and the grace window.
		let legacy = |file: &str, cwd: &str, ts: u64| {
			let body = serde_json::json!({"agent":"claude","cwd":cwd,"state":"waiting","ts":ts});
			std::fs::write(dir.join(file), body.to_string()).unwrap();
		};
		legacy("legacy_dead_stale.json", "/gone", old);
		legacy("legacy_dead_fresh.json", "/gone2", secs(now) - 10);
		legacy("legacy_live.json", "/live", old);

		let live: HashSet<(String, String)> =
			[("claude".to_string(), "/live".to_string())].into();
		prune_orphan_hook_states(&dir, || live.clone(), now);

		assert!(dir.join("alive.json").exists(), "a live process keeps its row, however old");
		assert!(!dir.join("gone.json").exists(), "a dead process clears with no grace");
		assert!(!dir.join("legacy_dead_stale.json").exists(), "legacy dead+stale is pruned");
		assert!(dir.join("legacy_dead_fresh.json").exists(), "legacy within grace survives");
		assert!(dir.join("legacy_live.json").exists(), "legacy live-keyed survives");
		let _ = std::fs::remove_dir_all(&dir);
	}
}
