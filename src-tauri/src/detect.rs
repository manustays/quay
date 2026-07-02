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
	/// Detected tech stack keyword (e.g. "vite", "django") for the row icon.
	pub stack: Option<String>,
}

/// Inspect a folder and suggest name/kind/start command/port/stack.
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
	let stack = stack_from_dir(path).map(str::to_string);
	DetectResult { name, kind: ItemKind::Project, start_cmd, port, stack }
}

/// Strip any path prefix from an argv element: `/a/b/node` → `node`.
fn basename(arg: &str) -> &str {
	arg.rsplit('/').next().unwrap_or(arg)
}

/// Identify a tech stack from a live process's argv. Pure.
///
/// Matches whole argv elements (basenames), not joined-string substrings, so
/// e.g. a project path containing "vite" can't false-match. Framework launchers
/// are checked before generic runtimes (a `node …/.bin/next dev` is "next").
pub fn stack_from_argv(argv: &[String]) -> Option<&'static str> {
	// (stack, argv basenames that identify it)
	const LAUNCHERS: &[(&str, &[&str])] = &[
		("next", &["next", "next-server"]),
		("vite", &["vite", "vite.js"]),
		("nuxt", &["nuxt", "nuxi"]),
		("astro", &["astro"]),
		("remix", &["remix", "remix-serve"]),
		("django", &["manage.py"]),
		("flask", &["flask"]),
		("fastapi", &["uvicorn", "fastapi"]),
		("rails", &["rails", "puma"]),
		("laravel", &["artisan"]),
		("rust", &["cargo"]),
		("go", &["go"]),
		("node", &["node", "npm", "npx", "bun", "deno"]),
		("python", &["python", "python3"]),
		("php", &["php"]),
		("ruby", &["ruby"]),
	];
	let bases: Vec<&str> = argv.iter().map(|a| basename(a)).collect();
	for (stack, names) in LAUNCHERS {
		if bases.iter().any(|b| names.contains(b)) {
			return Some(stack);
		}
	}
	// `go run` compiles to a temp binary under .../go-build/...; `cargo run`
	// executes from target/debug|release — the argv[0] path is the only tell.
	let exe = argv.first().map(String::as_str).unwrap_or("");
	if exe.contains("/go-build") { return Some("go"); }
	if exe.contains("/target/debug/") || exe.contains("/target/release/") { return Some("rust"); }
	None
}

/// Identify a tech stack from a project folder's manifests. Pure I/O reads.
///
/// Framework-specific markers win over generic runtimes (a Vite React app is
/// "vite", not "react"; a Django repo is "django", not "python").
pub fn stack_from_dir(dir: &Path) -> Option<&'static str> {
	if let Ok(text) = std::fs::read_to_string(dir.join("package.json")) {
		if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
			let has_dep = |name: &str| {
				["dependencies", "devDependencies"]
					.iter()
					.any(|k| v.get(k).and_then(|d| d.get(name)).is_some())
			};
			for (stack, dep) in [
				("next", "next"),
				("nuxt", "nuxt"),
				("remix", "@remix-run/react"),
				("astro", "astro"),
				("vite", "vite"),
				("react", "react"),
			] {
				if has_dep(dep) { return Some(stack); }
			}
			return Some("node");
		}
	}
	if dir.join("manage.py").exists() { return Some("django"); }
	for manifest in ["pyproject.toml", "requirements.txt"] {
		if let Ok(text) = std::fs::read_to_string(dir.join(manifest)) {
			let lower = text.to_lowercase();
			for stack in ["django", "flask", "fastapi"] {
				if lower.contains(stack) { return Some(stack); }
			}
			return Some("python");
		}
	}
	if dir.join("Gemfile").exists() { return Some("rails"); }
	if dir.join("go.mod").exists() { return Some("go"); }
	if dir.join("Cargo.toml").exists() { return Some("rust"); }
	if let Ok(text) = std::fs::read_to_string(dir.join("composer.json")) {
		return Some(if text.contains("laravel/framework") { "laravel" } else { "php" });
	}
	None
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

	fn argv(parts: &[&str]) -> Vec<String> {
		parts.iter().map(|s| s.to_string()).collect()
	}

	#[test]
	fn stack_from_argv_matches_framework_launchers() {
		assert_eq!(stack_from_argv(&argv(&["node", "/x/node_modules/.bin/next", "dev"])), Some("next"));
		assert_eq!(stack_from_argv(&argv(&["node", "/x/node_modules/vite/bin/vite.js"])), Some("vite"));
		assert_eq!(stack_from_argv(&argv(&["python", "manage.py", "runserver"])), Some("django"));
		assert_eq!(stack_from_argv(&argv(&["/usr/bin/php", "artisan", "serve"])), Some("laravel"));
		assert_eq!(stack_from_argv(&argv(&["uvicorn", "app:app"])), Some("fastapi"));
	}

	#[test]
	fn stack_from_argv_falls_back_to_runtime_and_exe_path() {
		assert_eq!(stack_from_argv(&argv(&["node", "server.js"])), Some("node"));
		assert_eq!(stack_from_argv(&argv(&["python3", "app.py"])), Some("python"));
		assert_eq!(stack_from_argv(&argv(&["/x/target/debug/myapp"])), Some("rust"));
		assert_eq!(stack_from_argv(&argv(&["/tmp/go-build123/b001/exe/main"])), Some("go"));
		// A project *path* containing a framework name must not match.
		assert_eq!(stack_from_argv(&argv(&["/Users/me/vite-clone/serve"])), None);
		assert_eq!(stack_from_argv(&[]), None);
	}

	#[test]
	fn stack_from_dir_prefers_framework_over_runtime() {
		let d = tmp();
		std::fs::write(
			d.join("package.json"),
			r#"{"dependencies":{"react":"18"},"devDependencies":{"vite":"5"}}"#,
		).unwrap();
		assert_eq!(stack_from_dir(&d), Some("vite"));
		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn stack_from_dir_detects_python_and_rust_manifests() {
		let d = tmp();
		std::fs::write(d.join("requirements.txt"), "Django==5.0\n").unwrap();
		assert_eq!(stack_from_dir(&d), Some("django"));
		std::fs::remove_file(d.join("requirements.txt")).unwrap();
		std::fs::write(d.join("Cargo.toml"), "[package]\n").unwrap();
		assert_eq!(stack_from_dir(&d), Some("rust"));
		std::fs::remove_dir_all(&d).ok();
	}
}
