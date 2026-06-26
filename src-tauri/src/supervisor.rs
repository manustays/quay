use crate::model::{AppError, ManagedItem};
use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// A tracked background process and its log file path.
///
/// `child` is `Some` when we spawned the process ourselves (an *owned* process,
/// which is its own session/process-group leader via `setsid`). It is `None` for
/// an *adopted* process — one we reattached to by PID/port after an app restart,
/// for which we hold no `Child` handle and did not create its process group.
pub struct Running {
	pub pid: u32,
	child: Option<Child>,
	pub log_path: PathBuf,
}

impl Running {
	/// True if we spawned this process (and therefore own its process group).
	pub fn is_owned(&self) -> bool {
		self.child.is_some()
	}
}

/// Spawn a background item via login shell in its own process group, logging to file.
///
/// Runs `zsh -lc "<start_cmd>"` with:
/// - cwd set to `item.dir`
/// - stdout+stderr appended to `logs_dir/<id>.log`
/// - `setsid()` called in a `pre_exec` hook so the child becomes a session/process-group leader
/// - any `item.env` entries merged into the child's environment
pub fn spawn_background(item: &ManagedItem, logs_dir: &Path) -> Result<Running, AppError> {
	let cmd_str = item
		.start_cmd
		.clone()
		.ok_or_else(|| AppError::Message("no start command".into()))?;
	let dir = item
		.dir
		.clone()
		.ok_or_else(|| AppError::Message("no directory".into()))?;

	let log_path = logs_dir.join(format!("{}.log", item.id));
	let log = OpenOptions::new()
		.create(true)
		.append(true)
		.open(&log_path)?;
	let log_err = log.try_clone()?;

	let mut cmd = Command::new("zsh");
	cmd.arg("-lc")
		.arg(&cmd_str)
		.current_dir(&dir)
		.stdout(Stdio::from(log))
		.stderr(Stdio::from(log_err))
		.stdin(Stdio::null());

	for (k, v) in &item.env {
		cmd.env(k, v);
	}

	// Run setsid() in the forked child before exec so the child becomes its own
	// session/process-group leader (PID == PGID). This lets stop() send a signal
	// to the whole process group via kill(-pgid, sig).
	unsafe {
		cmd.pre_exec(|| {
			libc::setsid();
			Ok(())
		});
	}

	let child = cmd
		.spawn()
		.map_err(|e| AppError::Message(format!("spawn failed: {e}")))?;
	let pid = child.id();
	Ok(Running { pid, child: Some(child), log_path })
}

/// Build a `Running` for an already-running process we did **not** spawn.
///
/// Used to reattach to an orphaned service (after an app restart) identified by PID
/// and/or its listening port. Has no `Child` handle, so liveness is checked via
/// `kill(pid, 0)` and stop targets the PID directly rather than its process group.
pub fn adopt(pid: u32, log_path: PathBuf) -> Running {
	Running { pid, child: None, log_path }
}

/// SIGTERM then (after 5 s) SIGKILL the process.
///
/// For an *owned* process (spawned by us as a `setsid` group leader) the signal is
/// sent to the whole process group via `kill(-pgid, …)`, reaping grandchildren. For
/// an *adopted* process we only signal the PID itself — we did not create its group,
/// so signalling `-pgid` could hit unrelated processes sharing that group.
pub fn stop(running: &mut Running) -> Result<(), AppError> {
	let owned = running.is_owned();
	let pid = running.pid as i32;
	let target = if owned { -pid } else { pid };
	unsafe { libc::kill(target, libc::SIGTERM) };

	for _ in 0..50 {
		if !is_alive(running) {
			return Ok(());
		}
		std::thread::sleep(std::time::Duration::from_millis(100));
	}

	// Still alive after 5 s — escalate.
	unsafe { libc::kill(target, libc::SIGKILL) };
	if let Some(child) = running.child.as_mut() {
		let _ = child.wait();
	}
	Ok(())
}

/// Return `true` if the process has not yet exited.
///
/// Owned processes use `try_wait` (reaps on exit); adopted ones probe with
/// `kill(pid, 0)`, which succeeds while the PID is live.
pub fn is_alive(running: &mut Running) -> bool {
	match running.child.as_mut() {
		Some(child) => matches!(child.try_wait(), Ok(None)),
		None => unsafe { libc::kill(running.pid as i32, 0) == 0 },
	}
}

