use crate::model::{AppError, ManagedItem};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;

/// PATH as seen by the user's interactive login shell, resolved once and cached.
///
/// A `.app` launched from Finder inherits a minimal PATH, and `zsh -lc` is a
/// *non-interactive* login shell that never sources `~/.zshrc` — where version
/// managers (nvm, asdf, pyenv, …) typically extend PATH. So binaries like `n8n`
/// are missing even though they work in a normal terminal. We ask an *interactive*
/// login shell (`-ilc`) for its PATH a single time and inject it into spawned
/// commands, keeping interactive `.zshrc` side effects (prompt init, `exec zsh`,
/// `compinit`, …) confined to this one cached probe instead of every service start.
///
/// The PATH is the last non-empty line of stdout, so framework chatter printed
/// before it (e.g. `exec zsh`) is ignored. Returns `None` if the probe fails or
/// yields nothing, in which case callers fall back to the inherited PATH.
pub fn interactive_path() -> Option<&'static str> {
	static PATH: OnceLock<Option<String>> = OnceLock::new();
	PATH.get_or_init(|| {
		let out = Command::new("/bin/zsh").args(["-ilc", "print -rn -- $PATH"]).output().ok()?;
		if !out.status.success() {
			return None;
		}
		let raw = String::from_utf8_lossy(&out.stdout);
		let path = raw.lines().rev().map(str::trim).find(|l| !l.is_empty())?;
		Some(path.to_string())
	})
	.as_deref()
}

/// Run a one-shot control command to completion and return its result.
///
/// Used by `command`-kind items whose lifecycle is a pair of user commands
/// (e.g. `omlx start` / `omlx stop`) that launch or signal a *detached* daemon
/// and return promptly — unlike [`spawn_background`], nothing is owned or kept.
/// Same PATH recipe as [`spawn_background`]: [`interactive_path`] first, then
/// `env` on top so the user can override. cwd = `dir` or `$HOME`.
///
/// A nonzero exit is an error carrying the last line of stderr (falling back to
/// stdout), so a failed start/stop surfaces in the UI instead of a false state.
///
/// ponytail: runs synchronously via `.output()` with no timeout — a control
/// command that never returns will wedge this Tauri worker. The contract is
/// that start/stop commands return promptly (they signal a daemon, not host it);
/// a `wait_timeout`-based kill is the upgrade path if that ever bites.
pub fn run_command(
	dir: Option<&str>,
	env: &BTreeMap<String, String>,
	cmd: &str,
) -> Result<(), AppError> {
	let workdir = dir
		.map(str::to_string)
		.filter(|d| !d.is_empty())
		.unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".into()));

	let mut command = Command::new("/bin/zsh");
	command
		.arg("-lc")
		.arg(cmd)
		.current_dir(&workdir)
		.stdin(Stdio::null());
	if let Some(path) = interactive_path() {
		command.env("PATH", path);
	}
	for (k, v) in env {
		command.env(k, v);
	}

	let out = command
		.output()
		.map_err(|e| AppError::Message(format!("run failed: {e}")))?;
	if out.status.success() {
		return Ok(());
	}
	let stderr = String::from_utf8_lossy(&out.stderr);
	let stdout = String::from_utf8_lossy(&out.stdout);
	let detail = stderr
		.lines()
		.rev()
		.find(|l| !l.trim().is_empty())
		.or_else(|| stdout.lines().rev().find(|l| !l.trim().is_empty()))
		.unwrap_or("no output")
		.trim();
	Err(AppError::Message(format!("command failed: {detail}")))
}

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
/// Runs `/bin/zsh -lc "<start_cmd>"` with:
/// - cwd set to `item.dir`
/// - stdout+stderr appended to `logs_dir/<id>.log`
/// - `setsid()` called in a `pre_exec` hook so the child becomes a session/process-group leader
/// - PATH set from [`interactive_path`] (so nvm/asdf/pyenv binaries resolve), then
///   any `item.env` entries merged on top (so the user can still override PATH)
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

	let mut cmd = Command::new("/bin/zsh");
	cmd.arg("-lc")
		.arg(&cmd_str)
		.current_dir(&dir)
		.stdout(Stdio::from(log))
		.stderr(Stdio::from(log_err))
		.stdin(Stdio::null());

	if let Some(path) = interactive_path() {
		cmd.env("PATH", path);
	}
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

