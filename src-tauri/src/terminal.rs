use crate::model::AppError;
use std::collections::BTreeMap;
use std::process::Command;

/// Build the shell line run inside the terminal: cd + env exports + command.
///
/// Single-quote-escapes `dir` and each env value so that paths and values
/// containing single quotes are handled correctly (using `'\''` escaping).
pub fn build_command_line(dir: &str, env: &BTreeMap<String, String>, cmd: &str) -> String {
	let mut parts = vec![format!("cd '{}'", dir.replace('\'', "'\\''"))];
	for (k, v) in env {
		parts.push(format!("export {}='{}'", k, v.replace('\'', "'\\''")));
	}
	parts.push(cmd.to_string());
	parts.join(" && ")
}

/// Open a terminal window already cd'd into `dir` and run `clear`.
///
/// `app_name` is `"Terminal"` or `"iTerm"` (matching `Settings::terminal_app`).
pub fn open_folder(app_name: &str, dir: &str) -> Result<(), AppError> {
	run_in_terminal(app_name, dir, &BTreeMap::new(), "clear")
}

/// Open a terminal window and run `cmd` in `dir` with the given env exports.
///
/// Uses `osascript -e` to drive either Terminal.app (`do script`) or
/// iTerm (`write text`). Returns `AppError` containing stderr on failure.
pub fn run_in_terminal(
	app_name: &str,
	dir: &str,
	env: &BTreeMap<String, String>,
	cmd: &str,
) -> Result<(), AppError> {
	let line = build_command_line(dir, env, cmd);
	let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
	let script = match app_name {
		"iTerm" => format!(
			"tell application \"iTerm\"\n create window with default profile\n tell current session of current window to write text \"{escaped}\"\nend tell"
		),
		_ => format!(
			"tell application \"Terminal\"\n activate\n do script \"{escaped}\"\nend tell"
		),
	};
	let out = Command::new("osascript")
		.arg("-e")
		.arg(&script)
		.output()
		.map_err(|e| AppError::Message(format!("osascript failed: {e}")))?;
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
	use std::collections::BTreeMap;

	#[test]
	fn builds_command_with_env_exports() {
		let mut env = BTreeMap::new();
		env.insert("FOO".to_string(), "bar".to_string());
		let line = build_command_line("/tmp/app", &env, "npm run dev");
		assert!(line.starts_with("cd '/tmp/app'"));
		assert!(line.contains("export FOO='bar'"));
		assert!(line.ends_with("npm run dev"));
	}
}
