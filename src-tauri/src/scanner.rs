//! Discover unmanaged processes listening on local TCP ports ("port radar").
//!
//! Like the metrics loop, scanning is gated on popover visibility: one `lsof`
//! pass every 5 s while the popover is open, nothing while it's hidden. Each
//! listener PID is resolved (argv, cwd, stack) at most once via a per-loop
//! cache; results are pushed to the frontend as a full snapshot per pass on
//! the `ports_discovered` event.

use crate::detect;
use crate::state::AppState;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tauri::{AppHandle, Emitter, Manager};

/// One discovered TCP listener, pushed to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredPort {
	pub port: u16,
	pub pid: u32,
	/// Display name: cwd basename when resolvable, else the process name.
	pub name: String,
	/// Shell-quoted argv — the adopt form's start-command prefill.
	pub command: String,
	pub cwd: Option<String>,
	pub stack: Option<String>,
	/// Set when the port belongs to a registered item (collision indicator);
	/// such entries are badges on existing rows, not adoptable listeners.
	#[serde(rename = "managedItemId")]
	pub managed_item_id: Option<String>,
}

/// What one PID resolves to, cached per scan loop so each new PID is looked
/// up in `sysinfo` exactly once.
#[derive(Debug, Clone)]
struct Resolved {
	name: String,
	command: String,
	cwd: Option<String>,
	stack: Option<String>,
}

/// Well-known non-dev listeners hidden from the radar. Matched as a
/// case-insensitive prefix of the process name.
// ponytail: static denylist; a settings toggle only if noise reports come in
const NAME_DENYLIST: &[&str] = &["rapportd", "controlcenter", "sharingd", "spotify", "dropbox"];

/// Parse `lsof -Fpn` field output into unique `(port, pid)` pairs. Pure.
///
/// The format is one field per line: `p<pid>` starts a process section, each
/// `n<addr>` names a socket (e.g. `n*:3000`, `n127.0.0.1:5173`, `n[::1]:8080`).
/// The port is whatever follows the last `:`. Garbage lines are skipped.
pub fn parse_lsof_fields(out: &str) -> Vec<(u16, u32)> {
	let mut pairs: Vec<(u16, u32)> = Vec::new();
	let mut seen: HashSet<(u16, u32)> = HashSet::new();
	let mut pid: Option<u32> = None;
	for line in out.lines() {
		match line.as_bytes().first() {
			Some(b'p') => pid = line[1..].trim().parse().ok(),
			Some(b'n') => {
				let Some(pid) = pid else { continue };
				let Some(port) = line.rsplit(':').next().and_then(|p| p.trim().parse().ok())
				else {
					continue;
				};
				if seen.insert((port, pid)) {
					pairs.push((port, pid));
				}
			}
			_ => {}
		}
	}
	pairs
}

/// All `(port, pid)` TCP listeners owned by the current user, via one `lsof`.
///
/// `-u <uid>` restricts to our own processes — foreign-user listeners can't be
/// resolved (cwd/argv) or signalled anyway, and skipping them avoids the
/// Full Disk Access prompt entirely. Empty when `lsof` is missing or fails.
pub fn scan_listeners() -> Vec<(u16, u32)> {
	let uid = unsafe { libc::getuid() }.to_string();
	let Ok(out) = Command::new("lsof")
		.args(["-iTCP", "-sTCP:LISTEN", "-P", "-n", "-a", "-u", &uid, "-Fpn"])
		.output()
	else {
		return vec![];
	};
	parse_lsof_fields(&String::from_utf8_lossy(&out.stdout))
}

/// Join argv into a shell-pasteable command, quoting elements with spaces.
fn shell_join(argv: &[String]) -> String {
	argv.iter()
		.map(|a| {
			if a.contains(' ') || a.contains('"') {
				format!("'{}'", a.replace('\'', r"'\''"))
			} else {
				a.clone()
			}
		})
		.collect::<Vec<_>>()
		.join(" ")
}

/// Resolve argv/cwd/stack for `pids` with one targeted `sysinfo` refresh.
fn resolve(pids: &[u32]) -> HashMap<u32, Resolved> {
	if pids.is_empty() {
		return HashMap::new();
	}
	let sys_pids: Vec<Pid> = pids.iter().map(|&p| Pid::from_u32(p)).collect();
	let mut sys = System::new();
	sys.refresh_processes_specifics(
		ProcessesToUpdate::Some(&sys_pids),
		true,
		ProcessRefreshKind::nothing()
			.with_cmd(UpdateKind::Always)
			.with_cwd(UpdateKind::Always),
	);
	let mut out = HashMap::new();
	for &pid in pids {
		let Some(proc_) = sys.process(Pid::from_u32(pid)) else { continue };
		let argv: Vec<String> =
			proc_.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect();
		let proc_name = proc_.name().to_string_lossy().into_owned();
		let cwd = proc_.cwd().map(|p| p.to_string_lossy().into_owned());
		let name = cwd
			.as_deref()
			.and_then(|c| Path::new(c).file_name())
			.map(|n| n.to_string_lossy().into_owned())
			.filter(|n| n != "/") // a cwd of "/" is not a project folder
			.unwrap_or_else(|| proc_name.clone());
		// Docker Desktop's host-side proxies own published container ports; tag
		// them as "docker" so the UI can label them and disable adoption.
		let is_docker_proxy =
			proc_name.to_lowercase().starts_with("com.docker") || proc_name.contains("vpnkit");
		let stack = if is_docker_proxy {
			Some("docker".to_string())
		} else {
			detect::stack_from_argv(&argv)
				.or_else(|| cwd.as_deref().and_then(|c| detect::stack_from_dir(Path::new(c))))
				.map(str::to_string)
		};
		out.insert(
			pid,
			Resolved { name, command: shell_join(&argv), cwd, stack },
		);
	}
	out
}

