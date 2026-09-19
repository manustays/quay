use crate::model::Status;
use std::collections::HashMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Inputs for a status decision.
pub struct Probe {
	pub pid_alive: bool,
	pub has_port: bool,
	pub port_open: bool,
}

/// Decide a background item's status from a probe. Pure.
pub fn decide_status(p: &Probe) -> Status {
	if !p.pid_alive { return Status::Error; }
	if !p.has_port { return Status::Running; }
	if p.port_open { Status::Running } else { Status::Starting }
}

/// Decide a terminal item's status. Pure.
///
/// `pid_alive` is `Some` when we captured the terminal's shell PID at start:
/// `Some(true)` while the window is open, `Some(false)` once it's closed. `None`
/// means we have no PID (capture failed, or a legacy/untracked item), in which case
/// we fall back to the old port-or-last-known behavior. A closed terminal is a
/// normal `Stopped`, not an error.
pub fn terminal_status(
	pid_alive: Option<bool>,
	has_port: bool,
	port_open: bool,
	current: Option<Status>,
) -> Status {
	match pid_alive {
		Some(true) => {
			if has_port {
				if port_open { Status::Running } else { Status::Starting }
			} else {
				Status::Running
			}
		}
		Some(false) => Status::Stopped,
		None => {
			if has_port {
				if port_open { Status::Running } else { Status::Starting }
			} else {
				current.unwrap_or(Status::Stopped)
			}
		}
	}
}

/// Decide a `command`-kind item's status from a port probe. Pure.
///
/// The daemon is not owned, so the configured port is the only liveness signal:
/// - port up → `Running`.
/// - port down while `current == Starting` → still coming up after an explicit
///   Start; stay `Starting`.
/// - port down otherwise (idle, or a previously-Running service that dropped) →
///   `Stopped`.
///
/// Mirrors how [`terminal_status`] leans on `current` to disambiguate. A start
/// command that exits 0 but never binds the port stays `Starting` — the
/// nonzero-exit error path in `start_item` catches the common start failures.
pub fn command_status(port_up: bool, current: Option<Status>) -> Status {
	if port_up {
		Status::Running
	} else if current == Some(Status::Starting) {
		Status::Starting
	} else {
		Status::Stopped
	}
}

/// Aggregate all item statuses into a tray attention state. Pure.
///
/// Precedence: any `Error` > any `Starting` > `None` (nominal). `Running` and
/// `Stopped` are nominal for the tray, and so is an empty set (no items).
pub fn aggregate_status(statuses: impl Iterator<Item = Status>) -> Option<Status> {
	let mut any_starting = false;
	for status in statuses {
		match status {
			Status::Error => return Some(Status::Error),
			Status::Starting => any_starting = true,
			Status::Running | Status::Stopped => {}
		}
	}
	any_starting.then_some(Status::Starting)
}

/// Last `n` lines of a file, reading at most the trailing 64 KiB so a
/// multi-GB log is never slurped whole. Empty string when the file is
/// missing or unreadable. Shared by [`exit_error`] and the `tail_log` command.
pub fn tail_lines(path: &std::path::Path, n: usize) -> String {
	use std::io::{Read, Seek, SeekFrom};
	// ponytail: 64 KiB cap — plenty for any on-screen tail; no rotation handling
	const CAP: u64 = 64 * 1024;
	let Ok(mut file) = std::fs::File::open(path) else { return String::new() };
	let len = file.metadata().map(|m| m.len()).unwrap_or(0);
	if len > CAP {
		let _ = file.seek(SeekFrom::End(-(CAP as i64)));
	}
	let mut bytes = Vec::new();
	if file.read_to_end(&mut bytes).is_err() {
		return String::new();
	}
	let text = String::from_utf8_lossy(&bytes);
	let tail: Vec<&str> = text.lines().rev().take(n).collect();
	tail.into_iter().rev().collect::<Vec<_>>().join("\n")
}

/// Build the error message for an exited process: the exit code when known
/// (owned children; signal deaths and adopted PIDs have none) plus the last
/// few log lines so the cause is visible without opening the log.
pub fn exit_error(exit_code: Option<i32>, log_path: &std::path::Path) -> String {
	let mut msg = match exit_code {
		Some(code) => format!("process exited with code {code}"),
		None => "process exited".to_string(),
	};
	let tail = tail_lines(log_path, 3);
	if !tail.is_empty() {
		msg = format!("{msg}\n{tail}");
	}
	msg
}

