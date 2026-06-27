//! Docker container management, parallel to [`crate::brew`].
//!
//! Docker items run inside the Docker Desktop VM, so there is no host PID to
//! supervise: status is derived from `docker ps`, metrics from `docker stats`
//! (see [`crate::metrics`]), and they never enter `AppState.running`. The `docker`
//! binary is resolved to an absolute path the same way [`crate::brew`] resolves
//! `brew`, so a bundled `.app` launched from Finder (minimal PATH) still works.

use crate::model::{AppError, ManagedItem, Status};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Candidate `docker` binary paths in priority order: Docker Desktop's Intel
/// symlink, the Apple-Silicon / Homebrew location, then Docker Desktop's bundled
/// CLI. Pure so it is unit-testable.
fn docker_candidates() -> Vec<PathBuf> {
	vec![
		PathBuf::from("/usr/local/bin/docker"),
		PathBuf::from("/opt/homebrew/bin/docker"),
		PathBuf::from("/Applications/Docker.app/Contents/Resources/bin/docker"),
	]
}

/// True if `path` is a regular file with at least one executable bit set.
fn is_executable(path: &Path) -> bool {
	std::fs::metadata(path)
		.map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
		.unwrap_or(false)
}

/// Ask the login shell where `docker` lives (covers nonstandard installs).
fn login_shell_docker() -> Option<PathBuf> {
	let out = Command::new("/bin/zsh").args(["-lc", "command -v docker"]).output().ok()?;
	if !out.status.success() {
		return None;
	}
	let pb = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
	is_executable(&pb).then_some(pb)
}

/// Resolve the `docker` binary: first executable candidate, then a login-shell
/// lookup, else the bare name `"docker"` (relies on PATH, as in dev mode).
fn resolve_docker() -> PathBuf {
	for c in docker_candidates() {
		if is_executable(&c) {
			return c;
		}
	}
	login_shell_docker().unwrap_or_else(|| PathBuf::from("docker"))
}

/// Absolute path to `docker`, resolved once and cached for the process lifetime.
fn docker_bin() -> &'static Path {
	static BIN: OnceLock<PathBuf> = OnceLock::new();
	BIN.get_or_init(resolve_docker)
}

// ── Daemon lifecycle ───────────────────────────────────────────────────────────

/// True if the Docker daemon answers. `docker info` exits non-zero when the
/// daemon is down even though the CLI itself exists, so exit status is the signal.
pub fn daemon_running() -> bool {
	Command::new(docker_bin())
		.arg("info")
		.output()
		.map(|o| o.status.success())
		.unwrap_or(false)
}

/// Launch Docker Desktop on macOS via `open -a Docker`. Errors only if `open`
/// itself can't be spawned; it returns immediately (the daemon comes up async).
pub fn start_daemon() -> Result<(), AppError> {
	let out = Command::new("open")
		.args(["-a", "Docker"])
		.output()
		.map_err(|e| AppError::Message(format!("could not launch Docker Desktop: {e}")))?;
	if out.status.success() {
		Ok(())
	} else {
		Err(AppError::Message(String::from_utf8_lossy(&out.stderr).trim().to_string()))
	}
}

/// Poll [`daemon_running`] roughly once a second until it is up or `timeout`
/// elapses. Returns true once the daemon answers, false on timeout.
pub fn wait_for_daemon(timeout: Duration) -> bool {
	let start = Instant::now();
	loop {
		if daemon_running() {
			return true;
		}
		if start.elapsed() >= timeout {
			return false;
		}
		std::thread::sleep(Duration::from_secs(1));
	}
}

// ── Images (add-form autocomplete) ───────────────────────────────────────────

