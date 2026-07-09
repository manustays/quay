use crate::model::AppError;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// How a terminal is launched.
enum Strategy {
	/// AppleScript `do script` (Terminal.app).
	AppleDoScript,
	/// AppleScript `write text` to a new iTerm session.
	AppleWriteText,
	/// GUI app run via `open -na "<App>" --args <args_before_shell…> <shell>`,
	/// or the bare CLI binary + the same prefix when only the CLI is on PATH.
	Cli { args_before_shell: &'static [&'static str] },
}

/// A supported terminal: the display `name` (also what `Settings::terminal_app`
/// stores), its macOS `.app` bundle file name, an optional CLI binary name for
/// the `command -v` detection fallback, the launch `strategy`, and `always`
/// (true only for Terminal.app, which ships with macOS).
struct TermSpec {
	name: &'static str,
	app_bundle: &'static str,
	cli_bin: Option<&'static str>,
	strategy: Strategy,
	always: bool,
}

/// All terminals Quay knows how to detect and launch. Single source of truth.
fn registry() -> &'static [TermSpec] {
	&[
		TermSpec { name: "Terminal", app_bundle: "Terminal.app", cli_bin: None,
			strategy: Strategy::AppleDoScript, always: true },
		TermSpec { name: "iTerm", app_bundle: "iTerm.app", cli_bin: None,
			strategy: Strategy::AppleWriteText, always: false },
		TermSpec { name: "Ghostty", app_bundle: "Ghostty.app", cli_bin: Some("ghostty"),
			strategy: Strategy::Cli { args_before_shell: &["-e"] }, always: false },
		TermSpec { name: "Kitty", app_bundle: "kitty.app", cli_bin: Some("kitty"),
			strategy: Strategy::Cli { args_before_shell: &[] }, always: false },
		TermSpec { name: "WezTerm", app_bundle: "WezTerm.app", cli_bin: Some("wezterm"),
			strategy: Strategy::Cli { args_before_shell: &["start", "--"] }, always: false },
		TermSpec { name: "Alacritty", app_bundle: "Alacritty.app", cli_bin: Some("alacritty"),
			strategy: Strategy::Cli { args_before_shell: &["-e"] }, always: false },
	]
}

/// Look up a terminal spec by its display name.
fn spec(name: &str) -> Option<&'static TermSpec> {
	registry().iter().find(|t| t.name == name)
}

/// The interactive login-shell command that runs `line` and then re-execs an
/// interactive shell so the window stays open afterwards (parity with
/// Terminal.app's `do script`). `-ilc` sources `~/.zshrc`, so PATH resolves.
fn keepalive_shell(line: &str) -> [String; 3] {
	[
		"/bin/zsh".into(),
		"-ilc".into(),
		format!("{line}; exec /bin/zsh -il"),
	]
}

/// Argv for launching a GUI terminal via `open -na "<open_name>" --args …`.
fn cli_open_argv(open_name: &str, args_before_shell: &[&str], line: &str) -> Vec<String> {
	let mut v = vec!["-na".to_string(), open_name.to_string(), "--args".to_string()];
	v.extend(args_before_shell.iter().map(|s| s.to_string()));
	v.extend(keepalive_shell(line));
	v
}

/// Argv for launching a terminal by its bare CLI binary (PATH fallback): the
/// per-terminal prefix followed by the keep-alive shell.
fn cli_bin_argv(args_before_shell: &[&str], line: &str) -> Vec<String> {
	let mut v: Vec<String> = args_before_shell.iter().map(|s| s.to_string()).collect();
	v.extend(keepalive_shell(line));
	v
}

/// True if `<bundle>` exists in `/Applications` or `~/Applications`.
fn app_bundle_installed(bundle: &str) -> bool {
	let home = std::env::var("HOME").unwrap_or_default();
	std::path::Path::new("/Applications").join(bundle).exists()
		|| std::path::Path::new(&home).join("Applications").join(bundle).exists()
}

/// Absolute path of `bin` per the login shell, or `None` if not found. Mirrors
/// the `command -v` probe used by `brew`/`docker` resolution.
fn resolve_cli(bin: &str) -> Option<String> {
	let out = Command::new("/bin/zsh")
		.args(["-lc", &format!("command -v {bin}")])
		.output()
		.ok()?;
	if !out.status.success() {
		return None;
	}
	let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
	(!p.is_empty()).then_some(p)
}

