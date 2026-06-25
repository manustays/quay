use crate::model::{AppError, ManagedItem};
use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// A spawned background child and its log file path.
pub struct Running {
	pub pid: u32,
	child: Child,
	pub log_path: PathBuf,
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
	Ok(Running { pid, child, log_path })
}

/// SIGTERM the process group; escalate to SIGKILL after 5 s if still alive.
pub fn stop(running: &mut Running) -> Result<(), AppError> {
	let pgid = running.pid as i32;
	// Negative pgid targets the entire process group.
	unsafe { libc::kill(-pgid, libc::SIGTERM) };

	for _ in 0..50 {
		if !is_alive(running) {
			return Ok(());
		}
		std::thread::sleep(std::time::Duration::from_millis(100));
	}

	// Still alive after 5 s — escalate.
	unsafe { libc::kill(-pgid, libc::SIGKILL) };
	let _ = running.child.wait();
	Ok(())
}

/// Return `true` if the child process has not yet exited.
pub fn is_alive(running: &mut Running) -> bool {
	matches!(running.child.try_wait(), Ok(None))
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
