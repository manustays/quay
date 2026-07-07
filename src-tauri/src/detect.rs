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

/// Read and parse a folder's `package.json`, if present and valid.
fn read_package_json(dir: &Path) -> Option<serde_json::Value> {
	let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
	serde_json::from_str(&text).ok()
}

/// Inspect a folder and suggest name/kind/start command/port/stack.
pub fn detect_folder(path: &Path) -> DetectResult {
	// Parse package.json once and reuse it for the name, start command, and the
	// stack detection below (each helper would otherwise re-read/re-parse it).
	let pkg = read_package_json(path);
	let name = name_from_dir_with(path, pkg.as_ref());
	let mut start_cmd = None;
	if pkg.is_some() {
		if let Some(v) = &pkg {
			for script in ["dev", "start", "serve"] {
				if v.get("scripts").and_then(|s| s.get(script)).is_some() {
					start_cmd = Some(format!("npm run {script}"));
					break;
				}
			}
		}
		if start_cmd.is_none() { start_cmd = Some("npm start".into()); }
	} else if path.join("pyproject.toml").exists() || path.join("requirements.txt").exists() {
		start_cmd = Some("python main.py".into());
	}
	let port = read_env_port(&path.join(".env"));
	let stack = stack_from_dir_with(path, pkg.as_ref()).map(str::to_string);
	DetectResult { name, kind: ItemKind::Project, start_cmd, port, stack }
}

/// Project display name from a folder's manifest, else the dir basename.
/// `package.json` `"name"` wins; then `Cargo.toml` `[package] name`.
pub fn name_from_dir(dir: &Path) -> String {
	name_from_dir_with(dir, read_package_json(dir).as_ref())
}

/// [`name_from_dir`] with an already-parsed `package.json` (or `None`), so a
/// caller that also needs the manifest doesn't read and parse it twice.
fn name_from_dir_with(dir: &Path, pkg: Option<&serde_json::Value>) -> String {
	if let Some(n) = pkg.and_then(|v| v.get("name")).and_then(|n| n.as_str()) {
		let n = n.trim();
		if !n.is_empty() { return n.to_string(); }
	}
	if let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) {
		if let Some(n) = cargo_package_name(&text) { return n; }
	}
	dir.file_name().and_then(|s| s.to_str()).unwrap_or("service").to_string()
}