/// Display names of every supported terminal that is installed: Terminal.app
/// (always), any whose `.app` bundle is present, or whose CLI binary is on the
/// login-shell PATH (covers brew-installed CLIs with no `.app`).
pub fn installed_terminals() -> Vec<String> {
	registry()
		.iter()
		.filter(|t| {
			t.always
				|| app_bundle_installed(t.app_bundle)
				|| t.cli_bin.map(|b| resolve_cli(b).is_some()).unwrap_or(false)
		})
		.map(|t| t.name.to_string())
		.collect()
}

/// Run `osascript -e <script>`, returning stderr text on failure.
fn run_osascript(script: &str) -> Result<(), AppError> {
	run_command("osascript", &["-e".to_string(), script.to_string()])
}

/// Run `osascript -e <script>` and return its stdout (trimmed).
fn run_osascript_out(script: &str) -> Result<String, AppError> {
	let out = Command::new("osascript")
		.args(["-e", script])
		.output()
		.map_err(|e| AppError::Message(format!("osascript failed: {e}")))?;
	if out.status.success() {
		Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
	} else {
		Err(AppError::Message(String::from_utf8_lossy(&out.stderr).trim().to_string()))
	}
}

/// AppleScript that focuses the iTerm session on `dev` (a `/dev/ttysNNN`
/// path), echoing "ok" on a hit and "miss" otherwise. `is running` guards
/// against `tell` launching the app. Pure.
fn focus_script_iterm(dev: &str) -> String {
	format!(
		"if application \"iTerm\" is running then\n\
		 tell application \"iTerm\"\n\
		 repeat with w in windows\n\
		 repeat with t in tabs of w\n\
		 repeat with s in sessions of t\n\
		 if tty of s is \"{dev}\" then\n\
		 select s\n select t\n select w\n activate\n return \"ok\"\n\
		 end if\n\
		 end repeat\n end repeat\n end repeat\n\
		 end tell\n\
		 end if\n\
		 return \"miss\""
	)
}

/// Same for Terminal.app tabs (their `tty` property is the `/dev/…` path). Pure.
fn focus_script_terminal(dev: &str) -> String {
	format!(
		"if application \"Terminal\" is running then\n\
		 tell application \"Terminal\"\n\
		 repeat with w in windows\n\
		 repeat with t in tabs of w\n\
		 if tty of t is \"{dev}\" then\n\
		 set selected of t to true\n set frontmost of w to true\n activate\n return \"ok\"\n\
		 end if\n\
		 end repeat\n end repeat\n\
		 end tell\n\
		 end if\n\
		 return \"miss\""
	)
}

/// Resolve a host-control CLI: prefer the app-bundled binary (works without
/// PATH setup), fall back to the login-shell PATH.
fn host_cli(bundled: &str, bin: &str) -> Result<String, AppError> {
	if Path::new(bundled).exists() {
		Ok(bundled.to_string())
	} else {
		resolve_cli(bin).ok_or_else(|| AppError::Message(format!("{bin} CLI not found")))
	}
}

/// Focus a supacode surface via its CLI (`supacode tab focus` + `surface
/// focus` + `open` to bring the app frontmost). IDs come from the session's
/// own environment, so they are exact — no tty matching involved.
pub fn focus_supacode(worktree: &str, tab: &str, surface: &str) -> Result<(), AppError> {
	let bin = host_cli("/Applications/supacode.app/Contents/Resources/bin/supacode", "supacode")?;
	let arg = |s: &str| s.to_string();
	run_command(&bin, &[
		arg("tab"), arg("focus"), arg("-w"), arg(worktree), arg("-t"), arg(tab),
	])?;
	run_command(&bin, &[
		arg("surface"), arg("focus"), arg("-w"), arg(worktree), arg("-t"), arg(tab),
		arg("-s"), arg(surface),
	])?;
	run_command(&bin, &[arg("open")])
}