/// Raw stdout of `docker images --format '{{.Repository}}:{{.Tag}}'`, or None if
/// `docker` couldn't be spawned (daemon down or CLI missing).
pub fn images_raw() -> Option<String> {
	let out = Command::new(docker_bin())
		.args(["images", "--format", "{{.Repository}}:{{.Tag}}"])
		.output()
		.ok()?;
	if !out.status.success() {
		return None;
	}
	Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parse `docker images` output into a deduped list of "repo:tag" strings,
/// dropping dangling (`<none>`) images and blank lines. Pure.
pub fn parse_docker_images(output: &str) -> Vec<String> {
	let mut seen = std::collections::HashSet::new();
	let mut out = Vec::new();
	for line in output.lines() {
		let tag = line.trim();
		if tag.is_empty() || tag.contains("<none>") {
			continue;
		}
		if seen.insert(tag.to_string()) {
			out.push(tag.to_string());
		}
	}
	out
}

/// Installed docker images as "repo:tag" strings, or empty if docker unavailable.
pub fn list_images() -> Vec<String> {
	images_raw().map(|o| parse_docker_images(&o)).unwrap_or_default()
}

// ── Container status ─────────────────────────────────────────────────────────

/// Raw `docker ps -a --format '{{.Names}}\t{{.State}}'` (`-a` includes stopped
/// containers), or None if docker couldn't be spawned.
pub fn ps_raw() -> Option<String> {
	let out = Command::new(docker_bin())
		.args(["ps", "-a", "--format", "{{.Names}}\t{{.State}}"])
		.output()
		.ok()?;
	if !out.status.success() {
		return None;
	}
	Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parse `docker ps` output into container name → Status. Pure.
///
/// `running` → Running; `restarting`/`created` → Starting; `exited`/`paused` →
/// Stopped; `dead`/`removing`/anything unrecognised → Error (an unknown state is
/// surfaced rather than silently reported as Stopped).
pub fn parse_docker_ps(output: &str) -> HashMap<String, Status> {
	let mut map = HashMap::new();
	for line in output.lines() {
		let mut cols = line.splitn(2, '\t');
		let (Some(name), Some(state)) = (cols.next(), cols.next()) else { continue; };
		let name = name.trim();
		if name.is_empty() {
			continue;
		}
		let status = match state.trim() {
			"running" => Status::Running,
			"restarting" | "created" => Status::Starting,
			"exited" | "paused" => Status::Stopped,
			_ => Status::Error,
		};
		map.insert(name.to_string(), status);
	}
	map
}

/// Raw stdout of `docker stats --no-stream` as name⇥cpu%⇥memusage, or None if
/// docker couldn't be spawned. Consumed by [`crate::metrics`].
pub fn stats_raw() -> Option<String> {
	let out = Command::new(docker_bin())
		.args(["stats", "--no-stream", "--format", "{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}"])
		.output()
		.ok()?;
	if !out.status.success() {
		return None;
	}
	Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Tri-state existence of a container by name: `Some(true)` exists & running,
/// `Some(false)` exists but not running, `None` absent (or docker unavailable).
pub fn container_state(name: &str) -> Option<bool> {
	let text = ps_raw()?;
	parse_docker_ps(&text).get(name).map(|s| *s == Status::Running)
}

/// Current status of a container by name (Stopped if docker missing or absent).
pub fn docker_status(name: &str) -> Status {
	let Some(text) = ps_raw() else { return Status::Stopped; };
	parse_docker_ps(&text).get(name).copied().unwrap_or(Status::Stopped)
}

// ── Start / stop ─────────────────────────────────────────────────────────────

/// How to bring a Docker item up, decided from its current container state. Pure.
#[derive(Debug, PartialEq)]
pub enum StartPlan {
	/// A container with this name already exists — reuse it via `docker start
	/// <name>`, preserving its volumes/data. Covers both running (no-op) and
	/// stopped containers.
	Reuse(String),
	/// No such container — run it fresh from the configured `start_cmd`.
	Fresh,
}

/// Decide the start strategy from `container_state(name)`. Pure.
pub fn plan_start(state: Option<bool>, container_name: &str) -> StartPlan {
	match state {
		Some(_) => StartPlan::Reuse(container_name.to_string()),
		None => StartPlan::Fresh,
	}
}

/// Start a Docker item. Reuses an existing container by name (`docker start`) or,
/// when none exists, runs the configured `start_cmd` fresh through a login shell
/// (so quoting in `-e "A=B C"` etc. is honoured and `docker` is on PATH).
pub fn docker_start(item: &ManagedItem) -> Result<(), AppError> {
	let name = item
		.container_name
		.as_deref()
		.filter(|n| !n.trim().is_empty())
		.ok_or_else(|| AppError::Message("docker item has no container name".into()))?;

	match plan_start(container_state(name), name) {
		StartPlan::Reuse(n) => {
			let out = Command::new(docker_bin())
				.args(["start", &n])
				.output()
				.map_err(|e| AppError::Message(format!("docker start failed to run: {e}")))?;
			if out.status.success() {
				Ok(())
			} else {
				Err(AppError::Message(String::from_utf8_lossy(&out.stderr).trim().to_string()))
			}
		}
		StartPlan::Fresh => {
			let cmd = item
				.start_cmd
				.as_deref()
				.filter(|c| !c.trim().is_empty())
				.ok_or_else(|| AppError::Message("docker item has no start command".into()))?;
			let mut shell = Command::new("/bin/zsh");
			shell.args(["-lc", cmd]);
			if let Some(path) = crate::supervisor::interactive_path() {
				shell.env("PATH", path);
			}
			for (k, v) in &item.env {
				shell.env(k, v);
			}
			let out = shell
				.output()
				.map_err(|e| AppError::Message(format!("docker run failed to run: {e}")))?;
			if out.status.success() {
				Ok(())
			} else {
				Err(AppError::Message(String::from_utf8_lossy(&out.stderr).trim().to_string()))
			}
		}
	}
}

/// Stop a container by name via `docker stop <name>`.
pub fn docker_stop(name: &str) -> Result<(), AppError> {
	let out = Command::new(docker_bin())
		.args(["stop", name])
		.output()
		.map_err(|e| AppError::Message(format!("docker stop failed to run: {e}")))?;
	if out.status.success() {
		Ok(())
	} else {
		Err(AppError::Message(String::from_utf8_lossy(&out.stderr).trim().to_string()))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn candidate_order_is_stable() {
		assert_eq!(docker_candidates(), vec![
			PathBuf::from("/usr/local/bin/docker"),
			PathBuf::from("/opt/homebrew/bin/docker"),
			PathBuf::from("/Applications/Docker.app/Contents/Resources/bin/docker"),
		]);
	}

	#[test]
	fn is_executable_distinguishes_files_and_dirs() {
		assert!(!is_executable(Path::new("/opt")));
		assert!(is_executable(Path::new("/bin/sh")));
		assert!(!is_executable(Path::new("/no/such/docker")));
	}

	#[test]
	fn parses_images_dedupes_and_drops_dangling() {
		let out = "mongodb/mongodb-community-server:latest\nredis:7\n<none>:<none>\nredis:7\n\n";
		let imgs = parse_docker_images(out);
		assert_eq!(imgs, vec![
			"mongodb/mongodb-community-server:latest".to_string(),
			"redis:7".to_string(),
		]);
	}

	#[test]
	fn parses_ps_states() {
		let out = "mongodb\trunning\nbuilder\tcreated\nold\texited\nbroke\tdead\nweird\tquantum\n";
		let m = parse_docker_ps(out);
		assert_eq!(m.get("mongodb"), Some(&Status::Running));
		assert_eq!(m.get("builder"), Some(&Status::Starting));
		assert_eq!(m.get("old"), Some(&Status::Stopped));
		// dead and unknown states both surface as Error, never silently Stopped.
		assert_eq!(m.get("broke"), Some(&Status::Error));
		assert_eq!(m.get("weird"), Some(&Status::Error));
	}

	#[test]
	fn plan_start_reuses_when_container_exists() {
		// Stopped container → reuse (preserve data).
		assert_eq!(plan_start(Some(false), "mongodb"), StartPlan::Reuse("mongodb".into()));
		// Running container → reuse (docker start is a harmless no-op).
		assert_eq!(plan_start(Some(true), "mongodb"), StartPlan::Reuse("mongodb".into()));
		// Absent container → run fresh.
		assert_eq!(plan_start(None, "mongodb"), StartPlan::Fresh);
	}
}
