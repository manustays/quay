//! Install the `quay-hook` helper and each agent's hook config so the radar
//! gets authoritative working/waiting/idle states instead of the mtime/CPU
//! guess. macOS-only (paths and the app data dir are Apple-specific).
//!
//! The helper is bundled with the app as a resource and copied to a stable,
//! app-managed path — `<data_dir>/bin/quay-hook` — that survives the .app
//! moving or updating. Every agent's config references that one path.

use crate::model::AppError;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const OPENCODE_JS: &str = include_str!("../assets/opencode-quay.js");
const PI_TS: &str = include_str!("../assets/pi-quay.ts");

/// One hook we install: the agent event, an optional matcher (present → written
/// as `"matcher"`), and the quay-hook state arg the event maps to.
struct HookSpec {
	event: &'static str,
	matcher: Option<&'static str>,
	state: &'static str,
}

/// Which `Notification` sub-types actually mean "this session needs you".
///
/// `Notification` is a catch-all: it also fires for `auth_success`,
/// `quota_auto_resume_fired`, `elicitation_complete` and more. Subscribing without
/// a matcher turned every one of those into a waiting agent — a false amber row and
/// a false tray badge.
///
/// `idle_prompt` is deliberately **excluded**. It fires when Claude Code nudges you
/// about a session that has been sitting idle, so including it would flip every
/// finished session to amber a minute after `Stop` already marked it idle, and the
/// tray badge counts waiting agents.
const CLAUDE_NEEDS_YOU: &str =
	"permission_prompt|agent_needs_input|elicitation_dialog|elicitation_url_dialog";

/// Which `SessionStart` sources mean "a session now exists and is at rest".
///
/// `compact` is excluded for the same reason it is on Codex: compaction happens
/// *inside* a turn, so reporting idle there blanks a working row exactly when the
/// agent is busiest. Codex documents this outright; Claude's docs are ambiguous, and
/// the asymmetry settles it — wrongly including `compact` is a visible wrong state,
/// while wrongly excluding it only delays discovery until the session's next event.
///
/// `clear` and `fork` are included: both may hand the session a new id, and a
/// session whose id we have never seen is a session that does not exist as far as
/// the radar is concerned.
const CLAUDE_SESSION_OPENED: &str = "startup|resume|clear|fork";

/// Claude Code (`~/.claude/settings.json`). PostToolUse → working is what clears
/// amber after you approve a permission prompt; `PostToolUseFailure` is the same
/// signal for a tool call that errored, which otherwise left the session looking
/// stale until its next success.
// ponytail: `PermissionRequest` is now a dedicated event and would be a more exact
// waiting signal than the filtered Notification — but its hooks can return an
// allow/deny decision, and this helper exits 0 with empty stdout. Verify that reads
// as "decline to decide" for Claude (it does for codex) before switching; getting it
// wrong would change permission behaviour, not just a status dot.
const CLAUDE_SPECS: &[HookSpec] = &[
	HookSpec { event: "SessionStart", matcher: Some(CLAUDE_SESSION_OPENED), state: "idle" },
	HookSpec { event: "UserPromptSubmit", matcher: None, state: "working" },
	HookSpec { event: "PostToolUse", matcher: Some(""), state: "working" },
	HookSpec { event: "PostToolUseFailure", matcher: Some(""), state: "working" },
	HookSpec { event: "Notification", matcher: Some(CLAUDE_NEEDS_YOU), state: "waiting" },
	HookSpec { event: "Stop", matcher: None, state: "idle" },
	HookSpec { event: "SessionEnd", matcher: None, state: "ended" },
];

/// Which `SessionStart` sources mean "a session now exists and is waiting for you".
///
/// `compact` is deliberately **excluded**: Codex runs `SessionStart` hooks matching
/// `source: "compact"` after it auto-compacts, *before the next model request* — i.e.
/// in the middle of a turn. Mapping that to idle would blank a working row exactly
/// when the agent is busiest.
///
/// `clear` is included. `Stop` has usually marked the session idle by then, so the
/// report is often redundant — but a cleared session may carry a new id, and one the
/// radar has never seen does not exist as far as it is concerned.
const CODEX_SESSION_OPENED: &str = "startup|resume|clear";