/// Focus a kitty window via its remote-control socket:
/// `kitten @ --to <socket> focus-window --match id:<window_id>`. Raises the
/// OS window itself; `open -a kitty` is belt-and-braces for macOS 15+'s
/// cooperative activation, tolerated failing (CLI-only installs).
pub fn focus_kitty(socket: &str, window_id: &str) -> Result<(), AppError> {
	if !window_id.chars().all(|c| c.is_ascii_digit()) {
		return Err(AppError::Message(format!("invalid kitty window id: {window_id}")));
	}
	let bin = host_cli("/Applications/kitty.app/Contents/MacOS/kitten", "kitten")?;
	run_command(&bin, &[
		"@".into(), "--to".into(), socket.into(),
		"focus-window".into(), "--match".into(), format!("id:{window_id}"),
	])?;
	let _ = run_command("open", &["-a".into(), "kitty".into()]);
	Ok(())
}

/// Focus a wezterm pane: `wezterm cli activate-pane --pane-id <pane>` against
/// the session's socket. activate-pane only switches the mux's internal
/// focus — `open -a WezTerm` raises the app (the activated window fronts).
pub fn focus_wezterm(socket: &str, pane: &str) -> Result<(), AppError> {
	if !pane.chars().all(|c| c.is_ascii_digit()) {
		return Err(AppError::Message(format!("invalid wezterm pane id: {pane}")));
	}
	let bin = host_cli("/Applications/WezTerm.app/Contents/MacOS/wezterm", "wezterm")?;
	run_command_env(
		&bin,
		&["cli".into(), "activate-pane".into(), "--pane-id".into(), pane.into()],
		&[("WEZTERM_UNIX_SOCKET", socket)],
	)?;
	let _ = run_command("open", &["-a".into(), "WezTerm".into()]);
	Ok(())
}

/// True for herdr's public id charset (cctop's sanitization; ids look like
/// `w1:p2`). Guards the ids before they become CLI args.
fn herdr_id_ok(id: &str) -> bool {
	!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || ":.@_%-".contains(c))
}

/// Focus a herdr pane: `herdr agent focus <pane>` against the session's API
/// socket. Only switches focus *inside* herdr — the caller raises the host
/// terminal window separately (see `jump_to_session`).
///
/// `agent focus` errors with `agent_not_found` when herdr hasn't recognized
/// the pane's process as an agent (verified live) — then fall back to
/// focusing the pane's workspace + tab by id, which lands the user one
/// keystroke away.
pub fn focus_herdr(socket: &str, pane: &str, workspace: &str, tab: &str) -> Result<(), AppError> {
	if ![pane, workspace, tab].into_iter().all(herdr_id_ok) {
		return Err(AppError::Message(format!("invalid herdr ids: {workspace} {tab} {pane}")));
	}
	let bin = host_cli("/opt/homebrew/bin/herdr", "herdr")?;
	let env = [("HERDR_SOCKET_PATH", socket)];
	if run_command_env(&bin, &["agent".into(), "focus".into(), pane.into()], &env).is_ok() {
		return Ok(());
	}
	run_command_env(&bin, &["workspace".into(), "focus".into(), workspace.into()], &env)?;
	run_command_env(&bin, &["tab".into(), "focus".into(), tab.into()], &env)
}

/// AppleScript that focuses the Ghostty terminal matching `field is "value"`,
/// echoing ok/miss like the tty scripts. `focus` fronts the window (Ghostty
/// ≥1.3 dictionary); `activate` makes the app frontmost. Pure.
fn focus_script_ghostty(field: &str, value: &str) -> String {
	let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
	format!(
		"if application \"Ghostty\" is running then\n\
		 tell application \"Ghostty\"\n\
		 repeat with w in windows\n\
		 repeat with t in tabs of w\n\
		 repeat with s in terminals of t\n\
		 if {field} of s is \"{escaped}\" then\n\
		 focus s\n activate\n return \"ok\"\n\
		 end if\n\
		 end repeat\n end repeat\n end repeat\n\
		 end tell\n\
		 end if\n\
		 return \"miss\""
	)
}

