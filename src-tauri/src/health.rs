use crate::model::Status;
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

/// Build the error message for an exited process: the exit code when known
/// (owned children; signal deaths and adopted PIDs have none) plus the last
/// few log lines so the cause is visible without opening the log.
pub fn exit_error(exit_code: Option<i32>, log_path: &std::path::Path) -> String {
	let mut msg = match exit_code {
		Some(code) => format!("process exited with code {code}"),
		None => "process exited".to_string(),
	};
	if let Ok(text) = std::fs::read_to_string(log_path) {
		let tail: Vec<&str> = text.lines().rev().take(3).collect();
		if !tail.is_empty() {
			let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
			msg = format!("{msg}\n{tail}");
		}
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

/// Spawn a background thread that calls `poll_once` every `poll_interval_sec` seconds.
pub fn spawn_poll_loop(app: AppHandle) {
	std::thread::spawn(move || loop {
		let interval = {
			let st = app.state::<AppState>();
			let secs = st.config.lock().unwrap().settings.poll_interval_sec.max(1);
			secs
		};
		poll_once(&app);
		std::thread::sleep(std::time::Duration::from_secs(interval));
	});
}

/// One poll pass: compute each item's live status and call `set_status` on changes.
///
/// Skips non-brew items whose status is None or Stopped (never started).
pub fn poll_once(app: &AppHandle) {
	let state = app.state::<AppState>();
	let items = state.config.lock().unwrap().items.clone();
	for item in items {
		let current = state.statuses.lock().unwrap().get(&item.id).copied();
		// Brew + Docker are polled even when Stopped: their state lives outside the
		// app (launchctl / `docker ps`), so a container started or stopped elsewhere
		// is still reflected.
		if matches!(current, None | Some(Status::Stopped))
			&& !matches!(item.kind, ItemKind::Brew | ItemKind::Docker)
		{
			continue; // never started; leave as-is
		}
		let status = match item.kind {
			ItemKind::Brew => {
				item.brew_formula.as_deref()
					.map(crate::brew::brew_status)
					.unwrap_or(Status::Stopped)
			}
			ItemKind::Docker => {
				item.container_name.as_deref()
					.map(crate::docker::docker_status)
					.unwrap_or(Status::Stopped)
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
							if !alive {
								let log_path = state.dir.join("logs").join(format!("{}.log", item.id));
								state.errors.lock().unwrap()
									.insert(item.id.clone(), exit_error(exit_code, &log_path));
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