/// One scan pass: list listeners, filter, resolve new PIDs via `cache`, and
/// return the snapshot to emit.
fn scan(app: &AppHandle, cache: &mut HashMap<u32, Resolved>) -> Vec<DiscoveredPort> {
	let listeners = scan_listeners();

	// Snapshot config/state under short locks before any resolution work.
	let (managed_ports, ignored_ports, tracked_pids) = {
		let state = app.state::<AppState>();
		let cfg = state.config.lock().unwrap();
		let managed: HashMap<u16, String> = cfg
			.items
			.iter()
			.filter_map(|i| i.port.map(|p| (p, i.id.clone())))
			.collect();
		let ignored: HashSet<u16> = cfg.settings.ignored_ports.iter().copied().collect();
		let tracked: HashSet<u32> =
			state.running.lock().unwrap().values().map(|r| r.pid).collect();
		(managed, ignored, tracked)
	};
	let own_pid = std::process::id();

	let candidates: Vec<(u16, u32)> = listeners
		.into_iter()
		.filter(|&(port, pid)| {
			pid != own_pid
				&& !tracked_pids.contains(&pid)
				&& port >= 1024
				&& !ignored_ports.contains(&port)
		})
		.collect();

	// Resolve only PIDs we haven't seen; evict cache entries for gone PIDs.
	let live: HashSet<u32> = candidates.iter().map(|&(_, pid)| pid).collect();
	cache.retain(|pid, _| live.contains(pid));
	let new_pids: Vec<u32> =
		live.iter().copied().filter(|pid| !cache.contains_key(pid)).collect();
	cache.extend(resolve(&new_pids));

	let mut out: Vec<DiscoveredPort> = candidates
		.into_iter()
		.filter_map(|(port, pid)| {
			let r = cache.get(&pid)?;
			let lower = r.name.to_lowercase();
			if NAME_DENYLIST.iter().any(|d| lower.starts_with(d)) {
				return None;
			}
			Some(DiscoveredPort {
				port,
				pid,
				name: r.name.clone(),
				command: r.command.clone(),
				cwd: r.cwd.clone(),
				stack: r.stack.clone(),
				managed_item_id: managed_ports.get(&port).cloned(),
			})
		})
		.collect();
	out.sort_by_key(|d| d.port);
	out
}

/// Spawn the visibility-gated radar loop: idle-tick 500 ms while the popover
/// is hidden, scan + emit `ports_discovered` every 5 s while it's open.
pub fn spawn_scan_loop(app: AppHandle) {
	std::thread::spawn(move || {
		let mut cache: HashMap<u32, Resolved> = HashMap::new();
		loop {
			let visible = app.state::<AppState>().visible.load(Ordering::Relaxed);
			if !visible {
				std::thread::sleep(Duration::from_millis(500));
				continue;
			}
			let discovered = scan(&app, &mut cache);
			if app.state::<AppState>().visible.load(Ordering::Relaxed) {
				let _ = app.emit("ports_discovered", &discovered);
			}
			// ponytail: hardcoded 5 s cadence; a settings knob only if asked for
			std::thread::sleep(Duration::from_secs(5));
		}
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parse_lsof_fields_handles_addr_shapes() {
		let out = "p123\nn*:3000\nn127.0.0.1:5173\np456\nn[::1]:8080\nn[::]:8080\n";
		assert_eq!(
			parse_lsof_fields(out),
			vec![(3000, 123), (5173, 123), (8080, 456)]
		);
	}

	#[test]
	fn parse_lsof_fields_dedupes_and_skips_garbage() {
		// v4+v6 listeners on the same port dedupe to one pair; f-lines, blank
		// lines, a port-less name, and an n-line before any p-line are skipped.
		let out = "nno-pid-yet:99\np12\nf34\nnlocalhost:3000\nn[::1]:3000\n\nnbadport:\n";
		assert_eq!(parse_lsof_fields(out), vec![(3000, 12)]);
		assert_eq!(parse_lsof_fields(""), Vec::<(u16, u32)>::new());
	}

	#[test]
	fn shell_join_quotes_spaces() {
		let argv = vec!["node".to_string(), "my server.js".to_string()];
		assert_eq!(shell_join(&argv), "node 'my server.js'");
		assert_eq!(shell_join(&["vite".to_string()]), "vite");
	}
}