/// Focus the Ghostty terminal hosting a session. Ghostty ≥1.4 exposes a `tty`
/// property — exact match; 1.3.x doesn't (the script errors on the property),
/// so fall back to matching `working directory` against the session's cwd.
// ponytail: cwd fallback is ambiguous when two Ghostty terminals share a
// folder; first match wins. tty match takes over once 1.4 is the norm.
pub fn focus_ghostty(tty: &str, cwd: &str) -> Result<(), AppError> {
	if tty.starts_with("tty") && tty.chars().all(|c| c.is_ascii_alphanumeric()) {
		let by_tty = focus_script_ghostty("tty", &format!("/dev/{tty}"));
		if let Ok(out) = run_osascript_out(&by_tty) {
			if out == "ok" {
				return Ok(());
			}
			// "miss" on 1.4+ means the tty truly isn't there — still try cwd,
			// harmless and covers surfaces whose tty the dictionary hides.
		}
		// Err(_): 1.3.x without the tty property — fall through to cwd.
	}
	match run_osascript_out(&focus_script_ghostty("working directory", cwd))?.as_str() {
		"ok" => Ok(()),
		_ => Err(AppError::Message("session's Ghostty window not found".into())),
	}
}

/// Focus the terminal window/tab whose controlling tty is `tty` ("ttys002").
///
/// A tty belongs to exactly one window, so both scriptable apps are simply
/// tried in turn; first "ok" wins. Errors when neither has a matching tab
/// (e.g. the session lives in a non-scriptable terminal).
// ponytail: Terminal.app + iTerm only; per-terminal adapters (kitty remote
// control, tmux select-window) are the upgrade path.
pub fn focus_by_tty(tty: &str) -> Result<(), AppError> {
	// tty comes from `ps`; reject anything that isn't a plain tty name so the
	// AppleScript string can't be escaped/injected.
	if !tty.starts_with("tty") || !tty.chars().all(|c| c.is_ascii_alphanumeric()) {
		return Err(AppError::Message(format!("invalid tty: {tty}")));
	}
	let dev = format!("/dev/{tty}");
	for script in [focus_script_iterm(&dev), focus_script_terminal(&dev)] {
		if run_osascript_out(&script)? == "ok" {
			return Ok(());
		}
	}
	Err(AppError::Message("session's terminal window not found".into()))
}

/// Spawn `program` with `args`, mapping a non-zero exit to its stderr.
fn run_command(program: &str, args: &[String]) -> Result<(), AppError> {
	run_command_env(program, args, &[])
}

/// `run_command` with extra environment variables (socket paths for the
/// wezterm/herdr control CLIs).
fn run_command_env(
	program: &str,
	args: &[String],
	envs: &[(&str, &str)],
) -> Result<(), AppError> {
	let out = Command::new(program)
		.args(args)
		.envs(envs.iter().map(|&(k, v)| (k, v)))
		.output()
		.map_err(|e| AppError::Message(format!("{program} failed: {e}")))?;
	if out.status.success() {
		Ok(())
	} else {
		Err(AppError::Message(String::from_utf8_lossy(&out.stderr).trim().to_string()))
	}
}

/// Build the shell line run inside the terminal: (optional pid capture) + cd +
/// env exports + command.
///
/// When `pidfile` is set, the line starts with `echo $$ > '<pidfile>'` so the
/// terminal's controlling shell records its own PID — `$$` is the PID of the shell
/// that owns the tab, which lives until the window/tab is closed (it survives the
/// `keepalive_shell` `exec`, since `exec` keeps the PID). The app polls that PID's
/// liveness so a closed terminal flips to Stopped.
///
/// Single-quote-escapes `dir`, the pidfile path, and each env value so that paths
/// and values containing single quotes are handled correctly (using `'\''`).
pub fn build_command_line(
	dir: &str,
	env: &BTreeMap<String, String>,
	cmd: &str,
	pidfile: Option<&Path>,
) -> String {
	let mut parts = Vec::new();
	if let Some(pf) = pidfile {
		parts.push(format!("echo $$ > '{}'", pf.to_string_lossy().replace('\'', "'\\''")));
	}
	parts.push(format!("cd '{}'", dir.replace('\'', "'\\''")));
	for (k, v) in env {
		parts.push(format!("export {}='{}'", k, v.replace('\'', "'\\''")));
	}
	parts.push(cmd.to_string());
	parts.join(" && ")
}

/// Open a terminal window already cd'd into `dir` and run `clear`.
///
/// `app_name` is the terminal display name matching `Settings::terminal_app`
/// (e.g. `"Terminal"`, `"iTerm"`, `"Ghostty"`).
pub fn open_folder(app_name: &str, dir: &str) -> Result<(), AppError> {
	run_in_terminal(app_name, dir, &BTreeMap::new(), "clear", None)
}

