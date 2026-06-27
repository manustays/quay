use crate::model::{AppError, Status};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Candidate `brew` binary paths in priority order.
///
/// `$HOMEBREW_PREFIX/bin/brew` first (honours custom installs), then the two
/// standard locations: Apple Silicon, then Intel. An empty prefix is ignored.
fn brew_candidates(homebrew_prefix: Option<&str>) -> Vec<PathBuf> {
	let mut v = Vec::new();
	if let Some(p) = homebrew_prefix {
		if !p.is_empty() {
			v.push(PathBuf::from(p).join("bin/brew"));
		}
	}
	v.push(PathBuf::from("/opt/homebrew/bin/brew"));
	v.push(PathBuf::from("/usr/local/bin/brew"));
	v
}

/// True if `path` is a regular file with at least one executable bit set.
fn is_executable(path: &Path) -> bool {
	std::fs::metadata(path)
		.map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
		.unwrap_or(false)
}

/// Ask the user's login shell where `brew` lives (covers nonstandard prefixes).
///
/// Runs `zsh -lc 'command -v brew'`, which sources the login profile so it sees
/// the same PATH a terminal would. Returns `None` if the shell or brew is absent.
fn login_shell_brew() -> Option<PathBuf> {
	let out = Command::new("/bin/zsh").args(["-lc", "command -v brew"]).output().ok()?;
	if !out.status.success() {
		return None;
	}
	let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
	let pb = PathBuf::from(path);
	is_executable(&pb).then_some(pb)
}

/// Resolve the `brew` binary: first executable candidate, then a login-shell
/// lookup, else the bare name `"brew"` (relies on PATH, as in dev mode).
fn resolve_brew() -> PathBuf {
	for c in brew_candidates(std::env::var("HOMEBREW_PREFIX").ok().as_deref()) {
		if is_executable(&c) {
			return c;
		}
	}
	login_shell_brew().unwrap_or_else(|| PathBuf::from("brew"))
}

/// Absolute path to `brew`, resolved once and cached for the process lifetime.
///
/// A bundled `.app` launched from Finder/Dock gets a minimal PATH without the
/// Homebrew dir, so a bare `Command::new("brew")` fails there; resolving the
/// absolute path fixes brew listing and status detection in the bundled app.
fn brew_bin() -> &'static Path {
	static BIN: OnceLock<PathBuf> = OnceLock::new();
	BIN.get_or_init(resolve_brew)
}

