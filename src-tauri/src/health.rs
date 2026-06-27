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
					let pid_alive: Option<bool> = {
						let mut running = state.running.lock().unwrap();
						running.get_mut(&item.id).map(|r| crate::supervisor::is_alive(r))
					};
					match pid_alive {
						None => Status::Stopped,
						Some(alive) => {
							if !alive {
								state.errors.lock().unwrap()
									.insert(item.id.clone(), "process exited".into());
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
				RunMode::Terminal => match item.port {
					Some(p) => if port_open(p) { Status::Running } else { Status::Starting },
					None => current.unwrap_or(Status::Stopped),
				},
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
}