/// Parse `lsof -t` output (one PID per line) into a sorted, de-duplicated list.
///
/// Tolerates blank lines and non-numeric garbage. Sorting makes PID selection
/// deterministic when a port has multiple listeners (e.g. IPv4 + IPv6).
pub fn parse_lsof_pids(out: &str) -> Vec<u32> {
	let mut pids: Vec<u32> = out
		.lines()
		.filter_map(|l| l.trim().parse::<u32>().ok())
		.collect();
	pids.sort_unstable();
	pids.dedup();
	pids
}

/// PIDs of processes listening on `127.0.0.1:<port>` (TCP), via `lsof`.
///
/// Returns an empty vec if `lsof` is missing or fails — callers degrade to
/// handle-only behavior.
pub fn pids_listening(port: u16) -> Vec<u32> {
	let Ok(out) = Command::new("lsof")
		.args(["-ti", &format!("tcp:{port}"), "-sTCP:LISTEN"])
		.output()
	else {
		return vec![];
	};
	parse_lsof_pids(&String::from_utf8_lossy(&out.stdout))
}

/// Best-effort: SIGTERM then (after 5 s) SIGKILL every PID listening on `port`.
///
/// The guaranteed "free the port" fallback used on explicit Stop when there is no
/// owned child to kill (e.g. an adopted/orphaned service). Signals PIDs directly —
/// never their process groups — since these processes were not spawned by us.
pub fn stop_port(port: u16) {
	let pids = pids_listening(port);
	if pids.is_empty() {
		return;
	}
	for &pid in &pids {
		unsafe { libc::kill(pid as i32, libc::SIGTERM) };
	}
	for _ in 0..50 {
		if pids_listening(port).is_empty() {
			return;
		}
		std::thread::sleep(std::time::Duration::from_millis(100));
	}
	for &pid in &pids {
		unsafe { libc::kill(pid as i32, libc::SIGKILL) };
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::model::{ItemKind, ManagedItem, RunMode};
	use std::collections::BTreeMap;

	fn item(cmd: &str, dir: &str) -> ManagedItem {
		ManagedItem {
			id: "test-id".into(), name: "t".into(), kind: ItemKind::Project,
			dir: Some(dir.into()), start_cmd: Some(cmd.into()), stop_cmd: None,
			port: None, run_mode: RunMode::Background, brew_formula: None, order: 0,
			favorite: false, env: BTreeMap::new(), health_path: None, auto_start: false,
		}
	}

	#[test]
	fn spawn_then_stop_kills_process() {
		let logs = std::env::temp_dir().join(format!("msm-sup-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&logs).unwrap();
		let it = item("sleep 30", "/tmp");
		let mut r = spawn_background(&it, &logs).unwrap();
		assert!(is_alive(&mut r));
		stop(&mut r).unwrap();
		assert!(!is_alive(&mut r));
		std::fs::remove_dir_all(&logs).ok();
	}

	#[test]
	fn parse_lsof_pids_handles_blanks_and_garbage() {
		assert_eq!(parse_lsof_pids("123\n456\n"), vec![123, 456]);
		assert_eq!(parse_lsof_pids(""), Vec::<u32>::new());
		// blank lines, whitespace, and non-numeric lines are ignored; sorted + deduped
		assert_eq!(parse_lsof_pids("\n  789 \nnot-a-pid\n123\n123\n"), vec![123, 789]);
	}

	#[test]
	fn adopted_running_tracks_liveness_via_kill0() {
		// Adopt a PID with no Child handle and verify the kill(pid,0) liveness path.
		// We reap via the real handle (in production an adopted process is reparented
		// to launchd and reaped there, so kill(pid,0) flips to ESRCH on death).
		let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
		let pid = child.id();
		let mut adopted = adopt(pid, std::path::PathBuf::from("/tmp/none.log"));
		assert!(!adopted.is_owned());
		assert!(is_alive(&mut adopted));
		child.kill().unwrap();
		child.wait().unwrap(); // reap so the PID is fully gone, not a zombie
		assert!(!is_alive(&mut adopted));
	}

	#[test]
	fn writes_log_file() {
		let logs = std::env::temp_dir().join(format!("msm-sup-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&logs).unwrap();
		let it = item("echo hello-marker", "/tmp");
		let mut r = spawn_background(&it, &logs).unwrap();
		std::thread::sleep(std::time::Duration::from_millis(400));
		let _ = stop(&mut r);
		let log = std::fs::read_to_string(logs.join("test-id.log")).unwrap();
		assert!(log.contains("hello-marker"));
		std::fs::remove_dir_all(&logs).ok();
	}
}
