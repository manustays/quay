//! Claude Code hook helper: records per-session state for Quay's agent radar.
//!
//! Registered in `~/.claude/settings.json` (see docs/agent-radar.md), invoked
//! by Claude Code on lifecycle events with the event JSON on stdin. Writes one
//! small file per session under `<data_dir>/am.abhi.quay/agent-state/` that the
//! radar's 5 s poll reads to distinguish "waiting on you" from plain idle.
//!
//! Usage: `quay-hook <working|waiting|idle|ended> [agent] [--pid N]` — the state is the
//! argv, mapped from the hook event in the agent's config (Claude Code:
//! UserPromptSubmit/PostToolUse → working, Notification → waiting, Stop → idle,
//! SessionEnd → ended). `agent` is one of claude|codex|opencode|pi and
//! defaults to `claude` when absent (back-compat with configs installed before
//! multi-agent support). It disambiguates two agents sharing a cwd.
//!
//! `--pid` is for agents whose adapter runs *inside* the agent process (opencode,
//! pi) and therefore already knows it. Claude and codex invoke this as a subprocess
//! and put no pid in their payload, so it is resolved by walking our own ancestry —
//! see [`proc_info`].
//!
//! Standalone std + serde_json + dirs + libc on purpose — importing the app lib would
//! link all of tauri into a helper that runs on every hook event.
//! Always exits 0: a broken helper must never block a Claude Code turn.

use quay_hook::proc_info;

use std::io::Read;
use std::path::PathBuf;

/// Duplicates `store::config_dir()` (can't import the lib, see module doc).
fn state_dir() -> Option<PathBuf> {
	let dir = dirs::data_dir()?.join("am.abhi.quay").join("agent-state");
	std::fs::create_dir_all(&dir).ok()?;
	Some(dir)
}

/// session_id becomes a file name — accept only plain token chars so a
/// malformed payload can't traverse paths.
fn safe_id(id: &str) -> bool {
	!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Cap a session label at 80 chars (char-boundary safe) with an ellipsis —
/// mirrors `agent_radar::truncate_label` (can't import the lib, see module doc).
fn truncate_label(text: &str) -> String {
	let mut label: String = text.chars().take(80).collect();
	if label.len() < text.len() {
		label.push('…');
	}
	label
}

/// The session's display name from a hook payload: the submitted prompt, so a
/// hooked session needs no read of the agent's own log files (privacy). Claude
/// Code's `UserPromptSubmit` carries `prompt`; a couple of fallbacks cover other
/// agents' field names. Skips slash-command envelopes (`<command-name>…`) the
/// same way `first_user_prompt` does. Only events that actually carry a prompt
/// (UserPromptSubmit) yield a name; PostToolUse/idle/waiting return None so the
/// first prompt is preserved by the caller.
fn prompt_name(v: &serde_json::Value) -> Option<String> {
	let raw = v["prompt"].as_str().or_else(|| v["user_prompt"].as_str()).or_else(|| v["message"].as_str())?;
	let text = raw.trim();
	if text.is_empty() || text.starts_with('<') {
		return None;
	}
	Some(truncate_label(text))
}

fn run() -> Option<()> {
	let state = std::env::args().nth(1)?;
	if !matches!(state.as_str(), "working" | "waiting" | "idle" | "ended") {
		return None;
	}
	// Default claude: configs installed before the multi-agent field omit it.
	let agent = std::env::args().nth(2).unwrap_or_else(|| "claude".to_string());
	if !matches!(agent.as_str(), "claude" | "codex" | "opencode" | "pi") {
		return None;
	}
	// The adapter that runs inside the agent knows its own pid; everyone else has to
	// be found by walking up from here.
	let args: Vec<String> = std::env::args().collect();
	let given_pid = args
		.iter()
		.position(|a| a == "--pid")
		.and_then(|i| args.get(i + 1))
		.and_then(|n| n.parse::<u32>().ok());

	let mut input = String::new();
	// Hook payloads are small; cap just in case something pipes a transcript.
	std::io::stdin().take(64 * 1024).read_to_string(&mut input).ok()?;
	let v: serde_json::Value = serde_json::from_str(&input).ok()?;
	let session_id = v["session_id"].as_str()?;
	// Canonicalize so a symlinked cwd (e.g. /var → /private/var) matches the
	// kernel-resolved cwd the radar gets from sysinfo.
	let cwd = std::fs::canonicalize(v["cwd"].as_str()?)
		.map(|p| p.to_string_lossy().into_owned())
		.unwrap_or_else(|_| v["cwd"].as_str().unwrap_or_default().to_string());
	if !safe_id(session_id) {
		return None;
	}
	let dir = state_dir()?;
	let path = dir.join(format!("{session_id}.json"));
	if state == "ended" {
		let _ = std::fs::remove_file(&path);
		return Some(());
	}
	let ts = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.ok()?
		.as_secs();
	// Session name: keep the first prompt seen for this session (matches the
	// radar's "first user prompt" label). Reuse the existing file's name when
	// this event carries none (PostToolUse/idle/waiting) or when one is already
	// set, so a later prompt can't overwrite the first.
	let existing_name = std::fs::read_to_string(&path)
		.ok()
		.and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
		.and_then(|old| old["name"].as_str().map(str::to_string));
	let name = existing_name.or_else(|| prompt_name(&v));
	let mut obj = serde_json::json!({ "agent": agent, "cwd": cwd, "state": state, "ts": ts });
	if let Some(n) = &name {
		obj["name"] = serde_json::Value::String(n.clone());
	}
	// Identity of the session's process, when we can establish it honestly.
	//
	// `pid` alone is not an identity — pids are recycled, so a liveness check would
	// report a *different* process alive under a dead session's number, and across a
	// reboot that is near certain. `startedAt` is what makes the pair identifying.
	//
	// All four fields are optional. A reader that finds no `pid` must fall back to
	// the old `(agent, cwd)` behaviour: configs written by an older helper, and
	// sessions whose agent we could not positively identify, both land there.
	if let Some(proc_) = given_pid.and_then(proc_info::info).or_else(|| {
		proc_info::find_agent(std::process::id(), &agent)
	}) {
		obj["pid"] = serde_json::Value::from(proc_.pid);
		obj["startedAt"] = serde_json::Value::from(proc_.started_at);
		if let Some(tty) = proc_.tty {
			// The radar cannot get a controlling terminal from sysinfo on macOS, and
			// jump-to-session needs one to find the window.
			obj["tty"] = serde_json::Value::String(tty);
		}
	}
	let body = obj.to_string();
	// Temp file is per-session too, so parallel hooks for different sessions
	// can't clobber each other's rename.
	let tmp = dir.join(format!("{session_id}.json.tmp"));
	std::fs::write(&tmp, body).ok()?;
	std::fs::rename(&tmp, &path).ok()?;
	Some(())
}

fn main() {
	// Never a non-zero exit — Claude Code treats those as hook failures.
	let _ = run();
}