/// Raw stdout of `brew services list`, or `None` if `brew` couldn't be spawned.
///
/// Exit status is intentionally ignored (matching prior behaviour): a nonzero
/// exit still returns whatever stdout was produced, which `parse_brew_list`
/// tolerates.
pub fn services_list_raw() -> Option<String> {
	let out = Command::new(brew_bin()).args(["services", "list"]).output().ok()?;
	Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parse `brew services list` output into formula → Status.
///
/// Skips the header line. Maps `started` → Running, `error` → Error,
/// everything else (`stopped`, `none`, …) → Stopped.
pub fn parse_brew_list(output: &str) -> HashMap<String, Status> {
	let mut map = HashMap::new();
	for line in output.lines().skip(1) {
		let mut cols = line.split_whitespace();
		let (Some(name), Some(state)) = (cols.next(), cols.next()) else { continue; };
		let status = match state {
			"started" => Status::Running,
			"error" => Status::Error,
			_ => Status::Stopped,
		};
		map.insert(name.to_string(), status);
	}
	map
}

/// Raw stdout of `launchctl list`, or `None` if it couldn't be spawned.
///
/// `launchctl` lives at `/bin/launchctl` and is always on PATH, so no resolution
/// dance (unlike [`brew_bin`]) is needed.
pub fn launchctl_list_raw() -> Option<String> {
	let out = Command::new("launchctl").arg("list").output().ok()?;
	Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Prefix every Homebrew launchd job label carries.
const HOMEBREW_LABEL_PREFIX: &str = "homebrew.mxcl.";

/// Parse `launchctl list` output into formula → main PID.
///
/// Lines are `PID<whitespace>ExitStatus<whitespace>Label`. Only labels starting
/// with `homebrew.mxcl.` are kept, and only when the PID column is numeric (a
/// stopped job has `-` there, which is skipped). The formula key is the label with
/// the prefix removed, so a versioned formula like `mysql@8.0` round-trips. On
/// duplicate labels (e.g. the same formula from two Homebrew prefixes) the last
/// line wins.
pub fn parse_service_pids(output: &str) -> HashMap<String, u32> {
	let mut map = HashMap::new();
	for line in output.lines() {
		let mut cols = line.split_whitespace();
		let (Some(pid), Some(_status), Some(label)) = (cols.next(), cols.next(), cols.next())
		else {
			continue;
		};
		let Some(formula) = label.strip_prefix(HOMEBREW_LABEL_PREFIX) else {
			continue;
		};
		let Ok(pid) = pid.parse::<u32>() else { continue };
		map.insert(formula.to_string(), pid);
	}
	map
}

/// Main PIDs of running Homebrew launchd services, keyed by formula. Empty if
/// `launchctl` couldn't be spawned.
pub fn service_pids() -> HashMap<String, u32> {
	launchctl_list_raw().map(|o| parse_service_pids(&o)).unwrap_or_default()
}

/// Current status of a brew formula (Stopped if brew missing or formula absent).
pub fn brew_status(formula: &str) -> Status {
	let Some(text) = services_list_raw() else {
		return Status::Stopped;
	};
	parse_brew_list(&text)
		.get(formula)
		.copied()
		.unwrap_or(Status::Stopped)
}

/// Start a brew formula's background service.
pub fn brew_start(formula: &str) -> Result<(), AppError> {
	run_brew("start", formula)
}

/// Stop a brew formula's background service.
pub fn brew_stop(formula: &str) -> Result<(), AppError> {
	run_brew("stop", formula)
}

/// Run `brew services <action> <formula>`, surfacing stderr on failure.
fn run_brew(action: &str, formula: &str) -> Result<(), AppError> {
	let brew = brew_bin();
	let out = Command::new(brew)
		.args(["services", action, formula])
		.output()
		.map_err(|e| AppError::Message(format!("brew ({}) failed to run: {e}", brew.display())))?;
	if out.status.success() {
		Ok(())
	} else {
		Err(AppError::Message(
			String::from_utf8_lossy(&out.stderr).trim().to_string(),
		))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::model::Status;

	#[test]
	fn candidate_order_prefers_homebrew_prefix() {
		let c = brew_candidates(Some("/custom/brew"));
		assert_eq!(c, vec![
			PathBuf::from("/custom/brew/bin/brew"),
			PathBuf::from("/opt/homebrew/bin/brew"),
			PathBuf::from("/usr/local/bin/brew"),
		]);
	}

	#[test]
	fn candidate_order_without_prefix_skips_empty() {
		let expected = vec![
			PathBuf::from("/opt/homebrew/bin/brew"),
			PathBuf::from("/usr/local/bin/brew"),
		];
		assert_eq!(brew_candidates(None), expected);
		// An empty prefix is ignored, not turned into "/bin/brew".
		assert_eq!(brew_candidates(Some("")), expected);
	}

	#[test]
	fn is_executable_distinguishes_files_and_dirs() {
		// A directory is never "executable" in our sense (not a regular file).
		assert!(!is_executable(Path::new("/opt")));
		// /bin/sh exists and is executable on macOS.
		assert!(is_executable(Path::new("/bin/sh")));
		assert!(!is_executable(Path::new("/no/such/brew")));
	}

	#[test]
	fn parses_brew_services_list() {
		let out = "Name    Status  User  File\nmysql   started abhi  ~/x\nmongodb stopped -     -\nredis   error   abhi  ~/y\n";
		let m = parse_brew_list(out);
		assert_eq!(m.get("mysql"), Some(&Status::Running));
		assert_eq!(m.get("mongodb"), Some(&Status::Stopped));
		assert_eq!(m.get("redis"), Some(&Status::Error));
	}

	#[test]
	fn parses_launchctl_service_pids() {
		// Mix of tabs and spaces, a running job, a stopped job (`-` PID), a
		// versioned formula, a non-homebrew label, and a short/malformed line.
		let out = "PID\tStatus\tLabel\n\
			61787\t0\thomebrew.mxcl.mysql\n\
			-      14   homebrew.mxcl.mongodb-community\n\
			512\t0\thomebrew.mxcl.mysql@8.0\n\
			999\t0\tcom.apple.something\n\
			garbage line\n";
		let m = parse_service_pids(out);
		// Running formula maps to its PID.
		assert_eq!(m.get("mysql"), Some(&61787));
		// Versioned formula round-trips with the version suffix intact.
		assert_eq!(m.get("mysql@8.0"), Some(&512));
		// Stopped job (`-` PID) is skipped.
		assert_eq!(m.get("mongodb-community"), None);
		// Non-homebrew labels are ignored; "Label" header line too.
		assert_eq!(m.get("something"), None);
		assert!(!m.contains_key("Label"));
		// Only the two valid homebrew entries survive.
		assert_eq!(m.len(), 2);
	}

	#[test]
	fn parse_service_pids_duplicate_label_last_wins() {
		// Same formula from two prefixes — deterministic last-line-wins.
		let out = "100 0 homebrew.mxcl.redis\n200 0 homebrew.mxcl.redis\n";
		assert_eq!(parse_service_pids(out).get("redis"), Some(&200));
	}
}