/// Exit code of an owned process that has already exited, if known.
///
/// `try_wait` caches the status once the child is reaped, so this is safe to
/// call after [`is_alive`] returned `false`. `None` for adopted processes (no
/// `Child` handle) and for signal-terminated children (no exit code).
pub fn exit_code(running: &mut Running) -> Option<i32> {
	running.child.as_mut()?.try_wait().ok().flatten()?.code()
}

/// Parse `lsof -Fpn` field output into unique `(port, pid)` pairs. Pure.
///
/// The format is one field per line: `p<pid>` starts a process section, each
/// `n<addr>` names a socket (e.g. `n*:3000`, `n127.0.0.1:5173`, `n[::1]:8080`).
/// The port is whatever follows the last `:`. Garbage lines are skipped.
pub fn parse_lsof_fields(out: &str) -> Vec<(u16, u32)> {
	let mut pairs: Vec<(u16, u32)> = Vec::new();
	let mut seen: std::collections::HashSet<(u16, u32)> = std::collections::HashSet::new();
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
/// The single lsof entry point shared by the port radar, metrics, adopt/stop,
/// and launch reattach. `-u <uid>` restricts to our own processes — foreign-user
/// listeners can't be resolved or signalled anyway, and skipping them avoids the
/// Full Disk Access prompt entirely. Empty when `lsof` is missing or fails.
pub fn listeners() -> Vec<(u16, u32)> {
	let uid = unsafe { libc::getuid() }.to_string();
	let Ok(out) = Command::new("lsof")
		.args(["-iTCP", "-sTCP:LISTEN", "-P", "-n", "-a", "-u", &uid, "-Fpn"])
		.output()
	else {
		return vec![];
	};
	parse_lsof_fields(&String::from_utf8_lossy(&out.stdout))
}

/// PIDs of our own processes listening on `<port>` (TCP), sorted and deduped.
///
/// A filter over [`listeners`]; sorting makes PID selection deterministic when a
/// port has multiple listeners (e.g. IPv4 + IPv6). Empty when `lsof` fails.
pub fn pids_listening(port: u16) -> Vec<u32> {
	pids_for_port(&listeners(), port)
}

/// PIDs listening on `port` within an existing `(port, pid)` snapshot, sorted
/// and deduped — lets a caller reuse one [`listeners`] scan across many ports.
pub fn pids_for_port(listeners: &[(u16, u32)], port: u16) -> Vec<u32> {
	let mut pids: Vec<u32> =
		listeners.iter().filter(|&&(p, _)| p == port).map(|&(_, pid)| pid).collect();
	pids.sort_unstable();
	pids.dedup();
	pids
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
			port: None, run_mode: RunMode::Background, brew_formula: None,
			docker_image: None, container_name: None, stack: None, group: None, order: 0,
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
	fn parse_lsof_fields_handles_addr_shapes_and_dedupes() {
		let out = "p123\nn*:3000\nn127.0.0.1:5173\np456\nn[::1]:8080\nn[::]:8080\n";
		assert_eq!(parse_lsof_fields(out), vec![(3000, 123), (5173, 123), (8080, 456)]);
		// v4+v6 on one port dedupe; an n-line before any p-line and a port-less name skip.
		assert_eq!(parse_lsof_fields("nno-pid:99\np12\nnlocalhost:3000\nn[::1]:3000\nnbad:\n"), vec![(3000, 12)]);
		assert_eq!(parse_lsof_fields(""), Vec::<(u16, u32)>::new());
	}

	#[test]
	fn pids_for_port_filters_sorts_dedupes() {
		let snap = vec![(3000, 456), (3000, 123), (5173, 999), (3000, 123)];
		assert_eq!(pids_for_port(&snap, 3000), vec![123, 456]);
		assert_eq!(pids_for_port(&snap, 8080), Vec::<u32>::new());
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