/// Codex (`~/.codex/hooks.json`).
///
/// `SessionStart` is what makes a session visible before it does anything — without
/// it a session that opens and sits idle never emits an event, so nothing knows it
/// exists. `SessionEnd` deletes the state file on close; Codex also fires it after
/// 30 minutes of inactivity, which is why a still-running CLI can lose its hook state
/// and fall back to the radar's own view of the process.
///
/// `SessionEnd` takes no matcher on purpose: `reason` is always `other` today, and
/// omitting it keeps us catching any reason Codex adds later.
///
/// A `PermissionRequest` hook can return an allow/deny decision, and this helper
/// exits 0 with empty stdout. Codex documents that as falling through to the normal
/// approval flow, so the prompt still shows — the mapping only observes, it never
/// decides.
const CODEX_SPECS: &[HookSpec] = &[
	HookSpec { event: "SessionStart", matcher: Some(CODEX_SESSION_OPENED), state: "idle" },
	HookSpec { event: "UserPromptSubmit", matcher: None, state: "working" },
	HookSpec { event: "PostToolUse", matcher: Some(""), state: "working" },
	HookSpec { event: "PermissionRequest", matcher: Some(""), state: "waiting" },
	HookSpec { event: "Stop", matcher: None, state: "idle" },
	HookSpec { event: "SessionEnd", matcher: None, state: "ended" },
];

/// Per-agent install state for the Settings pane. Mirrors the TS `HookStatus`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
	pub agent: &'static str,
	pub installed: bool,
}

/// Locate the `quay-hook` helper shipped with the app, to copy to the stable
/// path. In a bundle it is a resource at `<resource_dir>/binaries/quay-hook`;
/// under `tauri dev` the app runs from `target/debug/quay`, so fall back to the
/// release binary that `npm run hook:build` produced at `target/release/`.
pub fn bundled_hook(app: &tauri::AppHandle) -> Option<PathBuf> {
	use tauri::Manager;
	if let Ok(res) = app.path().resource_dir() {
		let p = res.join("binaries/quay-hook");
		if p.exists() {
			return Some(p);
		}
	}
	let exe = std::env::current_exe().ok()?;
	let dev = exe.parent()?.parent()?.join("release/quay-hook");
	dev.exists().then_some(dev)
}

/// Copy the helper to `<data_dir>/bin/quay-hook` (mode 0755) when it is missing
/// or its bytes differ from `src`. Returns the stable path. App-managed: a
/// user-modified copy is overwritten to match the bundled helper.
pub fn install_helper(src: &Path, data_dir: &Path) -> Result<PathBuf, AppError> {
	let bin_dir = data_dir.join("bin");
	std::fs::create_dir_all(&bin_dir)?;
	let dst = bin_dir.join("quay-hook");
	let differs = std::fs::read(&dst).ok() != std::fs::read(src).ok();
	if differs {
		std::fs::copy(src, &dst)?;
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o755))?;
		}
	}
	Ok(dst)
}

/// Write `bytes` to `path` atomically (temp + rename), creating parents.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
	if let Some(p) = path.parent() {
		std::fs::create_dir_all(p)?;
	}
	let tmp = path.with_extension("json.quay-tmp");
	std::fs::write(&tmp, bytes)?;
	std::fs::rename(&tmp, path)?;
	Ok(())
}

fn to_pretty(v: &Value) -> Result<String, AppError> {
	serde_json::to_string_pretty(v).map_err(|e| AppError::Message(e.to_string()))
}

/// Read a JSON config that must be an object; missing or empty → `{}`. A file
/// that exists but isn't valid JSON, or isn't an object, is an error — we never
/// silently overwrite a config we can't understand.
fn read_json_object(path: &Path) -> Result<Value, AppError> {
	match std::fs::read_to_string(path) {
		Ok(s) if s.trim().is_empty() => Ok(json!({})),
		Ok(s) => {
			let v: Value = serde_json::from_str(&s)
				.map_err(|e| AppError::Message(format!("{}: invalid JSON ({e})", path.display())))?;
			if !v.is_object() {
				return Err(AppError::Message(format!("{}: expected a JSON object", path.display())));
			}
			Ok(v)
		}
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
		Err(e) => Err(e.into()),
	}
}