/// True if a TCP connection to 127.0.0.1:port succeeds within 300ms.
pub fn port_open(port: u16) -> bool {
	let Ok(mut addrs) = format!("127.0.0.1:{port}").to_socket_addrs() else { return false; };
	addrs.next().map(|a| TcpStream::connect_timeout(&a, Duration::from_millis(300)).is_ok()).unwrap_or(false)
}

/// True if an HTTP GET to the port+path returns a 2xx.
///
/// Note: uses ureq v3 API — timeout is set via `.config().timeout_global(Some(...)).build()`
/// and `response.status()` returns `http::StatusCode` (`.as_u16()` needed), unlike ureq v2.
pub fn http_ok(port: u16, path: &str) -> bool {
	let url = format!("http://127.0.0.1:{port}{path}");
	match ureq::get(&url)
		.config()
		.timeout_global(Some(Duration::from_millis(500)))
		.build()
		.call()
	{
		Ok(response) => response.status().as_u16() < 300,
		Err(_) => false,
	}
}

// ── Poll loop ────────────────────────────────────────────────────────────────

use crate::commands::set_status;
use crate::model::{ItemKind, RunMode};
use crate::state::AppState;
use tauri::{AppHandle, Manager};

/// Spawn a background thread that calls `poll_once` every `poll_interval_sec` seconds
/// for as long as a human could see the result.
///
/// This is the app's energy floor: the metrics and radar loops park when the popover
/// closes, but status and the tray badge have to stay fresh while the tray is on
/// screen. They are *not* on screen when every display is asleep or the Mac is
/// locked, so the loop parks on the same condvar the other two use — see
/// [`AppState::wait_awake`]. Parked, it costs no wakeups at all.
///
/// Gating the next iteration does not cancel a pass already in flight, so what this
/// buys is eventual quiescence, not an instant stop.
pub fn spawn_poll_loop(app: AppHandle) {
	std::thread::spawn(move || loop {
		// Park first: on resume the fresh status pass and badge refresh are the very
		// first thing that runs, so the tray is right by the time it is looked at.
		let generation = app.state::<AppState>().wait_awake();
		let interval = {
			let st = app.state::<AppState>();
			let secs = st.config.lock().unwrap().settings.poll_interval_sec.max(1);
			secs
		};
		poll_once(&app);
		// Always-on: refresh the waiting-agent menubar signal even while the popover
		// (and its heavier radar scan) is closed. `false`: the `ps`-forking orphan
		// sweep is rate-limited here, not run every tick.
		crate::refresh_waiting_badge(&app, false);
		app.state::<AppState>()
			.wait_awake_interval(generation, std::time::Duration::from_secs(interval));
	});
}

