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
	/// False for listeners that must not be adopted as project services
	/// (currently Docker Desktop's port proxies — manage those via a
	/// Docker-kind service instead).
	pub adoptable: bool,
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
	adoptable: bool,
}

/// Well-known non-dev listeners hidden from the radar. Matched as a
/// case-insensitive prefix of the process name.
// ponytail: static denylist; a settings toggle only if noise reports come in
const NAME_DENYLIST: &[&str] = &["rapportd", "controlcenter", "sharingd", "spotify", "dropbox"];

/// Join argv into a shell-pasteable command. An element stays bare only when
/// every character is from a known-safe set; anything else (spaces, quotes,
/// `$`, `;`, backticks, …) is single-quoted, with embedded single quotes
/// escaped as `'\''` — so an apostrophe in a path can't unbalance the command.
fn shell_join(argv: &[String]) -> String {
	fn is_safe(a: &str) -> bool {
		!a.is_empty()
			&& a.chars().all(|c| c.is_ascii_alphanumeric() || "_-./:=@%+,".contains(c))
	}
	argv.iter()
		.map(|a| {
			if is_safe(a) {
				a.clone()
			} else {
				format!("'{}'", a.replace('\'', r"'\''"))
			}
		})
		.collect::<Vec<_>>()
		.join(" ")
}

/// True when `pid`'s parent chain reaches a tracked (managed) root PID.
///
/// A managed service usually listens via a *descendant* of the PID we track
/// (`zsh -lc` wrapper → `npm` → `node`), so exact-PID exclusion isn't enough.
/// Parents are refreshed into `sys` on demand, one hop at a time; the walk is
/// capped so a pathological parent cycle can't spin.
fn has_tracked_ancestor(sys: &mut System, pid: u32, tracked: &HashSet<u32>) -> bool {
	if tracked.is_empty() {
		return false;
	}
	let mut cur = pid;
	for _ in 0..16 {
		let sys_pid = Pid::from_u32(cur);
		if sys.process(sys_pid).is_none() {
			sys.refresh_processes_specifics(
				ProcessesToUpdate::Some(&[sys_pid]),
				true,
				ProcessRefreshKind::nothing(),
			);
		}
		let Some(ppid) = sys.process(sys_pid).and_then(|p| p.parent()).map(|p| p.as_u32())
		else {
			return false;
		};
		if tracked.contains(&ppid) {
			return true;
		}
		if ppid <= 1 {
			return false;
		}
		cur = ppid;
	}
	false
}

/// Resolve argv/cwd/stack for `pids` with one targeted `sysinfo` refresh.
fn resolve(sys: &mut System, pids: &[u32]) -> HashMap<u32, Resolved> {
	if pids.is_empty() {
		return HashMap::new();
	}
	let sys_pids: Vec<Pid> = pids.iter().map(|&p| Pid::from_u32(p)).collect();
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
		// Prefer the project's manifest name (package.json / Cargo.toml), falling
		// back to the cwd basename, then the process name for a "/" or missing cwd.
		// Read once per new PID — resolve() only runs for PIDs not already cached.
		let name = cwd
			.as_deref()
			.filter(|c| *c != "/") // a cwd of "/" is not a project folder
			.map(|c| detect::name_from_dir(Path::new(c)))
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
			Resolved { name, command: shell_join(&argv), cwd, stack, adoptable: !is_docker_proxy },
		);
	}
	out
}

/// One scan pass: list listeners, filter, resolve new PIDs via `cache`, and
/// return the snapshot to emit.
fn scan(app: &AppHandle, cache: &mut HashMap<u32, Resolved>) -> Vec<DiscoveredPort> {
	let listeners = crate::supervisor::listeners();

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

	// One System shared by the ancestor walks and the argv/cwd resolution.
	let mut sys = System::new();
	let candidates: Vec<(u16, u32)> = listeners
		.into_iter()
		.filter(|&(port, pid)| {
			pid != own_pid
				&& !tracked_pids.contains(&pid)
				&& port >= 1024
				&& !ignored_ports.contains(&port)
		})
		// Drop descendants of tracked PIDs: they are our own managed services
		// (the listener is usually a child of the tracked shell wrapper), and
		// offering a Kill button for them invites self-inflicted outages.
		.filter(|&(_, pid)| !has_tracked_ancestor(&mut sys, pid, &tracked_pids))
		.collect();

	// Resolve only PIDs we haven't seen; evict cache entries for gone PIDs.
	let live: HashSet<u32> = candidates.iter().map(|&(_, pid)| pid).collect();
	cache.retain(|pid, _| live.contains(pid));
	let new_pids: Vec<u32> =
		live.iter().copied().filter(|pid| !cache.contains_key(pid)).collect();
	cache.extend(resolve(&mut sys, &new_pids));

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
				adoptable: r.adoptable,
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
	fn shell_join_quotes_spaces() {
		let argv = vec!["node".to_string(), "my server.js".to_string()];
		assert_eq!(shell_join(&argv), "node 'my server.js'");
		assert_eq!(shell_join(&["vite".to_string()]), "vite");
	}

	#[test]
	fn shell_join_quotes_metachars_and_apostrophes() {
		// An apostrophe (no spaces) must still be quoted, and quoted balanced.
		assert_eq!(
			shell_join(&["node".to_string(), "/u/bob's-app/server.js".to_string()]),
			r"node '/u/bob'\''s-app/server.js'"
		);
		// Shell metacharacters can't pass through bare.
		assert_eq!(shell_join(&["echo".to_string(), "$HOME;ls".to_string()]), "echo '$HOME;ls'");
		// Plain paths and flags stay readable.
		assert_eq!(
			shell_join(&["/usr/bin/python3".to_string(), "-m".to_string(), "http.server".to_string()]),
			"/usr/bin/python3 -m http.server"
		);
	}
}