/// True when this hook group contains a command that runs our helper — matched
/// by the exact stable helper path, so it can't touch an unrelated user hook.
fn is_managed_group(g: &Value, helper: &str) -> bool {
	g.get("hooks").and_then(Value::as_array).is_some_and(|hooks| {
		hooks.iter().any(|h| {
			h.get("command").and_then(Value::as_str).is_some_and(|c| c.contains(helper))
		})
	})
}

/// A fresh managed hook group for one event.
fn managed_group(command: &str, matcher: Option<&str>) -> Value {
	let mut g = json!({ "hooks": [ { "type": "command", "command": command, "timeout": 5 } ] });
	if let Some(m) = matcher {
		g["matcher"] = json!(m);
	}
	g
}

/// Merge our hook groups into a Claude/Codex JSON config. Idempotent and
/// path-updating: each event's managed groups are stripped and re-added, so a
/// double install is byte-identical and a helper-path change is corrected.
/// Unknown keys and the user's own hooks are never touched.
fn install_json_hooks(path: &Path, helper: &Path, agent: &str, specs: &[HookSpec]) -> Result<(), AppError> {
	let helper_str = helper.display().to_string();
	let mut root = read_json_object(path)?;
	let obj = root.as_object_mut().unwrap();
	let hooks = obj
		.entry("hooks")
		.or_insert_with(|| json!({}))
		.as_object_mut()
		.ok_or_else(|| AppError::Message(format!("{}: \"hooks\" is not an object", path.display())))?;
	for spec in specs {
		// Path may contain spaces ("Application Support") — quote for the shell.
		let command = format!("\"{}\" {} {}", helper_str, spec.state, agent);
		let arr = hooks
			.entry(spec.event)
			.or_insert_with(|| json!([]))
			.as_array_mut()
			.ok_or_else(|| AppError::Message(format!("{}: \"{}\" is not an array", path.display(), spec.event)))?;
		arr.retain(|g| !is_managed_group(g, &helper_str));
		arr.push(managed_group(&command, spec.matcher));
	}
	write_atomic(path, format!("{}\n", to_pretty(&root)?).as_bytes())
}

/// Remove our hook groups from a Claude/Codex JSON config. Only strips groups we
/// manage and only removes an event key / the `hooks` object when *our* removal
/// emptied it (a user's own empty array is left alone). `delete_if_empty` (codex
/// hooks.json, which is entirely ours) deletes the file when nothing remains.
fn uninstall_json_hooks(path: &Path, helper: &Path, delete_if_empty: bool) -> Result<(), AppError> {
	if !path.exists() {
		return Ok(());
	}
	let helper_str = helper.display().to_string();
	let mut root = read_json_object(path)?;
	let obj = root.as_object_mut().unwrap();
	if let Some(hooks) = obj.get_mut("hooks").and_then(Value::as_object_mut) {
		let events: Vec<String> = hooks.keys().cloned().collect();
		for ev in events {
			if let Some(arr) = hooks.get_mut(&ev).and_then(Value::as_array_mut) {
				let before = arr.len();
				arr.retain(|g| !is_managed_group(g, &helper_str));
				if arr.is_empty() && arr.len() < before {
					hooks.remove(&ev);
				}
			}
		}
		if hooks.is_empty() {
			obj.remove("hooks");
		}
	}
	if delete_if_empty && obj.is_empty() {
		std::fs::remove_file(path)?;
		return Ok(());
	}
	write_atomic(path, format!("{}\n", to_pretty(&root)?).as_bytes())
}

fn remove_if_exists(path: &Path) -> Result<(), AppError> {
	match std::fs::remove_file(path) {
		Ok(()) => Ok(()),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
		Err(e) => Err(e.into()),
	}
}

fn claude_config(home: &Path) -> PathBuf { home.join(".claude/settings.json") }
fn codex_config(home: &Path) -> PathBuf { home.join(".codex/hooks.json") }
fn opencode_plugin(home: &Path) -> PathBuf { home.join(".config/opencode/plugin/quay.js") }
fn pi_extension(home: &Path) -> PathBuf { home.join(".pi/agent/extensions/quay.ts") }

