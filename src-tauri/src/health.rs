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
