use crate::model::{AppError, Status};
use std::collections::HashMap;
use std::process::Command;

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

/// Current status of a brew formula (Stopped if brew missing or formula absent).
pub fn brew_status(formula: &str) -> Status {
	let Ok(out) = Command::new("brew").args(["services", "list"]).output() else {
		return Status::Stopped;
	};
	let text = String::from_utf8_lossy(&out.stdout);
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
	let out = Command::new("brew")
		.args(["services", action, formula])
		.output()
		.map_err(|e| AppError::Message(format!("brew not found: {e}")))?;
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
	fn parses_brew_services_list() {
		let out = "Name    Status  User  File\nmysql   started abhi  ~/x\nmongodb stopped -     -\nredis   error   abhi  ~/y\n";
		let m = parse_brew_list(out);
		assert_eq!(m.get("mysql"), Some(&Status::Running));
		assert_eq!(m.get("mongodb"), Some(&Status::Stopped));
		assert_eq!(m.get("redis"), Some(&Status::Error));
	}
}
