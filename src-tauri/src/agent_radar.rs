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
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use sysinfo::{
	MINIMUM_CPU_UPDATE_INTERVAL, Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind,
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
	/// Per-PID CPU% (may exceed 100 on multi-core).
	// ponytail: per-PID only, no child-subtree sum; metrics::aggregate_tree is
	// the upgrade if the numbers look too small.
	pub cpu_percent: f32,
	pub memory_bytes: u64,
	/// "waiting" (hook-reported: session blocked on the user — claude only),
	/// "active" (recent session-log write or busy CPU), or "idle". Active/idle
	/// is a recent-activity signal, not proof of work.
	pub state: &'static str,
	/// Controlling tty (e.g. "ttys002") — jump-to-session's window lookup key.
	pub tty: String,
	/// True when the configured/known terminal can be focused by tty via
	/// AppleScript (Terminal.app / iTerm). GPU terminals (Kitty, Ghostty…)
	/// have a tty too but no scriptable lookup, so the UI hides Jump for them.
	pub jump_supported: bool,
}

/// How a session's active/idle state (and name, where possible) is derived.
enum Activity {
	/// Newest `.jsonl` under `~/.claude/projects/<claude_slug(cwd)>/`; its
	/// mtime is the activity signal and its first user prompt the name.
	ClaudeJsonl,
	/// Newest `.jsonl` mtime under `~/.pi/agent/sessions/<pi_slug(cwd)>/`.
	/// No session name on disk.
	PiJsonl,
	/// Newest `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` whose
	/// `session_meta` cwd matches; mtime is the activity signal and the
	/// indexed `thread_name` the session name.
	CodexRollout,
	/// CPU-only heuristic (opencode stores sessions in sqlite).
	// ponytail: `lsof -p <pid>` mapping the open db is the upgrade.
	Cpu,
}

struct AgentDef {
	kind: &'static str,
	bin: &'static str,
	activity: Activity,
}