/// Install the hook config for one agent. The helper binary must already be at
/// `helper` (installed by `install_helper`) — every agent references it: claude
/// and codex from their JSON configs, opencode and pi from their plugins.
pub fn install(agent: &str, home: &Path, helper: &Path) -> Result<(), AppError> {
	match agent {
		"claude" => install_json_hooks(&claude_config(home), helper, "claude", CLAUDE_SPECS),
		"codex" => install_json_hooks(&codex_config(home), helper, "codex", CODEX_SPECS),
		"opencode" => write_atomic(&opencode_plugin(home), OPENCODE_JS.as_bytes()),
		"pi" => write_atomic(&pi_extension(home), PI_TS.as_bytes()),
		_ => Err(AppError::Message(format!("unknown agent: {agent}"))),
	}
}

/// Remove one agent's hook config. Leaves the shared helper binary in place —
/// other agents may still reference it. `helper` is the stable path used to
/// recognize the commands we manage.
pub fn uninstall(agent: &str, home: &Path, helper: &Path) -> Result<(), AppError> {
	match agent {
		"claude" => uninstall_json_hooks(&claude_config(home), helper, false),
		"codex" => uninstall_json_hooks(&codex_config(home), helper, true),
		"opencode" => remove_if_exists(&opencode_plugin(home)),
		"pi" => remove_if_exists(&pi_extension(home)),
		_ => Err(AppError::Message(format!("unknown agent: {agent}"))),
	}
}

/// Re-apply the hook config of every agent that already has one.
///
/// [`install`] is idempotent and path-updating by design, so this is safe to run on
/// every launch. It exists because the *content* of what we install changes between
/// app versions — a corrected event matcher, a fixed plugin — and without this those
/// corrections would only reach a user who happened to toggle the hook off and on in
/// Settings. Same rule as the helper refresh: **never installs for an agent that has
/// none**, so it can't opt anyone in behind their back.
///
/// Returns the agents it refreshed, for the caller to trace.
pub fn refresh_installed(home: &Path, data_dir: &Path) -> Vec<&'static str> {
	let helper = data_dir.join("bin/quay-hook");
	if !helper.exists() {
		return Vec::new();
	}
	statuses(home, data_dir)
		.into_iter()
		.filter(|s| s.installed)
		.filter(|s| install(s.agent, home, &helper).is_ok())
		.map(|s| s.agent)
		.collect()
}

fn file_contains(path: &Path, needle: &str) -> bool {
	std::fs::read_to_string(path).is_ok_and(|s| s.contains(needle))
}

