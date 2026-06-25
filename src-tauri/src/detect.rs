use crate::model::ItemKind;
use serde::Serialize;
use std::path::Path;

/// Suggested item config inferred from a folder's contents.
#[derive(Debug, Clone, Serialize)]
pub struct DetectResult {
	pub name: String,
	pub kind: ItemKind,
	#[serde(rename = "startCmd")] pub start_cmd: Option<String>,
	pub port: Option<u16>,
}

/// Inspect a folder and suggest name/kind/start command/port.
pub fn detect_folder(path: &Path) -> DetectResult {
	let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("service").to_string();
	let mut start_cmd = None;
	let pkg = path.join("package.json");
	if pkg.exists() {
		if let Ok(text) = std::fs::read_to_string(&pkg) {
			if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
				for script in ["dev", "start", "serve"] {
					if v.get("scripts").and_then(|s| s.get(script)).is_some() {
						start_cmd = Some(format!("npm run {script}"));
						break;
					}
				}
			}
		}
		if start_cmd.is_none() { start_cmd = Some("npm start".into()); }
	} else if path.join("pyproject.toml").exists() || path.join("requirements.txt").exists() {
		start_cmd = Some("python main.py".into());
	}
	let port = read_env_port(&path.join(".env"));
	DetectResult { name, kind: ItemKind::Project, start_cmd, port }
}

/// Parse a `PORT=NNNN` line from a .env file, if present.
fn read_env_port(env_path: &Path) -> Option<u16> {
	let text = std::fs::read_to_string(env_path).ok()?;
	for line in text.lines() {
		if let Some(rest) = line.trim().strip_prefix("PORT=") {
			if let Ok(p) = rest.trim().parse::<u16>() { return Some(p); }
		}
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tmp() -> std::path::PathBuf {
		let d = std::env::temp_dir().join(format!("msm-det-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&d).unwrap();
		d
	}

	#[test]
	fn detects_npm_dev_script_and_env_port() {
		let d = tmp();
		std::fs::write(d.join("package.json"), r#"{"scripts":{"dev":"vite"}}"#).unwrap();
		std::fs::write(d.join(".env"), "PORT=5173\n").unwrap();
		let r = detect_folder(&d);
		assert_eq!(r.kind, crate::model::ItemKind::Project);
		assert_eq!(r.start_cmd.as_deref(), Some("npm run dev"));
		assert_eq!(r.port, Some(5173));
		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn detects_python_project() {
		let d = tmp();
		std::fs::write(d.join("requirements.txt"), "flask\n").unwrap();
		let r = detect_folder(&d);
		assert_eq!(r.kind, crate::model::ItemKind::Project);
		assert!(r.start_cmd.as_deref().unwrap().starts_with("python"));
		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn name_falls_back_to_dir_basename() {
		let d = tmp();
		let r = detect_folder(&d);
		assert!(!r.name.is_empty());
		std::fs::remove_dir_all(&d).ok();
	}
}
