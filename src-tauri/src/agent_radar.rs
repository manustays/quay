//! Discover interactive terminal AI-agent sessions ("agent radar").
//!
//! An interactive session = an agent binary (claude / codex / opencode / pi)
//! with an attached tty. sysinfo does not expose the controlling tty on macOS,
//! so each pass starts with one `ps -axo pid=,tty=` to collect tty-attached
//! PIDs, then resolves argv/cwd/cpu/mem for just those via two targeted
//! sysinfo refreshes (`MINIMUM_CPU_UPDATE_INTERVAL` apart, so CPU% is a valid
//! delta). Runs inside the port-radar loop (scanner.rs): 5 s cadence, only
//! while the popover is visible; snapshots go out on `agents_discovered`.

use crate::state::AppState;
use serde::Serialize;
use std::collections::HashSet;
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
	/// Display name: the cwd basename (project folder).
	pub name: String,
	pub cwd: String,
	pub uptime_sec: u64,
	/// Per-PID CPU% (may exceed 100 on multi-core).
	// ponytail: per-PID only, no child-subtree sum; metrics::aggregate_tree is
	// the upgrade if the numbers look too small.
	pub cpu_percent: f32,
	pub memory_bytes: u64,
	/// "active" (recent session-log write or busy CPU) or "idle" — a
	/// recent-activity signal, not proof of work.
	pub state: &'static str,
}

/// How a session's active/idle state is derived.
enum Activity {
	/// Newest `.jsonl` mtime under `~/.claude/projects/<claude_slug(cwd)>/`.
	ClaudeJsonl,
	/// Newest `.jsonl` mtime under `~/.pi/agent/sessions/<pi_slug(cwd)>/`.
	PiJsonl,
	/// CPU-only heuristic (no per-cwd session dir known: codex's
	/// session_index.jsonl has no cwd, opencode stores sessions in sqlite).
	// ponytail: `lsof -p <pid>` mapping the open rollout file/db is the upgrade.
	Cpu,
}

struct AgentDef {
	kind: &'static str,
	bin: &'static str,
	activity: Activity,
}

const AGENTS: &[AgentDef] = &[
	AgentDef { kind: "claude", bin: "claude", activity: Activity::ClaudeJsonl },
	AgentDef { kind: "codex", bin: "codex", activity: Activity::Cpu },
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

/// Parse `ps -axo pid=,tty=` output into the set of tty-attached PIDs.
/// A tty of `??` (daemons, helpers, .app bundles) is dropped. Pure.
fn parse_tty_pids(out: &str) -> HashSet<u32> {
	out.lines()
		.filter_map(|line| {
			let mut cols = line.split_whitespace();
			let pid = cols.next()?.parse().ok()?;
			cols.next()?.starts_with("tty").then_some(pid)
		})
		.collect()
}

/// tty-attached PIDs right now — one `ps` per scan pass.
fn tty_pids() -> HashSet<u32> {
	std::process::Command::new("ps")
		.args(["-axo", "pid=,tty="])
		.output()
		.map(|o| parse_tty_pids(&String::from_utf8_lossy(&o.stdout)))
		.unwrap_or_default()
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

/// The session-log dir for an agent in `cwd`, or None for CPU-only agents.
fn session_dir(activity: &Activity, home: &Path, cwd: &str) -> Option<PathBuf> {
	match activity {
		Activity::ClaudeJsonl => Some(home.join(".claude/projects").join(claude_slug(cwd))),
		Activity::PiJsonl => Some(home.join(".pi/agent/sessions").join(pi_slug(cwd))),
		Activity::Cpu => None,
	}
}

/// Newest `.jsonl` mtime in `dir` (non-recursive). None when the dir is
/// missing or holds no `.jsonl` — the caller degrades to the CPU signal.
fn newest_mtime(dir: &Path) -> Option<SystemTime> {
	std::fs::read_dir(dir)
		.ok()?
		.flatten()
		.filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
		.filter_map(|e| e.metadata().ok()?.modified().ok())
		.max()
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
pub fn scan(app: &AppHandle) -> Vec<DiscoveredAgent> {
	let tty = tty_pids();
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
	let sys_pids: Vec<Pid> = tty.iter().map(|&p| Pid::from_u32(p)).collect();
	let refresh = ProcessRefreshKind::nothing()
		.with_cmd(UpdateKind::Always)
		.with_cwd(UpdateKind::Always)
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
	let mut out: Vec<DiscoveredAgent> = candidates
		.into_iter()
		.filter_map(|(pid, def, cwd)| {
			let proc_ = sys.process(Pid::from_u32(pid))?;
			let cpu_percent = proc_.cpu_usage();
			let log_mtime =
				session_dir(&def.activity, &home, &cwd).and_then(|d| newest_mtime(&d));
			let name = Path::new(&cwd)
				.file_name()
				.map(|s| s.to_string_lossy().into_owned())
				.unwrap_or_else(|| def.kind.to_string());
			Some(DiscoveredAgent {
				pid,
				agent: def.kind,
				name,
				cwd,
				uptime_sec: proc_.run_time(),
				cpu_percent,
				memory_bytes: proc_.memory(),
				state: if is_active(log_mtime, cpu_percent, now) { "active" } else { "idle" },
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
	fn parse_tty_pids_keeps_only_tty_attached() {
		assert_eq!(parse_tty_pids("123 ttys002\n456 ??\n789 ttys013\n"), HashSet::from([123, 789]));
		assert_eq!(parse_tty_pids("garbage line\n"), HashSet::new());
	}

	#[test]
	fn newest_mtime_picks_latest_jsonl_only() {
		let d = std::env::temp_dir().join(format!("msm-agent-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&d).unwrap();
		assert_eq!(newest_mtime(&d), None);
		std::fs::write(d.join("a.jsonl"), "x").unwrap();
		std::fs::write(d.join("ignored.txt"), "x").unwrap();
		let m = newest_mtime(&d).unwrap();
		assert_eq!(m, std::fs::metadata(d.join("a.jsonl")).unwrap().modified().unwrap());
		assert_eq!(newest_mtime(&d.join("missing")), None);
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