const AGENTS: &[AgentDef] = &[
	AgentDef { kind: "claude", bin: "claude", activity: Activity::ClaudeJsonl },
	AgentDef { kind: "codex", bin: "codex", activity: Activity::CodexRollout },
	AgentDef { kind: "opencode", bin: "opencode", activity: Activity::Cpu },
	AgentDef { kind: "pi", bin: "pi", activity: Activity::PiJsonl },
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

/// Claude Code project-dir slug: every non-alphanumeric char → `-`.
/// Verified: `/Users/abhi/DEV/abhi_github/abhi.page.11ty` →
/// `-Users-abhi-DEV-abhi-github-abhi-page-11ty`.
// ponytail: built from the raw sysinfo cwd; a symlinked//private/var path may
// miss the session dir, degrading gracefully to the CPU signal.
fn claude_slug(cwd: &str) -> String {
	cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// Pi session-dir slug: `/` → `-` (other chars kept), wrapped as `-<…>--`.
/// Verified against one real dir: `/Users/abhi/DEV/eko_github/wlc-webapp` →
/// `--Users-abhi-DEV-eko_github-wlc-webapp--`.
// ponytail: inferred from a single sample; on mismatch the state degrades to
// the CPU signal automatically.
fn pi_slug(cwd: &str) -> String {
	format!("-{}--", cwd.replace('/', "-"))
}

/// Newest `.jsonl` (path + mtime) in `dir`, non-recursive. None when the dir
/// is missing or holds no `.jsonl` — the caller degrades to the CPU signal.
fn newest_jsonl(dir: &Path) -> Option<(PathBuf, SystemTime)> {
	std::fs::read_dir(dir)
		.ok()?
		.flatten()
		.filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
		.filter_map(|e| {
			let mtime = e.metadata().ok()?.modified().ok()?;
			Some((e.path(), mtime))
		})
		.max_by_key(|&(_, m)| m)
}

/// First real user prompt in a Claude Code session log — the closest thing
/// to a session title on disk. Skips meta entries, tool-result-only user
/// messages, and slash-command envelopes (`<command-name>…`). Pure.
fn first_user_prompt(jsonl: &str) -> Option<String> {
	for line in jsonl.lines() {
		let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
		if v["type"] != "user" || v["isMeta"] == true {
			continue;
		}
		let content = &v["message"]["content"];
		let text = if let Some(s) = content.as_str() {
			s
		} else if let Some(blocks) = content.as_array() {
			let text_block = blocks
				.iter()
				.find_map(|b| if b["type"] == "text" { b["text"].as_str() } else { None });
			match text_block {
				Some(t) => t,
				None => continue, // tool_result-only user entry
			}
		} else {
			continue;
		};
		let text = text.trim();
		if text.is_empty() || text.starts_with('<') {
			continue;
		}
		return Some(truncate_label(text));
	}
	None
}

/// Cap a session label at 80 chars (char-boundary safe) with an ellipsis.
fn truncate_label(text: &str) -> String {
	let mut label: String = text.chars().take(80).collect();
	if label.len() < text.len() {
		label.push('…');
	}
	label
}

/// Parse a codex rollout file's first `session_meta` line → (cwd, session id).
/// Pure; None for anything that isn't a session_meta with both fields.
fn rollout_meta(line: &str) -> Option<(String, String)> {
	let v: serde_json::Value = serde_json::from_str(line).ok()?;
	if v["type"] != "session_meta" {
		return None;
	}
	let p = &v["payload"];
	let cwd = p["cwd"].as_str()?.to_string();
	let id = p["id"].as_str().or_else(|| p["session_id"].as_str())?.to_string();
	Some((cwd, id))
}

/// Look up a codex session id's `thread_name` in `session_index.jsonl` text.
/// Last matching line wins (names get regenerated over time). Pure.
fn thread_name_for(index: &str, id: &str) -> Option<String> {
	let mut name = None;
	for line in index.lines() {
		let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
		if v["id"] == id {
			if let Some(n) = v["thread_name"].as_str() {
				name = Some(truncate_label(n));
			}
		}
	}
	name
}

/// All codex rollout files as (cwd, session id, mtime), walking every
/// `~/.codex/sessions/YYYY/MM/DD/` dir. The `session_meta` first line is
/// immutable, so it's cached by path across passes; only mtimes are re-read.
/// A long-running session's rollout lives under its *start* date — selection
/// is by mtime, never by directory recency.
fn codex_rollouts(
	home: &Path,
	meta_cache: &mut HashMap<PathBuf, Option<(String, String)>>,
) -> Vec<(String, String, SystemTime)> {
	fn subdirs(dir: &Path) -> Vec<PathBuf> {
		std::fs::read_dir(dir)
			.map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
			.unwrap_or_default()
	}
	let mut out = Vec::new();
	for year in subdirs(&home.join(".codex/sessions")) {
		for month in subdirs(&year) {
			for day in subdirs(&month) {
				let Ok(rd) = std::fs::read_dir(&day) else { continue };
				for entry in rd.flatten() {
					let path = entry.path();
					if path.extension().is_none_or(|x| x != "jsonl") {
						continue;
					}
					let meta = meta_cache
						.entry(path.clone())
						.or_insert_with(|| {
							// Only the first line is needed; rollouts can be MBs.
							let mut head = String::new();
							use std::io::BufRead;
							if let Ok(f) = std::fs::File::open(&path) {
								let _ = std::io::BufReader::new(f).read_line(&mut head);
							}
							rollout_meta(&head)
						})
						.clone();
					let Some((cwd, id)) = meta else { continue };
					let Some(mtime) = entry.metadata().ok().and_then(|m| m.modified().ok())
					else {
						continue;
					};
					out.push((cwd, id, mtime));
				}
			}
		}
	}
	out
}

/// Per-cwd session signal: newest log mtime + best-effort session name.
struct SessionInfo {
	mtime: Option<SystemTime>,
	name: Option<String>,
}

/// Resolve the session signal for one agent candidate. Every miss degrades
/// gracefully: no mtime → CPU-only activity, no name → None.
fn session_info(
	def: &AgentDef,
	home: &Path,
	cwd: &str,
	codex: &[(String, String, SystemTime)],
	codex_index: &str,
) -> SessionInfo {
	match def.activity {
		Activity::ClaudeJsonl => {
			let dir = home.join(".claude/projects").join(claude_slug(cwd));
			let Some((path, mtime)) = newest_jsonl(&dir) else {
				return SessionInfo { mtime: None, name: None };
			};
			// Read a bounded head: the first user prompt sits within the first
			// few entries; a session log itself can grow to many MBs.
			let name = read_head(&path, 256 * 1024).as_deref().and_then(first_user_prompt);
			SessionInfo { mtime: Some(mtime), name }
		}
		Activity::PiJsonl => {
			let dir = home.join(".pi/agent/sessions").join(pi_slug(cwd));
			SessionInfo { mtime: newest_jsonl(&dir).map(|(_, m)| m), name: None }
		}
		Activity::CodexRollout => {
			let newest = codex
				.iter()
				.filter(|(c, _, _)| c == cwd)
				.max_by_key(|&&(_, _, m)| m);
			match newest {
				Some((_, id, mtime)) => SessionInfo {
					mtime: Some(*mtime),
					name: thread_name_for(codex_index, id),
				},
				None => SessionInfo { mtime: None, name: None },
			}
		}
		Activity::Cpu => SessionInfo { mtime: None, name: None },
	}
}

/// Read up to `cap` bytes from the start of a file as (lossy) UTF-8.
fn read_head(path: &Path, cap: usize) -> Option<String> {
	use std::io::Read;
	let mut buf = vec![0u8; cap];
	let mut f = std::fs::File::open(path).ok()?;
	let mut filled = 0;
	while filled < cap {
		match f.read(&mut buf[filled..]) {
			Ok(0) => break,
			Ok(n) => filled += n,
			Err(_) => return None,
		}
	}
	buf.truncate(filled);
	Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Active = a session-log write in the last 20 s, or busy CPU. Pure given a
/// clock reading.
// ponytail: the mtime is cwd-keyed, so two sessions sharing a cwd both read
// active when either writes; per-session attribution via lsof is the upgrade.
fn is_active(log_mtime: Option<SystemTime>, cpu_percent: f32, now: SystemTime) -> bool {
	let recent_write = log_mtime
		.and_then(|m| now.duration_since(m).ok())
		.is_some_and(|age| age.as_secs() < 20);
	recent_write || cpu_percent > 10.0
}

/// One hook-reported session state, written by `quay-hook` (see
/// `src/bin/quay-hook.rs`): the event's cwd, the mapped state string, and the
/// event time as unix seconds.
struct HookState {
	state: String,
	ts: SystemTime,
}

/// Grace before a hook-state file whose cwd has no live session is deleted —
/// long enough to ride out a transient cwd-resolution miss, short enough that
/// crashed-while-waiting sessions don't leave junk.
const HOOK_PRUNE_GRACE_SECS: u64 = 600;

/// Read `agent-state/*.json` into cwd → strongest state. Precedence per cwd:
/// waiting > working > idle (matches the folder-clubbing UX — one amber member
/// makes the folder need you). Files whose cwd has no live claude session and
/// whose event is older than the grace window are pruned in the same walk.
fn hook_states(dir: &Path, live_cwds: &[&str], now: SystemTime) -> HashMap<String, HookState> {
	let mut out: HashMap<String, HookState> = HashMap::new();
	let Ok(rd) = std::fs::read_dir(dir) else { return out };
	let rank = |s: &str| match s {
		"waiting" => 2,
		"working" => 1,
		_ => 0,
	};
	for entry in rd.flatten() {
		let path = entry.path();
		if path.extension().is_none_or(|x| x != "json") {
			continue;
		}
		let parsed = std::fs::read_to_string(&path).ok().and_then(|text| {
			let v: serde_json::Value = serde_json::from_str(&text).ok()?;
			Some((v["cwd"].as_str()?.to_string(), v["state"].as_str()?.to_string(), v["ts"].as_u64()?))
		});
		let Some((cwd, state, ts)) = parsed else {
			let _ = std::fs::remove_file(&path); // corrupt/partial: drop it
			continue;
		};
		let ts = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(ts);
		let age = now.duration_since(ts).unwrap_or_default().as_secs();
		if !live_cwds.contains(&cwd.as_str()) {
			if age > HOOK_PRUNE_GRACE_SECS {
				let _ = std::fs::remove_file(&path);
			}
			continue;
		}
		let stronger = out
			.get(&cwd)
			.is_none_or(|cur| rank(&state) > rank(&cur.state) || (rank(&state) == rank(&cur.state) && ts > cur.ts));
		if stronger {
			out.insert(cwd, HookState { state, ts });
		}
	}
	out
}

/// Three-way session state. The hook's "waiting" wins — unless the session log
/// was written *after* the hook event (the session resumed but no clearing
/// hook has landed yet, e.g. hooks were unregistered mid-session); then the
/// mtime/CPU heuristic decides active vs idle as before. Pure.
// ponytail: hook state is cwd-keyed like everything else here; two claude
// sessions in one folder share it (waiting wins per hook_states precedence).
fn resolve_state(
	hook: Option<&HookState>,
	log_mtime: Option<SystemTime>,
	cpu_percent: f32,
	now: SystemTime,
) -> &'static str {
	if let Some(h) = hook {
		if h.state == "waiting" {
			let resumed = log_mtime
				.is_some_and(|m| m > h.ts + std::time::Duration::from_secs(2));
			if !resumed {
				return "waiting";
			}
		}
	}
	if is_active(log_mtime, cpu_percent, now) { "active" } else { "idle" }
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

/// One scan pass: list tty PIDs, resolve the agent candidates among them, and
/// return the snapshot to emit. Rows are rebuilt from scratch every pass (a
/// PID gone between `ps` and the sysinfo refresh is simply skipped) and
/// sorted by pid so rows don't jump between passes.
pub fn scan(
	app: &AppHandle,
	codex_meta: &mut HashMap<PathBuf, Option<(String, String)>>,
) -> Vec<DiscoveredAgent> {
	let ps = ps_snapshot();
	let tty: HashMap<u32, &str> = ps
		.iter()
		.filter_map(|(&pid, p)| Some((pid, p.tty.as_deref()?)))
		.collect();
	if tty.is_empty() {
		return Vec::new();
	}

	// Ignored agent+cwd pairs, snapshotted under a short lock (like the port
	// radar's settings snapshot) before any resolution work.
	let ignored: Vec<crate::model::IgnoredAgent> = {
		let state = app.state::<AppState>();
		let cfg = state.config.lock().unwrap();
		cfg.settings.ignored_agents.clone()
	};

	let mut sys = System::new();
	let sys_pids: Vec<Pid> = tty.keys().map(|&p| Pid::from_u32(p)).collect();
	let refresh = ProcessRefreshKind::nothing()
		.with_cmd(UpdateKind::Always)
		.with_cwd(UpdateKind::Always)
		.with_environ(UpdateKind::Always) // supacode jump coordinates
		.with_cpu()
		.with_memory();
	sys.refresh_processes_specifics(ProcessesToUpdate::Some(&sys_pids), true, refresh);

	// Candidates: tty-attached agent processes with a resolvable cwd (macOS
	// can refuse cwd for privileged processes; those can't be named or given
	// an activity dir, so they're dropped rather than shown as junk rows).
	let candidates: Vec<(u32, &'static AgentDef, String)> = sys_pids
		.iter()
		.filter_map(|&pid| {
			let proc_ = sys.process(pid)?;
			let argv: Vec<String> =
				proc_.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect();
			let def = agent_from_argv(&argv)?;
			let cwd = proc_.cwd()?.to_string_lossy().into_owned();
			if ignored.iter().any(|i| i.agent == def.kind && i.cwd == cwd) {
				return None;
			}
			Some((pid.as_u32(), def, cwd))
		})
		.collect();
	if candidates.is_empty() {
		return Vec::new();
	}

	// Second targeted refresh so cpu_usage() has a valid delta to measure.
	std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
	let cand_pids: Vec<Pid> = candidates.iter().map(|&(p, _, _)| Pid::from_u32(p)).collect();
	sys.refresh_processes_specifics(ProcessesToUpdate::Some(&cand_pids), true, refresh);

	let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()));
	let now = SystemTime::now();

	// Hook-reported states (quay-hook, claude only today) — read once per pass;
	// same walk prunes files for sessions that are gone.
	let claude_cwds: Vec<&str> = candidates
		.iter()
		.filter(|(_, d, _)| d.kind == "claude")
		.map(|(_, _, cwd)| cwd.as_str())
		.collect();
	let hooks = {
		let dir = app.state::<AppState>().dir.join("agent-state");
		hook_states(&dir, &claude_cwds, now)
	};

	// Codex rollout inventory + name index, built once per pass and only when
	// a codex session is actually on screen. The index is re-read every pass —
	// thread names get (re)generated over time, so caching it would go stale.
	let has_codex = candidates.iter().any(|(_, d, _)| matches!(d.activity, Activity::CodexRollout));
	let (codex, codex_index) = if has_codex {
		(
			codex_rollouts(&home, codex_meta),
			std::fs::read_to_string(home.join(".codex/session_index.jsonl")).unwrap_or_default(),
		)
	} else {
		(Vec::new(), String::new())
	};

	let mut out: Vec<DiscoveredAgent> = candidates
		.into_iter()
		.filter_map(|(pid, def, cwd)| {
			let proc_ = sys.process(Pid::from_u32(pid))?;
			let cpu_percent = proc_.cpu_usage();
			let session = session_info(def, &home, &cwd, &codex, &codex_index);
			// Manifest name + stack, like the port radar. Re-read per pass —
			// a handful of agents × a few small manifest reads every 5 s.
			let name = detect::name_from_dir(Path::new(&cwd));
			let stack = detect::stack_from_dir(Path::new(&cwd)).map(str::to_string);
			let state = resolve_state(
				(def.kind == "claude").then(|| hooks.get(&cwd)).flatten(),
				session.mtime,
				cpu_percent,
				now,
			);
			Some(DiscoveredAgent {
				pid,
				agent: def.kind,
				name,
				cwd,
				stack,
				session_name: session.name,
				uptime_sec: proc_.run_time(),
				cpu_percent,
				memory_bytes: proc_.memory(),
				state,
				tty: tty.get(&pid).copied().unwrap_or_default().to_string(),
				jump_supported: jump_target_from_env(
					proc_.environ().iter().filter_map(|s| s.to_str()),
				)
				.is_some() || ancestry_target(pid, &ps).is_some(),
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
	fn slugs_match_verified_real_paths() {
		assert_eq!(
			claude_slug("/Users/abhi/DEV/abhi_github/abhi.page.11ty"),
			"-Users-abhi-DEV-abhi-github-abhi-page-11ty"
		);
		assert_eq!(
			pi_slug("/Users/abhi/DEV/eko_github/wlc-webapp"),
			"--Users-abhi-DEV-eko_github-wlc-webapp--"
		);
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
	fn newest_jsonl_picks_latest_jsonl_only() {
		let d = std::env::temp_dir().join(format!("msm-agent-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&d).unwrap();
		assert!(newest_jsonl(&d).is_none());
		std::fs::write(d.join("a.jsonl"), "x").unwrap();
		std::fs::write(d.join("ignored.txt"), "x").unwrap();
		let (path, mtime) = newest_jsonl(&d).unwrap();
		assert_eq!(path, d.join("a.jsonl"));
		assert_eq!(mtime, std::fs::metadata(d.join("a.jsonl")).unwrap().modified().unwrap());
		assert!(newest_jsonl(&d.join("missing")).is_none());
		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn first_user_prompt_finds_first_real_prompt() {
		let jsonl = concat!(
			"{\"type\":\"last-prompt\",\"leafUuid\":\"x\"}\n",
			"{\"type\":\"user\",\"isMeta\":true,\"message\":{\"content\":\"Caveat: meta\"}}\n",
			"{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"content\":\"out\"}]}}\n",
			"{\"type\":\"user\",\"message\":{\"content\":\"<command-name>/clear</command-name>\"}}\n",
			"{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Fix the login bug\"}]}}\n",
		);
		assert_eq!(first_user_prompt(jsonl).as_deref(), Some("Fix the login bug"));
		// String-content form and no-user-message form.
		assert_eq!(
			first_user_prompt("{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n").as_deref(),
			Some("hello")
		);
		assert_eq!(first_user_prompt("{\"type\":\"assistant\"}\nnot json\n"), None);
	}

	#[test]
	fn truncate_label_caps_at_80_chars() {
		let long = "x".repeat(100);
		let t = truncate_label(&long);
		assert_eq!(t.chars().count(), 81); // 80 + ellipsis
		assert!(t.ends_with('…'));
		assert_eq!(truncate_label("short"), "short");
	}

	#[test]
	fn rollout_meta_parses_session_meta_line() {
		let line = "{\"timestamp\":\"t\",\"type\":\"session_meta\",\"payload\":{\"session_id\":\"s1\",\"id\":\"id1\",\"cwd\":\"/x/proj\"}}";
		assert_eq!(rollout_meta(line), Some(("/x/proj".into(), "id1".into())));
		// Falls back to session_id when id is missing.
		let line2 = "{\"type\":\"session_meta\",\"payload\":{\"session_id\":\"s2\",\"cwd\":\"/y\"}}";
		assert_eq!(rollout_meta(line2), Some(("/y".into(), "s2".into())));
		assert_eq!(rollout_meta("{\"type\":\"response_item\"}"), None);
		assert_eq!(rollout_meta("garbage"), None);
	}

	#[test]
	fn thread_name_for_takes_last_match() {
		let index = concat!(
			"{\"id\":\"a\",\"thread_name\":\"old name\",\"updated_at\":\"t1\"}\n",
			"{\"id\":\"b\",\"thread_name\":\"other\",\"updated_at\":\"t2\"}\n",
			"{\"id\":\"a\",\"thread_name\":\"new name\",\"updated_at\":\"t3\"}\n",
		);
		assert_eq!(thread_name_for(index, "a").as_deref(), Some("new name"));
		assert_eq!(thread_name_for(index, "b").as_deref(), Some("other"));
		assert_eq!(thread_name_for(index, "missing"), None);
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

	#[test]
	fn resolve_state_waiting_wins_unless_log_resumed() {
		let now = SystemTime::now();
		let hook_ts = now - std::time::Duration::from_secs(30);
		let waiting = HookState { state: "waiting".into(), ts: hook_ts };
		let working = HookState { state: "working".into(), ts: hook_ts };
		// Waiting holds with no log write, or a write from before the event.
		assert_eq!(resolve_state(Some(&waiting), None, 0.0, now), "waiting");
		let before = hook_ts - std::time::Duration::from_secs(10);
		assert_eq!(resolve_state(Some(&waiting), Some(before), 0.0, now), "waiting");
		// A log write after the event means the session resumed → heuristic
		// (5 s old write → active).
		let after = hook_ts + std::time::Duration::from_secs(25);
		assert_eq!(resolve_state(Some(&waiting), Some(after), 0.0, now), "active");
		// Non-waiting hook states defer to the heuristic entirely.
		assert_eq!(resolve_state(Some(&working), None, 0.0, now), "idle");
		assert_eq!(resolve_state(None, None, 50.0, now), "active");
		assert_eq!(resolve_state(None, None, 0.0, now), "idle");
	}

	#[test]
	fn hook_states_precedence_pruning_and_corrupt_files() {
		let d = std::env::temp_dir().join(format!("msm-hook-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&d).unwrap();
		let now = SystemTime::now();
		let ts = now.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
		let write = |name: &str, cwd: &str, state: &str, ts: u64| {
			std::fs::write(
				d.join(name),
				format!("{{\"cwd\":\"{cwd}\",\"state\":\"{state}\",\"ts\":{ts}}}"),
			)
			.unwrap();
		};
		// Two sessions in one cwd: waiting must win over a newer idle.
		write("s1.json", "/x/app", "waiting", ts - 60);
		write("s2.json", "/x/app", "idle", ts);
		// Live cwd → kept even when the dead-cwd grace has passed.
		write("s3.json", "/y/live", "working", ts - HOOK_PRUNE_GRACE_SECS - 100);
		// Dead cwd: young file kept on disk (grace), old file pruned.
		write("s4.json", "/z/dead-young", "idle", ts);
		write("s5.json", "/z/dead-old", "idle", ts - HOOK_PRUNE_GRACE_SECS - 100);
		std::fs::write(d.join("s6.json"), "{ partial garb").unwrap();

		let map = hook_states(&d, &["/x/app", "/y/live"], now);
		assert_eq!(map["/x/app"].state, "waiting");
		assert_eq!(map["/y/live"].state, "working");
		assert!(!map.contains_key("/z/dead-young")); // not live → not in map
		assert!(d.join("s4.json").exists()); // …but still on disk (grace)
		assert!(!d.join("s5.json").exists()); // pruned
		assert!(!d.join("s6.json").exists()); // corrupt → deleted
		assert!(hook_states(&d.join("missing"), &[], now).is_empty());
		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn is_active_on_recent_write_or_busy_cpu() {
		let now = SystemTime::now();
		let fresh = now - std::time::Duration::from_secs(5);
		let stale = now - std::time::Duration::from_secs(60);
		assert!(is_active(Some(fresh), 0.0, now));
		assert!(!is_active(Some(stale), 0.0, now));
		assert!(is_active(None, 15.0, now));
		assert!(!is_active(None, 5.0, now));
	}
}

