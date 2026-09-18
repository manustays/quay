//! Who invoked us, resolved without forking.
//!
//! The hook runs on the agent's hot path — Claude Code fires `PostToolUse` on every
//! tool call — so shelling out to `ps` once per event is not acceptable. `libproc`
//! answers the same questions with a syscall: parent, executable, start time and
//! controlling terminal.
//!
//! None of the four agents put a pid in their hook payload, which is why the radar
//! has to identify sessions by `(agent, cwd)` and fork `ps` to tell whether one is
//! still alive. Stamping the pid here is what removes both.

use std::ffi::CStr;

/// What we can learn about one process.
pub struct ProcInfo {
	pub pid: u32,
	pub ppid: u32,
	/// Absolute executable path, or the kernel's truncated process name if the full
	/// path is unreadable (another user's process, say).
	pub exe: String,
	/// Process start, as whole seconds since the epoch.
	///
	/// The pid alone is not an identity: pids are recycled, so `kill(pid, 0)` will
	/// happily report a *different* process alive under a dead session's number —
	/// and across a reboot that is near certain rather than unlucky. The pair is the
	/// identity; the start time is what makes it one.
	pub started_at: u64,
	/// Controlling terminal as `ttysNNN`, or `None` when the process has none.
	pub tty: Option<String>,
}

// `devname(3)` turns a device number into "ttys004". Stable C, but libc has no
// binding for it on Apple targets.
unsafe extern "C" {
	fn devname(dev: libc::dev_t, mode: libc::mode_t) -> *const libc::c_char;
}

/// Read one process's info. `None` if it does not exist or we may not look at it.
pub fn info(pid: u32) -> Option<ProcInfo> {
	if pid <= 1 {
		return None;
	}
	let mut bsd: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
	let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
	let got = unsafe {
		libc::proc_pidinfo(
			pid as libc::c_int,
			libc::PROC_PIDTBSDINFO,
			0,
			&mut bsd as *mut _ as *mut libc::c_void,
			size,
		)
	};
	// A short read means the struct we were given is not the one we asked for.
	if got != size {
		return None;
	}
	Some(ProcInfo {
		pid: bsd.pbi_pid,
		ppid: bsd.pbi_ppid,
		exe: exe_path(pid).unwrap_or_else(|| c_str(&bsd.pbi_name).unwrap_or_default()),
		started_at: bsd.pbi_start_tvsec,
		tty: tty_name(bsd.e_tdev),
	})
}

/// Full executable path. Fails for processes we don't own, hence the caller's
/// fallback to the kernel's 32-char process name.
fn exe_path(pid: u32) -> Option<String> {
	let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
	let len = unsafe {
		libc::proc_pidpath(pid as libc::c_int, buf.as_mut_ptr() as *mut libc::c_void, buf.len() as u32)
	};
	if len <= 0 {
		return None;
	}
	buf.truncate(len as usize);
	String::from_utf8(buf).ok()
}

fn tty_name(tdev: u32) -> Option<String> {
	// NODEV is -1; a process with no controlling terminal reports it.
	if tdev == u32::MAX {
		return None;
	}
	let raw = unsafe { devname(tdev as libc::dev_t, libc::S_IFCHR) };
	if raw.is_null() {
		return None;
	}
	let name = unsafe { CStr::from_ptr(raw) }.to_str().ok()?;
	// `devname` answers "ttys004"; the radar and AppleScript both want it bare.
	(!name.is_empty()).then(|| name.to_string())
}

fn c_str(raw: &[libc::c_char]) -> Option<String> {
	let bytes: Vec<u8> = raw.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
	String::from_utf8(bytes).ok().filter(|s| !s.is_empty())
}

/// Is the process recorded as `(pid, started_at)` still that same process?
///
/// The whole point of recording the start time. `kill(pid, 0)` answers "is *a*
/// process alive under this number", which is a different and much weaker question:
/// pids are recycled, so a dead session's number is eventually reused by something
/// unrelated, and after a reboot low numbers are reused immediately. Comparing the
/// start time makes the answer about *this* process.
///
/// Reads as dead if the process cannot be inspected at all. Agents run as the same
/// user as Quay, so that means gone rather than forbidden.
pub fn is_same_process(pid: u32, started_at: u64) -> bool {
	info(pid).is_some_and(|p| p.started_at == started_at)
}

/// Does this executable path belong to `agent`?
///
/// Basename match, mirroring how `agent_radar::agent_from_argv` identifies a session
/// from argv\[0\]. Pure, so the walk below stays testable.
pub fn is_agent_exe(exe: &str, agent: &str) -> bool {
	exe.rsplit('/').next().unwrap_or(exe) == agent
}