/// One poll pass: compute each item's live status and call `set_status` on changes.
///
/// Skips non-brew items whose status is None or Stopped (never started).
pub fn poll_once(app: &AppHandle) {
	let state = app.state::<AppState>();
	let items = state.config.lock().unwrap().items.clone();
	// One `brew services list` / `docker ps -a` for the whole pass instead of one
	// fork per item — the batching `metrics::collect` already does for `launchctl`
	// and `lsof`. Built only when an item of that kind exists, so a config without
	// one forks nothing. A failed spawn yields an empty map, which lands on the same
	// `Stopped` fallback `brew_status`/`docker_status` use.
	let brew_map: HashMap<String, Status> = if items
		.iter()
		.any(|i| matches!(i.kind, ItemKind::Brew) && i.brew_formula.is_some())
	{
		crate::brew::services_list_raw()
			.map(|t| crate::brew::parse_brew_list(&t))
			.unwrap_or_default()
	} else {
		HashMap::new()
	};
	let docker_map: HashMap<String, Status> = if items
		.iter()
		.any(|i| matches!(i.kind, ItemKind::Docker) && i.container_name.is_some())
	{
		crate::docker::ps_raw()
			.map(|t| crate::docker::parse_docker_ps(&t))
			.unwrap_or_default()
	} else {
		HashMap::new()
	};
	for item in items {
		let current = state.statuses.lock().unwrap().get(&item.id).copied();
		// Brew + Docker + Command are polled even when Stopped: their state lives
		// outside the app (launchctl / `docker ps` / a detached daemon on a port),
		// so a service started or stopped elsewhere is still reflected.
		if matches!(current, None | Some(Status::Stopped))
			&& !matches!(item.kind, ItemKind::Brew | ItemKind::Docker | ItemKind::Command)
		{
			continue; // never started; leave as-is
		}
		let status = match item.kind {
			ItemKind::Brew => {
				item.brew_formula.as_deref()
					.and_then(|f| brew_map.get(f).copied())
					.unwrap_or(Status::Stopped)
			}
			ItemKind::Docker => {
				item.container_name.as_deref()
					.and_then(|n| docker_map.get(n).copied())
					.unwrap_or(Status::Stopped)
			}
			ItemKind::Command => {
				// Not owned — the configured port is the only liveness signal.
				// Same probe as a background item (HTTP when health_path is set,
				// else a TCP connect). Portless command items have no probe source,
				// so they keep whatever start/stop last set.
				match item.port {
					Some(p) => {
						let port_up = match item.health_path.as_deref() {
							Some(path) => http_ok(p, path),
							None => port_open(p),
						};
						command_status(port_up, current)
					}
					None => current.unwrap_or(Status::Stopped),
				}
			}
			_ => match item.run_mode {
				RunMode::Background => {
					// Check liveness while holding the lock briefly, then release before blocking I/O.
					let probed: Option<(bool, Option<i32>)> = {
						let mut running = state.running.lock().unwrap();
						running.get_mut(&item.id).map(|r| {
							(crate::supervisor::is_alive(r), crate::supervisor::exit_code(r))
						})
					};
					match probed {
						None => Status::Stopped,
						Some((alive, exit_code)) => {
							// Build the message once, on the alive→dead transition (the dead
							// entry stays in `running` and would otherwise re-read the log
							// every tick), and do the file read outside the errors lock.
							if !alive && !state.errors.lock().unwrap().contains_key(&item.id) {
								let msg = exit_error(exit_code, &state.log_path(&item.id));
								state.errors.lock().unwrap().insert(item.id.clone(), msg);
							}
							let has_port = item.port.is_some();
							// Port/HTTP checks happen outside any lock (can block up to 500 ms).
							let port_up = match (item.port, item.health_path.as_deref()) {
								(Some(p), Some(path)) => http_ok(p, path),
								(Some(p), None) => port_open(p),
								_ => false,
							};
							decide_status(&Probe { pid_alive: alive, has_port, port_open: port_up })
						}
					}
				}
				RunMode::Terminal => {
					// Liveness of the captured terminal-shell PID, if we have one.
					let pid_alive: Option<bool> = {
						let mut running = state.running.lock().unwrap();
						running.get_mut(&item.id).map(|r| crate::supervisor::is_alive(r))
					};
					let port_up = match item.port {
						Some(p) => port_open(p),
						None => false,
					};
					let s = terminal_status(pid_alive, item.port.is_some(), port_up, current);
					// The window closed: drop the dead entry (scope the lock, then
					// persist outside it — persist_pids locks `running` too).
					if matches!(pid_alive, Some(false)) {
						let removed = { state.running.lock().unwrap().remove(&item.id).is_some() };
						if removed {
							crate::commands::persist_pids(&state);
						}
					}
					s
				}
			},
		};
		set_status(app, &item.id, status);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::model::Status;

	#[test]
	fn dead_pid_is_error() {
		assert_eq!(decide_status(&Probe { pid_alive: false, has_port: true, port_open: false }), Status::Error);
	}
	#[test]
	fn alive_no_port_is_running() {
		assert_eq!(decide_status(&Probe { pid_alive: true, has_port: false, port_open: false }), Status::Running);
	}
	#[test]
	fn alive_port_open_is_running() {
		assert_eq!(decide_status(&Probe { pid_alive: true, has_port: true, port_open: true }), Status::Running);
	}
	#[test]
	fn alive_port_closed_is_starting() {
		assert_eq!(decide_status(&Probe { pid_alive: true, has_port: true, port_open: false }), Status::Starting);
	}

	#[test]
	fn command_port_up_is_running() {
		assert_eq!(command_status(true, None), Status::Running);
		assert_eq!(command_status(true, Some(Status::Stopped)), Status::Running);
		assert_eq!(command_status(true, Some(Status::Starting)), Status::Running);
	}
	#[test]
	fn command_starting_holds_until_port_opens() {
		// Just pressed Start; daemon not bound yet — stay Starting, don't flash Stopped.
		assert_eq!(command_status(false, Some(Status::Starting)), Status::Starting);
	}
	#[test]
	fn command_port_down_when_idle_or_dropped_is_stopped() {
		assert_eq!(command_status(false, None), Status::Stopped);
		assert_eq!(command_status(false, Some(Status::Stopped)), Status::Stopped);
		// A previously-Running daemon whose port dropped → Stopped.
		assert_eq!(command_status(false, Some(Status::Running)), Status::Stopped);
	}

	#[test]
	fn terminal_alive_no_port_is_running() {
		assert_eq!(terminal_status(Some(true), false, false, None), Status::Running);
	}
	#[test]
	fn terminal_alive_port_open_is_running_else_starting() {
		assert_eq!(terminal_status(Some(true), true, true, None), Status::Running);
		assert_eq!(terminal_status(Some(true), true, false, None), Status::Starting);
	}
	#[test]
	fn terminal_dead_pid_is_stopped() {
		// Closed window — Stopped, not Error.
		assert_eq!(terminal_status(Some(false), false, false, Some(Status::Running)), Status::Stopped);
	}
	#[test]
	fn terminal_untracked_no_port_keeps_last_known() {
		assert_eq!(terminal_status(None, false, false, Some(Status::Running)), Status::Running);
		assert_eq!(terminal_status(None, false, false, None), Status::Stopped);
	}

	#[test]
	fn exit_error_includes_code_and_log_tail() {
		let dir = std::env::temp_dir().join(format!("msm-he-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&dir).unwrap();
		let log = dir.join("x.log");
		std::fs::write(&log, "one\ntwo\nthree\nfour\n").unwrap();
		let msg = exit_error(Some(1), &log);
		assert!(msg.starts_with("process exited with code 1"));
		// Only the last 3 lines are appended.
		assert!(msg.contains("two\nthree\nfour"));
		assert!(!msg.contains("one"));
		// Unknown code + missing log → the bare message.
		assert_eq!(exit_error(None, &dir.join("missing.log")), "process exited");
		std::fs::remove_dir_all(&dir).ok();
	}

	#[test]
	fn tail_lines_reads_only_the_end_of_huge_files() {
		let dir = std::env::temp_dir().join(format!("msm-ht-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&dir).unwrap();
		let log = dir.join("big.log");
		// ~200 KiB of noise, well past the 64 KiB cap, then a final marker line.
		let mut text = "noise line\n".repeat(20_000);
		text.push_str("last-marker\n");
		std::fs::write(&log, &text).unwrap();
		let tail = tail_lines(&log, 2);
		assert!(tail.ends_with("last-marker"));
		assert_eq!(tail.lines().count(), 2);
		std::fs::remove_dir_all(&dir).ok();
	}

	#[test]
	fn aggregate_empty_is_nominal() {
		// No items at all — nominal, same as all-running/all-stopped.
		assert_eq!(aggregate_status(std::iter::empty()), None);
	}
	#[test]
	fn aggregate_running_and_stopped_are_nominal() {
		assert_eq!(aggregate_status([Status::Running, Status::Running].into_iter()), None);
		assert_eq!(aggregate_status([Status::Stopped, Status::Stopped].into_iter()), None);
		assert_eq!(aggregate_status([Status::Running, Status::Stopped].into_iter()), None);
	}
	#[test]
	fn aggregate_any_starting_is_starting() {
		assert_eq!(
			aggregate_status([Status::Running, Status::Starting, Status::Stopped].into_iter()),
			Some(Status::Starting)
		);
	}
	#[test]
	fn aggregate_error_beats_starting() {
		assert_eq!(
			aggregate_status([Status::Starting, Status::Error, Status::Running].into_iter()),
			Some(Status::Error)
		);
	}
}