/// Extract `name = "..."` from a Cargo.toml's `[package]` table.
///
/// ponytail: hand-scan scoped to `[package]`; strips line comments and accepts a
/// quoted value only (so `name.workspace = true` and dependency-table `name`
/// keys are ignored). Swap for a real toml parse only if a crate name needs a
/// `#` or a multi-line string — it can't (crate names are `[A-Za-z0-9_-]`).
fn cargo_package_name(text: &str) -> Option<String> {
	let mut in_package = false;
	for raw in text.lines() {
		let line = raw.split('#').next().unwrap_or("").trim();
		if line.starts_with('[') {
			in_package = line == "[package]";
			continue;
		}
		if in_package {
			if let Some(rest) = line.strip_prefix("name").map(str::trim_start) {
				if let Some(val) = rest.strip_prefix('=').map(str::trim) {
					if val.starts_with('"') || val.starts_with('\'') {
						let name = val.trim_matches(|c| c == '"' || c == '\'');
						if !name.is_empty() { return Some(name.to_string()); }
					}
				}
			}
		}
	}
	None
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
	// (stack, argv basenames that identify it). Framework launchers first so a
	// specific tool wins over the generic runtime that spawned it (e.g. a
	// `streamlit`/`gunicorn` process also has `python` in argv → keep streamlit).
	const LAUNCHERS: &[(&str, &[&str])] = &[
		("next", &["next", "next-server"]),
		("vite", &["vite", "vite.js"]),
		("nuxt", &["nuxt", "nuxi"]),
		("astro", &["astro"]),
		("remix", &["remix", "remix-serve"]),
		("angular", &["ng"]),
		("gatsby", &["gatsby"]),
		("vue", &["vue-cli-service"]),
		("nest", &["nest"]),
		("expo", &["expo"]),
		("react", &["react-scripts"]),
		("webpack", &["webpack", "webpack-dev-server"]),
		("parcel", &["parcel"]),
		("django", &["manage.py"]),
		("flask", &["flask"]),
		("fastapi", &["uvicorn", "fastapi"]),
		("streamlit", &["streamlit"]),
		("gunicorn", &["gunicorn"]),
		("gradio", &["gradio"]),
		("jupyter", &["jupyter-lab", "jupyter-notebook", "jupyter"]),
		("rails", &["rails", "puma"]),
		("laravel", &["artisan"]),
		("rust", &["cargo"]),
		("go", &["go"]),
		("elixir", &["mix", "beam.smp"]),
		("node", &["node", "npm", "npx", "bun", "deno"]),
		("python", &["python", "python3"]),
		// Bare runtimes for JVM/.NET/PHP/Ruby. ponytail: matches any JVM/.NET
		// process, so infra like Elasticsearch/Kafka reads as "java" — acceptable
		// for a label; the dev-only radar filter (opt-in) will let those through.
		("java", &["java"]),
		("dotnet", &["dotnet"]),
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
	stack_from_dir_with(dir, read_package_json(dir).as_ref())
}

/// [`stack_from_dir`] with an already-parsed `package.json` (or `None`), so a
/// caller that also needs the manifest doesn't read and parse it twice.
fn stack_from_dir_with(dir: &Path, pkg: Option<&serde_json::Value>) -> Option<&'static str> {
	// Generic runtime fallbacks ("node", "python") are deferred to the end:
	// a Rails/Django/Go repo often carries a package.json for its JS tooling
	// (jsbundling, Tailwind) and must not be mislabeled by it.
	// Tauri wraps a web frontend into a native desktop app — it's the shipping
	// stack, so it wins over the Vite/React the frontend also carries. Covers
	// both a repo root (src-tauri/tauri.conf.json) and the src-tauri/ dir itself
	// (tauri.conf.json), the latter otherwise falling through to Cargo.toml->rust.
	// ponytail: standard layout only; add a @tauri-apps/* dep check if a repo
	// puts its conf elsewhere.
	if dir.join("src-tauri/tauri.conf.json").exists() || dir.join("tauri.conf.json").exists() {
		return Some("tauri");
	}
	let mut fallback: Option<&'static str> = None;
	if let Some(v) = pkg {
		let has_dep = |name: &str| {
			["dependencies", "devDependencies"]
				.iter()
				.any(|k| v.get(k).and_then(|d| d.get(name)).is_some())
		};
		// Framework deps before the bundler/library ones: a SvelteKit or Vue app
		// also carries `vite`, and CRA carries `react`, but the framework is the
		// truer label.
		for (stack, dep) in [
			("next", "next"),
			("nuxt", "nuxt"),
			("remix", "@remix-run/react"),
			("astro", "astro"),
			("angular", "@angular/core"),
			("svelte", "@sveltejs/kit"),
			("vue", "vue"),
			("gatsby", "gatsby"),
			("solid", "solid-js"),
			("nest", "@nestjs/core"),
			("expo", "expo"),
			("vite", "vite"),
			("react", "react"),
		] {
			if has_dep(dep) { return Some(stack); }
		}
		fallback = Some("node");
	}
	if dir.join("manage.py").exists() { return Some("django"); }
	for manifest in ["pyproject.toml", "requirements.txt"] {
		if let Ok(text) = std::fs::read_to_string(dir.join(manifest)) {
			let lower = text.to_lowercase();
			// ponytail: substring scan can match a comment/description mentioning
			// the tool; fine for a label, and the dev-only filter only keys off
			// "some stack was detected", not which one.
			for stack in ["django", "flask", "fastapi", "streamlit", "gradio", "gunicorn"] {
				if lower.contains(stack) { return Some(stack); }
			}
			fallback = fallback.or(Some("python"));
		}
	}
	if dir.join("Gemfile").exists() { return Some("rails"); }
	if dir.join("go.mod").exists() { return Some("go"); }
	if dir.join("Cargo.toml").exists() { return Some("rust"); }
	if dir.join("pom.xml").exists()
		|| dir.join("build.gradle").exists()
		|| dir.join("build.gradle.kts").exists()
	{
		return Some("java");
	}
	if let Ok(text) = std::fs::read_to_string(dir.join("mix.exs")) {
		return Some(if text.contains("phoenix") { "phoenix" } else { "elixir" });
	}
	// .NET: no fixed filename — scan the top level for a project/solution file.
	// ponytail: top-level only; a repo that nests its .csproj under a subfolder
	// won't detect from the root, but the live `dotnet` argv still labels it.
	if let Ok(entries) = std::fs::read_dir(dir) {
		if entries.filter_map(Result::ok).any(|e| {
			e.path()
				.extension()
				.and_then(|x| x.to_str())
				.map(|x| matches!(x, "csproj" | "sln" | "fsproj"))
				.unwrap_or(false)
		}) {
			return Some("dotnet");
		}
	}
	if let Ok(text) = std::fs::read_to_string(dir.join("composer.json")) {
		return Some(if text.contains("laravel/framework") { "laravel" } else { "php" });
	}
	fallback
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

	#[test]
	fn name_from_manifest_beats_dir_basename() {
		let d = tmp();
		// package.json "name" wins.
		std::fs::write(d.join("package.json"), r#"{"name":"acme-api"}"#).unwrap();
		assert_eq!(name_from_dir(&d), "acme-api");
		// Blank package name falls through to Cargo.toml [package] name.
		std::fs::write(d.join("package.json"), r#"{"name":"  "}"#).unwrap();
		std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"my-crate\"\n").unwrap();
		assert_eq!(name_from_dir(&d), "my-crate");
		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn cargo_name_only_from_package_table() {
		// A `name` under [dependencies] must not be picked up.
		let toml = "[package]\nname = \"real-crate\"\n\n[dependencies]\nname = \"0.1\"\n";
		assert_eq!(cargo_package_name(toml).as_deref(), Some("real-crate"));
		// name.workspace = true (no quoted value) is ignored.
		let ws = "[package]\nname.workspace = true\n";
		assert_eq!(cargo_package_name(ws), None);
		// A dep-table name before any [package] is ignored.
		let dep_first = "[dependencies]\nname = \"0.1\"\n";
		assert_eq!(cargo_package_name(dep_first), None);
	}

	#[test]
	fn name_falls_back_to_service_for_empty_path() {
		assert_eq!(name_from_dir(Path::new("/")), "service");
	}

	/// Build an owned argv from string literals.
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
		// Newly-covered launchers.
		assert_eq!(stack_from_argv(&argv(&["node", "/x/.bin/ng", "serve"])), Some("angular"));
		assert_eq!(stack_from_argv(&argv(&["/x/.bin/gatsby", "develop"])), Some("gatsby"));
		// streamlit/gunicorn win over the `python` also present in argv.
		assert_eq!(stack_from_argv(&argv(&["python", "/x/bin/streamlit", "run", "app.py"])), Some("streamlit"));
		assert_eq!(stack_from_argv(&argv(&["/x/bin/gunicorn", "app:app"])), Some("gunicorn"));
		// Bare runtimes for JVM / .NET / Elixir.
		assert_eq!(stack_from_argv(&argv(&["/usr/bin/java", "-jar", "app.jar"])), Some("java"));
		assert_eq!(stack_from_argv(&argv(&["dotnet", "run"])), Some("dotnet"));
		assert_eq!(stack_from_argv(&argv(&["/x/bin/mix", "phx.server"])), Some("elixir"));
		assert_eq!(stack_from_argv(&argv(&["/x/erts/bin/beam.smp", "-K", "true"])), Some("elixir"));
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
	fn stack_from_dir_prefers_tauri_over_frontend_and_rust() {
		let d = tmp();
		// A Tauri app: Vite frontend at root, tauri.conf.json under src-tauri/.
		std::fs::write(d.join("package.json"), r#"{"devDependencies":{"vite":"5"}}"#).unwrap();
		std::fs::create_dir_all(d.join("src-tauri")).unwrap();
		std::fs::write(d.join("src-tauri/tauri.conf.json"), "{}").unwrap();
		std::fs::write(d.join("src-tauri/Cargo.toml"), "[package]\n").unwrap();
		assert_eq!(stack_from_dir(&d), Some("tauri"));
		// Scanning the src-tauri/ dir itself (conf at its root, alongside Cargo.toml)
		// is tauri, not rust.
		assert_eq!(stack_from_dir(&d.join("src-tauri")), Some("tauri"));
		std::fs::remove_dir_all(&d).ok();
	}

	#[test]
	fn stack_from_dir_prefers_backend_manifest_over_tooling_package_json() {
		let d = tmp();
		// A Rails repo with a JS-tooling package.json (no framework deps) is rails.
		std::fs::write(d.join("package.json"), r#"{"devDependencies":{"esbuild":"0.20"}}"#).unwrap();
		std::fs::write(d.join("Gemfile"), "gem 'rails'\n").unwrap();
		assert_eq!(stack_from_dir(&d), Some("rails"));
		// Without a backend manifest the same package.json falls back to node.
		std::fs::remove_file(d.join("Gemfile")).unwrap();
		assert_eq!(stack_from_dir(&d), Some("node"));
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

	#[test]
	fn stack_from_dir_detects_new_frameworks_and_backends() {
		let d = tmp();
		// package.json framework deps.
		std::fs::write(d.join("package.json"), r#"{"dependencies":{"@angular/core":"18"}}"#).unwrap();
		assert_eq!(stack_from_dir(&d), Some("angular"));
		std::fs::write(d.join("package.json"), r#"{"devDependencies":{"@sveltejs/kit":"2","vite":"5"}}"#).unwrap();
		assert_eq!(stack_from_dir(&d), Some("svelte"));
		std::fs::remove_file(d.join("package.json")).unwrap();
		// JVM / Elixir / .NET manifests.
		std::fs::write(d.join("pom.xml"), "<project/>").unwrap();
		assert_eq!(stack_from_dir(&d), Some("java"));
		std::fs::remove_file(d.join("pom.xml")).unwrap();
		std::fs::write(d.join("mix.exs"), "defp deps do [{:phoenix, \"~> 1.7\"}] end").unwrap();
		assert_eq!(stack_from_dir(&d), Some("phoenix"));
		std::fs::remove_file(d.join("mix.exs")).unwrap();
		std::fs::write(d.join("Api.csproj"), "<Project/>").unwrap();
		assert_eq!(stack_from_dir(&d), Some("dotnet"));
		std::fs::remove_dir_all(&d).ok();
	}
}