/// How far up to look for the agent. Claude Code runs a hook through a shell, so the
/// agent is usually the grandparent; the rest is slack for wrappers.
const MAX_HOPS: usize = 8;

/// Executable paths of `pid` and its ancestors, nearest first.
///
/// The fork-free replacement for walking a `ps` snapshot's parent chain. The radar
/// uses it to spot a host terminal it can focus by tty (Terminal.app, iTerm) — those
/// expose no environment variable identifying themselves, so ancestry is the only
/// way to recognise them.
pub fn ancestor_exes(pid: u32, max_hops: usize) -> Vec<String> {
	let mut out = Vec::new();
	let mut current = pid;
	for _ in 0..max_hops {
		let Some(proc_) = info(current) else { break };
		out.push(proc_.exe);
		if proc_.ppid <= 1 {
			break;
		}
		current = proc_.ppid;
	}
	out
}

/// Walk up from `start` looking for the process running `agent`.
///
/// Deliberately returns `None` rather than guessing. Stamping the wrong pid is worse
/// than stamping none: the shell that ran this hook exits the moment we do, so a
/// state file carrying *its* pid would read as dead immediately and the session would
/// vanish from the radar. No pid just means the old `(agent, cwd)` behaviour.
pub fn find_agent(start: u32, agent: &str) -> Option<ProcInfo> {
	let mut current = start;
	for _ in 0..MAX_HOPS {
		let proc_ = info(current)?;
		if is_agent_exe(&proc_.exe, agent) {
			return Some(proc_);
		}
		if proc_.ppid <= 1 {
			return None;
		}
		current = proc_.ppid;
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn agent_is_matched_by_basename_not_by_substring() {
		assert!(is_agent_exe("/opt/homebrew/bin/claude", "claude"));
		assert!(is_agent_exe("claude", "claude"));
		assert!(is_agent_exe("/Users/x/.pi/bin/pi", "pi"));
		// A project path that merely contains the name is not the agent.
		assert!(!is_agent_exe("/Users/x/claude/notes/editor", "claude"));
		// Nor is a different agent.
		assert!(!is_agent_exe("/usr/local/bin/codex", "claude"));
		// Nor a longer name sharing the prefix.
		assert!(!is_agent_exe("/usr/local/bin/claude-helper", "claude"));
	}

	#[test]
	fn our_own_process_reads_back_consistently() {
		// This is raw libproc FFI, so what matters is that the struct we get back is
		// really ours — a size or layout mistake shows up as nonsense here.
		let me = info(std::process::id()).expect("our own process must be readable");
		assert_eq!(me.pid, std::process::id());
		assert_eq!(me.ppid, unsafe { libc::getppid() } as u32);
		assert!(!me.exe.is_empty(), "executable path or name must resolve");
		assert!(me.exe.contains("quay") || me.exe.contains("proc_info"), "got: {}", me.exe);

		let now = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_secs();
		assert!(me.started_at <= now, "a process cannot have started in the future");
		assert!(now - me.started_at < 60 * 60 * 24, "start time is not a plausible epoch value");
	}

	#[test]
	fn a_nonexistent_or_reserved_pid_is_none() {
		assert!(info(0).is_none(), "pid 0 is not a thing we can stamp");
		assert!(info(1).is_none(), "launchd is never the agent");
		// Very high pids are unused on macOS (default max is ~99999).
		assert!(info(4_000_000_000).is_none());
	}

	#[test]
	fn the_walk_gives_up_instead_of_guessing() {
		// Nothing in our ancestry is called this, so the walk must run out and return
		// None rather than fall back to whatever it happened to be looking at.
		assert!(find_agent(std::process::id(), "definitely-not-an-agent").is_none());
	}

	#[test]
	fn identity_is_the_pair_not_the_pid() {
		let me = std::process::id();
		let started = info(me).expect("our own process").started_at;
		assert!(is_same_process(me, started), "our own pid and start time must match");

		// The same pid with any other start time is a different process — this is the
		// recycled-pid case, and the only thing standing between it and a session row
		// that never goes away.
		assert!(!is_same_process(me, started + 1));
		assert!(!is_same_process(me, 0));
		// A pid nothing is using reads as gone rather than as a match.
		assert!(!is_same_process(4_000_000_000, started));
	}

	#[test]
	fn ancestors_are_listed_nearest_first_and_are_bounded() {
		let chain = ancestor_exes(std::process::id(), 8);
		assert!(!chain.is_empty(), "our own executable at minimum");
		assert!(chain.len() <= 8, "the hop limit is what stops a cyclic table looping");
		assert!(
			chain[0].contains("quay") || chain[0].contains("proc_info"),
			"nearest first — got {}",
			chain[0]
		);
		// The test binary is not launched by launchd directly, so there is a parent.
		assert!(chain.len() >= 2, "a test process has at least one ancestor");
	}
}