/// Install state for all four agents, for the Settings pane. claude/codex need
/// both the helper binary present and their config referencing it by its exact
/// stable path; opencode/pi are just the presence of the plugin file (which
/// references the helper by absolute path).
pub fn statuses(home: &Path, data_dir: &Path) -> Vec<HookStatus> {
	let helper = data_dir.join("bin/quay-hook");
	let helper_str = helper.display().to_string();
	let json_ok = |p: PathBuf| helper.exists() && file_contains(&p, &helper_str);
	vec![
		HookStatus { agent: "claude", installed: json_ok(claude_config(home)) },
		HookStatus { agent: "codex", installed: json_ok(codex_config(home)) },
		HookStatus { agent: "opencode", installed: opencode_plugin(home).exists() },
		HookStatus { agent: "pi", installed: pi_extension(home).exists() },
	]
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tmp() -> PathBuf {
		let d = std::env::temp_dir().join(format!("quay-hooks-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&d).unwrap();
		d
	}

	#[test]
	fn install_helper_copies_sets_exec_and_recopies_on_diff() {
		let d = tmp();
		let src = d.join("src-hook");
		std::fs::write(&src, b"v1").unwrap();
		let data = d.join("data");

		let dst = install_helper(&src, &data).unwrap();
		assert_eq!(std::fs::read(&dst).unwrap(), b"v1");
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			assert_eq!(std::fs::metadata(&dst).unwrap().permissions().mode() & 0o777, 0o755);
		}

		// Same bytes → no error, still present.
		install_helper(&src, &data).unwrap();
		assert_eq!(std::fs::read(&dst).unwrap(), b"v1");

		// Changed source → re-copied.
		std::fs::write(&src, b"v2-longer").unwrap();
		install_helper(&src, &data).unwrap();
		assert_eq!(std::fs::read(&dst).unwrap(), b"v2-longer");

		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn claude_install_is_idempotent_and_preserves_user_keys() {
		let d = tmp();
		let cfg = d.join(".claude/settings.json");
		std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
		// User config: an unrelated top-level key and their own hook.
		std::fs::write(
			&cfg,
			r#"{"model":"opus","hooks":{"Stop":[{"hooks":[{"type":"command","command":"my-own-thing"}]}]}}"#,
		)
		.unwrap();
		let helper = PathBuf::from("/Users/x/Library/Application Support/am.abhi.quay/bin/quay-hook");

		let helper_str = helper.display().to_string();
		install_json_hooks(&cfg, &helper, "claude", CLAUDE_SPECS).unwrap();
		let once = std::fs::read_to_string(&cfg).unwrap();
		// User's key and their own Stop hook both survive.
		assert!(once.contains("\"model\": \"opus\""));
		assert!(once.contains("my-own-thing"));
		// Our events are present, commands reference the helper + agent tag.
		assert!(once.contains("UserPromptSubmit"));
		assert!(once.contains("SessionEnd"));
		assert!(once.contains(&helper_str));
		assert!(once.contains("working claude"));

		// Second install is byte-identical (no duplicate groups).
		install_json_hooks(&cfg, &helper, "claude", CLAUDE_SPECS).unwrap();
		assert_eq!(std::fs::read_to_string(&cfg).unwrap(), once);
		// Exactly one managed Stop group alongside the user's.
		let v: Value = serde_json::from_str(&once).unwrap();
		let stop = v["hooks"]["Stop"].as_array().unwrap();
		assert_eq!(stop.len(), 2);
		assert_eq!(stop.iter().filter(|g| is_managed_group(g, &helper_str)).count(), 1);

		// Uninstall strips only our groups; user's Stop hook and key remain.
		// (The stable helper path is fixed in production, so uninstall matches
		// the same path install wrote.)
		uninstall_json_hooks(&cfg, &helper, false).unwrap();
		let after = std::fs::read_to_string(&cfg).unwrap();
		assert!(!after.contains(&helper_str));
		assert!(after.contains("my-own-thing"));
		assert!(after.contains("\"model\": \"opus\""));
		let va: Value = serde_json::from_str(&after).unwrap();
		assert_eq!(va["hooks"]["Stop"].as_array().unwrap().len(), 1);
		// Events that were entirely ours are gone.
		assert!(va["hooks"].get("UserPromptSubmit").is_none());

		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn codex_creates_and_deletes_its_own_file() {
		let d = tmp();
		let cfg = codex_config(&d);
		let helper = PathBuf::from("/x/am.abhi.quay/bin/quay-hook");
		// Create-if-missing.
		assert!(!cfg.exists());
		install_json_hooks(&cfg, &helper, "codex", CODEX_SPECS).unwrap();
		assert!(cfg.exists());
		assert!(std::fs::read_to_string(&cfg).unwrap().contains("PermissionRequest"));
		// Uninstall of an all-ours file removes it entirely.
		uninstall_json_hooks(&cfg, &helper, true).unwrap();
		assert!(!cfg.exists());
		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn non_object_config_is_rejected() {
		let d = tmp();
		let cfg = d.join("settings.json");
		std::fs::write(&cfg, "[1,2,3]").unwrap();
		let helper = PathBuf::from("/x/am.abhi.quay/bin/quay-hook");
		assert!(install_json_hooks(&cfg, &helper, "claude", CLAUDE_SPECS).is_err());
		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn statuses_reflect_install_state() {
		let d = tmp();
		let home = d.join("home");
		let data = d.join("data");
		std::fs::create_dir_all(&home).unwrap();
		// Nothing installed → all false.
		assert!(statuses(&home, &data).iter().all(|s| !s.installed));

		// Helper present + claude config referencing it → claude installed.
		let helper = install_helper(&{ let p = d.join("h"); std::fs::write(&p, b"x").unwrap(); p }, &data).unwrap();
		install(&"claude".to_string(), &home, &helper).unwrap();
		install(&"opencode".to_string(), &home, &helper).unwrap();
		let st = statuses(&home, &data);
		let get = |a: &str| st.iter().find(|s| s.agent == a).unwrap().installed;
		assert!(get("claude"));
		assert!(get("opencode"));
		assert!(!get("codex"));
		assert!(!get("pi"));
		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn claude_notification_hook_is_filtered_to_events_that_need_you() {
		// Regression: subscribing to Notification with no matcher made every
		// notification a waiting agent — auth_success, quota auto-resume and the
		// elicitation_complete/response pair included — so rows went amber and the
		// tray badge counted sessions that wanted nothing.
		let d = tmp();
		let cfg = d.join(".claude/settings.json");
		std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
		let helper = PathBuf::from("/tmp/quay-hook");
		install_json_hooks(&cfg, &helper, "claude", CLAUDE_SPECS).unwrap();

		let v: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
		let groups = v["hooks"]["Notification"].as_array().unwrap();
		assert_eq!(groups.len(), 1);
		let matcher = groups[0]["matcher"].as_str().expect("Notification must carry a matcher");

		for needs_you in ["permission_prompt", "agent_needs_input", "elicitation_dialog"] {
			assert!(matcher.contains(needs_you), "{needs_you} should mark the session waiting");
		}
		for noise in ["auth_success", "quota_auto_resume_fired", "elicitation_complete"] {
			assert!(!matcher.contains(noise), "{noise} must not mark the session waiting");
		}
		// `idle_prompt` is a nudge about an already-idle session; Stop has marked it
		// idle already, so counting it would badge every finished session.
		assert!(!matcher.contains("idle_prompt"), "idle_prompt must not count as waiting");

		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn pi_extension_settles_before_reporting_idle() {
		// `agent_end` fires when a run ends, but pi may auto-retry or continue, so
		// the row flashed idle mid-work. `agent_settled` is the one that means done.
		assert!(PI_TS.contains("agent_settled"), "pi must report idle on agent_settled");
		assert!(
			!PI_TS.contains("\"agent_end\""),
			"agent_end is premature — pi retries and continues after it"
		);
	}

	#[test]
	fn refresh_updates_an_installed_config_but_never_opts_anyone_in() {
		// The content we install changes between app versions. Without this refresh a
		// corrected matcher would only reach someone who toggled the hook in Settings.
		let d = tmp();
		let data = d.join("data");
		std::fs::create_dir_all(data.join("bin")).unwrap();
		let helper = data.join("bin/quay-hook");
		std::fs::write(&helper, b"#!/bin/sh\n").unwrap();

		// claude is opted in, but with a stale group: no matcher on Notification.
		let cfg = d.join(".claude/settings.json");
		std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
		let stale = format!(
			r#"{{"hooks":{{"Notification":[{{"hooks":[{{"type":"command","command":"\"{}\" waiting claude"}}]}}]}}}}"#,
			helper.display()
		);
		std::fs::write(&cfg, &stale).unwrap();

		let refreshed = refresh_installed(&d, &data);
		assert_eq!(refreshed, vec!["claude"], "only the opted-in agent is refreshed");

		let v: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
		assert!(
			v["hooks"]["Notification"][0]["matcher"].is_string(),
			"the stale unmatched group must be corrected in place"
		);
		// Agents that were never opted in stay that way — no file created.
		assert!(!d.join(".codex/hooks.json").exists());
		assert!(!d.join(".config/opencode/plugin/quay.js").exists());
		assert!(!d.join(".pi/agent/extensions/quay.ts").exists());

		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn refresh_is_a_no_op_without_the_helper() {
		// Nobody has opted in, so nothing should be written anywhere.
		let d = tmp();
		let data = d.join("data");
		std::fs::create_dir_all(&data).unwrap();
		assert!(refresh_installed(&d, &data).is_empty());
		assert!(!d.join(".claude/settings.json").exists());
		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn codex_session_start_ignores_mid_turn_compaction() {
		// Codex runs SessionStart hooks matching source "compact" after it
		// auto-compacts, before the next model request — mid-turn. Matching it would
		// mark a busy session idle, the same bug pi had with agent_end.
		let d = tmp();
		let cfg = d.join(".codex/hooks.json");
		std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
		let helper = PathBuf::from("/tmp/quay-hook");
		install_json_hooks(&cfg, &helper, "codex", CODEX_SPECS).unwrap();

		let v: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
		let start = v["hooks"]["SessionStart"].as_array().unwrap();
		assert_eq!(start.len(), 1);
		let matcher = start[0]["matcher"].as_str().expect("SessionStart must be filtered");
		assert!(matcher.contains("startup"), "a fresh session must become visible");
		assert!(matcher.contains("resume"), "a resumed session must become visible");
		assert!(!matcher.contains("compact"), "compaction happens mid-turn, not at rest");

		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn codex_session_end_clears_state_and_matches_every_reason() {
		// The old spec had no SessionEnd at all — on the false premise that Codex
		// lacks the event — so every closed session leaked its state file until the
		// orphan sweep noticed. `reason` is always "other" today; omitting the
		// matcher keeps us catching whatever Codex adds later.
		let d = tmp();
		let cfg = d.join(".codex/hooks.json");
		std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
		let helper = PathBuf::from("/tmp/quay-hook");
		install_json_hooks(&cfg, &helper, "codex", CODEX_SPECS).unwrap();

		let v: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
		let end = v["hooks"]["SessionEnd"].as_array().unwrap();
		assert_eq!(end.len(), 1);
		assert!(end[0].get("matcher").is_none(), "SessionEnd must match every reason");
		let command = end[0]["hooks"][0]["command"].as_str().unwrap();
		assert!(command.contains("ended codex"), "SessionEnd must clear the state file");

		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn claude_session_start_ignores_compaction_but_catches_clear_and_fork() {
		// Compaction happens inside a turn, so reporting idle there blanks a working
		// row. `clear` and `fork` may hand the session a new id, and a session id the
		// radar has never seen is one it does not know exists.
		let d = tmp();
		let cfg = d.join(".claude/settings.json");
		std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
		let helper = PathBuf::from("/tmp/quay-hook");
		install_json_hooks(&cfg, &helper, "claude", CLAUDE_SPECS).unwrap();

		let v: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
		let start = v["hooks"]["SessionStart"].as_array().unwrap();
		assert_eq!(start.len(), 1);
		let matcher = start[0]["matcher"].as_str().expect("SessionStart must be filtered");
		for opened in ["startup", "resume", "clear", "fork"] {
			assert!(matcher.contains(opened), "{opened} opens a session the radar must see");
		}
		assert!(!matcher.contains("compact"), "compaction is mid-turn, not a session at rest");

		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn a_failed_tool_call_still_marks_the_session_working() {
		// PostToolUse fires only on success, so a session whose tool call errored got
		// no refresh and looked stale until its next successful call.
		let d = tmp();
		let cfg = d.join(".claude/settings.json");
		std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
		let helper = PathBuf::from("/tmp/quay-hook");
		install_json_hooks(&cfg, &helper, "claude", CLAUDE_SPECS).unwrap();

		let v: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
		let failed = v["hooks"]["PostToolUseFailure"].as_array().unwrap();
		assert_eq!(failed.len(), 1);
		assert_eq!(failed[0]["matcher"], "", "every tool, not a subset");
		assert!(failed[0]["hooks"][0]["command"].as_str().unwrap().contains("working claude"));

		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn a_cleared_codex_session_is_still_discovered() {
		// Same reasoning as Claude's `clear`: the session may come back with a new id.
		let d = tmp();
		let cfg = d.join(".codex/hooks.json");
		std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
		let helper = PathBuf::from("/tmp/quay-hook");
		install_json_hooks(&cfg, &helper, "codex", CODEX_SPECS).unwrap();

		let v: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
		let matcher = v["hooks"]["SessionStart"][0]["matcher"].as_str().unwrap();
		assert!(matcher.contains("clear"));
		assert!(!matcher.contains("compact"));

		std::fs::remove_dir_all(&d).ok();
	}
}
