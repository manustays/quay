//! Claude Code hook helper: records per-session state for Quay's agent radar.
//!
//! Registered in `~/.claude/settings.json` (see docs/agent-radar.md), invoked
//! by Claude Code on lifecycle events with the event JSON on stdin. Writes one
//! small file per session under `<data_dir>/am.abhi.quay/agent-state/` that the
//! radar's 5 s poll reads to distinguish "waiting on you" from plain idle.
//!
//! Usage: `quay-hook <working|waiting|idle|ended>` — the state is the argv,
//! mapped from the hook event in settings.json (UserPromptSubmit/PostToolUse →
//! working, Notification → waiting, Stop → idle, SessionEnd → ended).
//!
//! Standalone std + serde_json + dirs on purpose — importing the app lib would
//! link all of tauri into a helper that runs on every hook event.
//! Always exits 0: a broken helper must never block a Claude Code turn.

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

fn run() -> Option<()> {
	let state = std::env::args().nth(1)?;
	if !matches!(state.as_str(), "working" | "waiting" | "idle" | "ended") {
		return None;
	}
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
	let body = serde_json::json!({ "cwd": cwd, "state": state, "ts": ts }).to_string();
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