/// Open a terminal window and run `cmd` in `dir` with the given env exports.
///
/// Dispatches via the terminal registry: AppleScript strategies for
/// Terminal.app and iTerm, CLI launch (via `open -na` or bare binary) for
/// GPU terminals (Ghostty, Kitty, WezTerm, Alacritty).
/// Returns `AppError` containing stderr on failure.
pub fn run_in_terminal(
	app_name: &str,
	dir: &str,
	env: &BTreeMap<String, String>,
	cmd: &str,
	pidfile: Option<&Path>,
) -> Result<(), AppError> {
	let line = build_command_line(dir, env, cmd, pidfile);
	let spec = spec(app_name)
		.ok_or_else(|| AppError::Message(format!("unknown terminal: {app_name}")))?;
	match &spec.strategy {
		Strategy::AppleDoScript | Strategy::AppleWriteText => {
			let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
			// `matches!` borrows — don't move the non-Copy strategy out of `&spec`.
			let script = if matches!(spec.strategy, Strategy::AppleWriteText) {
				format!("tell application \"iTerm\"\n create window with default profile\n tell current session of current window to write text \"{escaped}\"\nend tell")
			} else {
				format!("tell application \"Terminal\"\n activate\n do script \"{escaped}\"\nend tell")
			};
			run_osascript(&script)
		}
		Strategy::Cli { args_before_shell } => {
			if app_bundle_installed(spec.app_bundle) {
				let open_name = spec.app_bundle.strip_suffix(".app").unwrap_or(spec.app_bundle);
				run_command("open", &cli_open_argv(open_name, args_before_shell, &line))
			} else if let Some(bin) = spec.cli_bin.and_then(resolve_cli) {
				run_command(&bin, &cli_bin_argv(args_before_shell, &line))
			} else {
				Err(AppError::Message(format!("{} is not installed", spec.name)))
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::BTreeMap;

	#[test]
	fn builds_command_with_env_exports() {
		let mut env = BTreeMap::new();
		env.insert("FOO".to_string(), "bar".to_string());
		let line = build_command_line("/tmp/app", &env, "npm run dev", None);
		assert!(line.starts_with("cd '/tmp/app'"));
		assert!(line.contains("export FOO='bar'"));
		assert!(line.ends_with("npm run dev"));
	}

	#[test]
	fn pidfile_prepends_pid_capture_without_touching_command() {
		let env = BTreeMap::new();
		let pf = Path::new("/tmp/quay/logs/abc.term.pid");
		let line = build_command_line("/tmp/app", &env, "cd sub && npm run dev", Some(pf));
		// PID capture is the first segment; the command is preserved verbatim.
		assert!(line.starts_with("echo $$ > '/tmp/quay/logs/abc.term.pid' && cd '/tmp/app'"));
		assert!(line.ends_with("cd sub && npm run dev"));
	}

	#[test]
	fn keepalive_shell_runs_line_then_keeps_open() {
		let s = keepalive_shell("cd '/tmp' && npm run dev");
		assert_eq!(s[0], "/bin/zsh");
		assert_eq!(s[1], "-ilc");
		assert_eq!(s[2], "cd '/tmp' && npm run dev; exec /bin/zsh -il");
	}

	#[test]
	fn cli_open_argv_wraps_open_na_with_prefix() {
		let v = cli_open_argv("WezTerm", &["start", "--"], "echo hi");
		assert_eq!(
			v,
			vec![
				"-na", "WezTerm", "--args", "start", "--",
				"/bin/zsh", "-ilc", "echo hi; exec /bin/zsh -il",
			]
		);
	}

	#[test]
	fn cli_bin_argv_omits_open_wrapper() {
		let v = cli_bin_argv(&["-e"], "echo hi");
		assert_eq!(v, vec!["-e", "/bin/zsh", "-ilc", "echo hi; exec /bin/zsh -il"]);
	}

	#[test]
	fn registry_lookup_is_case_sensitive_by_name() {
		assert!(spec("Ghostty").is_some());
		assert!(spec("nope").is_none());
		assert_eq!(spec("Terminal").unwrap().app_bundle, "Terminal.app");
	}
}
