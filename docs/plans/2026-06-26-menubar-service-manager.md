# Menubar Service Manager Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A native macOS menubar app (Tauri) that registers, starts/stops, and monitors long-running local services — project servers, Homebrew services, and terminal agents — with one-click browser and terminal access.

**Architecture:** Tauri v2 app. A Rust core owns all process supervision and state; a vanilla-TypeScript webview popover (anchored under the tray icon) is the UI. They talk via Tauri commands (frontend → Rust) and events (Rust → frontend status/log push). Services run as owned child processes (background mode, logs to file) or in a Terminal/iTerm window (terminal mode); both die conceptually with the app session.

**Tech Stack:** Rust + Tauri v2, vanilla TypeScript + Vite frontend, `vitest` (frontend tests), `cargo test` (Rust tests).

## Global Constraints

- **Platform:** macOS only. Uses `osascript`, `open`, `brew`, `zsh -lc`.
- **Tauri:** v2.x (`tauri = "2"`, `@tauri-apps/api` v2).
- **Rust crates (run `cargo add` — get user approval before adding, per project policy):** `serde` (features `derive`), `serde_json`, `uuid` (features `v4`), `dirs`, `libc`, `ureq` (HTTP health check), `tauri-plugin-positioner` (features `tray-icon`).
- **Frontend deps:** `@tauri-apps/api`, `@tauri-apps/plugin-positioner`, `typescript`, `vite`, `vitest`.
- **Config path:** `dirs::data_dir()` → `~/Library/Application Support/com.abhi.menubar-service-manager/config.json`. Logs: same dir, `logs/<id>.log`.
- **Indentation:** tabs (per user preference) in both Rust and TS.
- **Types/interfaces:** always declared — Rust structs `#[derive(Serialize, Deserialize)]`; TS `interface` for every shape crossing the IPC boundary.
- **Docstrings:** every public Rust fn/struct gets a `///` doc comment; every exported TS function a JSDoc block.
- **Commits:** conventional commits, no `Co-Authored-By` trailer.
- **Status states (canonical strings, used verbatim everywhere):** `"stopped" | "starting" | "running" | "error"`.
- **Item kinds:** `"project" | "brew" | "agent"`. **Run modes:** `"background" | "terminal"`.

---

## File Structure

**Rust (`src-tauri/src/`):**
- `lib.rs` — Tauri builder, plugin/command registration, tray + popover setup, app-state, quit handler.
- `model.rs` — `ManagedItem`, `Settings`, `AppConfig`, `ItemKind`, `RunMode`, `Status`, `AppError`, `ItemStatus`.
- `store.rs` — load/save `AppConfig` (atomic write), corrupt-file backup, config path resolution.
- `detect.rs` — inspect a folder → `DetectResult` (suggested name/cmd/port/kind).
- `brew.rs` — `brew services list/start/stop` wrappers + list-output parser.
- `supervisor.rs` — background spawn/stop, process-group kill, log files, `RuntimeState`.
- `health.rs` — pure status-decision fn + the poll loop that emits `status_changed`.
- `terminal.rs` — open folder / run terminal-mode item via `osascript`.
- `commands.rs` — all `#[tauri::command]` handlers.
- `state.rs` — `AppState` (shared config + runtime maps behind a `Mutex`).

**Frontend (`src/`):**
- `ipc.ts` — typed wrappers over `invoke`/`listen`; shared TS interfaces.
- `model.ts` — TS mirror of item/status types + helpers (`statusDot`, `matchesSearch`, `splitFavorites`).
- `list.ts` — render the two-tier list, search, Stop-all.
- `row.ts` — render one item row + expand panel + action wiring.
- `form.ts` — Add/Edit form, detect prefill.
- `settings.ts` — settings panel.
- `main.ts` — bootstrap, event subscriptions, popover lifecycle.
- `styles.css` — popover styling.

**Tests:**
- Rust: inline `#[cfg(test)] mod tests` per module (`model`, `store`, `detect`, `brew`, `health`, `supervisor`).
- Frontend: `src/*.test.ts` (vitest) for `model.ts` pure helpers.

---

## Task 1: Scaffold Tauri app + tray icon + empty popover

**Files:**
- Create: whole `src-tauri/` + `src/` via scaffolder, then edit `src-tauri/src/lib.rs`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src/main.ts`, `src/styles.css`, `index.html`.

**Interfaces:**
- Consumes: nothing.
- Produces: a running app with a tray icon that toggles a frameless, always-on-top popover window named `"main"`, hidden on blur.

- [ ] **Step 1: Scaffold the app**

Run (non-interactive; vanilla-TS template):
```bash
npm create tauri-app@latest -- menubar-service-manager --template vanilla-ts --manager npm --yes
```
If the tool refuses to write into the existing repo root, scaffold into a temp dir and move `src/`, `src-tauri/`, `index.html`, `package.json`, `vite.config.ts`, `tsconfig.json` into the repo root. Expected: `src-tauri/Cargo.toml` and `src/main.ts` exist.

- [ ] **Step 2: Add the positioner plugin deps**

```bash
cd src-tauri && cargo add tauri-plugin-positioner --features tray-icon && cd ..
npm install @tauri-apps/plugin-positioner
```
Expected: both resolve without error.

- [ ] **Step 3: Configure the popover window in `tauri.conf.json`**

In `src-tauri/tauri.conf.json`, set the `app.windows` array to a single window:
```json
{
	"label": "main",
	"width": 380,
	"height": 520,
	"decorations": false,
	"resizable": false,
	"transparent": false,
	"alwaysOnTop": true,
	"visible": false,
	"skipTaskbar": true
}
```
And set `app.macOSPrivateApi` to `true`. Set `productName` to `Menubar Service Manager` and `identifier` to `com.abhi.menubar-service-manager`.

- [ ] **Step 4: Write tray + popover wiring in `lib.rs`**

Replace `src-tauri/src/lib.rs` body with:
```rust
use tauri::{
	Manager,
	tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
	WindowEvent,
};

/// Toggle the popover window: show+focus if hidden, hide if visible.
fn toggle_popover(app: &tauri::AppHandle) {
	if let Some(win) = app.get_webview_window("main") {
		if win.is_visible().unwrap_or(false) {
			let _ = win.hide();
		} else {
			let _ = tauri_plugin_positioner::WindowExt::move_window(
				&win,
				tauri_plugin_positioner::Position::TrayCenter,
			);
			let _ = win.show();
			let _ = win.set_focus();
		}
	}
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
	tauri::Builder::default()
		.plugin(tauri_plugin_positioner::init())
		.setup(|app| {
			TrayIconBuilder::new()
				.icon(app.default_window_icon().unwrap().clone())
				.on_tray_icon_event(|tray, event| {
					tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
					if let TrayIconEvent::Click {
						button: MouseButton::Left,
						button_state: MouseButtonState::Up,
						..
					} = event
					{
						toggle_popover(tray.app_handle());
					}
				})
				.build(app)?;
			Ok(())
		})
		.on_window_event(|window, event| {
			// Hide the popover when it loses focus (menubar-app behavior).
			if let WindowEvent::Focused(false) = event {
				let _ = window.hide();
			}
		})
		.run(tauri::generate_context!())
		.expect("error while running tauri application");
}
```

- [ ] **Step 5: Minimal popover body**

Replace `src/main.ts` with a placeholder render:
```ts
document.querySelector('#app')!.innerHTML = `<div class="popover"><h1>Services</h1><p>No items yet.</p></div>`;
```
Add to `src/styles.css`:
```css
body { margin: 0; font: 13px -apple-system, system-ui, sans-serif; }
.popover { padding: 10px; }
```

- [ ] **Step 6: Run and verify**

Run: `npm run tauri dev`
Expected: app launches with no dock-less crash; a tray icon appears; clicking it shows a 380×520 popover under the icon reading "Services / No items yet."; clicking elsewhere hides it.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: scaffold tauri app with tray icon and popover window"
```

---

## Task 2: Core data model (`model.rs`)

**Files:**
- Create: `src-tauri/src/model.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod model;`)

**Interfaces:**
- Produces:
  - `enum ItemKind { Project, Brew, Agent }`, `enum RunMode { Background, Terminal }`, `enum Status { Stopped, Starting, Running, Error }` — all serde as the canonical lowercase strings.
  - `struct ManagedItem { id: String, name: String, kind: ItemKind, dir: Option<String>, start_cmd: Option<String>, stop_cmd: Option<String>, port: Option<u16>, run_mode: RunMode, brew_formula: Option<String>, order: u32, favorite: bool, env: std::collections::BTreeMap<String,String>, health_path: Option<String>, auto_start: bool }`
  - `struct Settings { terminal_app: String, poll_interval_sec: u64, browser: String, launch_at_login: bool }` with `Default` (terminal_app `"Terminal"`, poll 3, browser `"default"`, launch_at_login false).
  - `struct AppConfig { settings: Settings, items: Vec<ManagedItem> }` with `Default`.
  - `struct ItemStatus { id: String, status: Status, last_error: Option<String> }` (event payload).
  - `enum AppError { ... }` implementing `Display`, `serde::Serialize` (serialize as its string message), and `From<std::io::Error>`.

- [ ] **Step 1: Write failing tests**

Create `src-tauri/src/model.rs` ending with:
```rust
#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn status_serializes_to_canonical_strings() {
		assert_eq!(serde_json::to_string(&Status::Running).unwrap(), "\"running\"");
		assert_eq!(serde_json::to_string(&ItemKind::Brew).unwrap(), "\"brew\"");
		assert_eq!(serde_json::to_string(&RunMode::Terminal).unwrap(), "\"terminal\"");
	}

	#[test]
	fn settings_defaults_match_spec() {
		let s = Settings::default();
		assert_eq!(s.terminal_app, "Terminal");
		assert_eq!(s.poll_interval_sec, 3);
		assert_eq!(s.browser, "default");
		assert!(!s.launch_at_login);
	}

	#[test]
	fn app_error_serializes_as_message_string() {
		let e = AppError::Message("boom".into());
		assert_eq!(serde_json::to_string(&e).unwrap(), "\"boom\"");
	}
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cd src-tauri && cargo test model:: 2>&1 | tail -20`
Expected: compile error (types not defined).

- [ ] **Step 3: Implement the model**

Put above the test module in `model.rs`:
```rust
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What kind of managed item this is.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind { Project, Brew, Agent }

/// How an item is launched.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RunMode { Background, Terminal }

/// Live status of an item.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Status { Stopped, Starting, Running, Error }

/// A registered service the app manages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedItem {
	pub id: String,
	pub name: String,
	pub kind: ItemKind,
	pub dir: Option<String>,
	#[serde(rename = "startCmd")] pub start_cmd: Option<String>,
	#[serde(rename = "stopCmd")] pub stop_cmd: Option<String>,
	pub port: Option<u16>,
	#[serde(rename = "runMode")] pub run_mode: RunMode,
	#[serde(rename = "brewFormula")] pub brew_formula: Option<String>,
	pub order: u32,
	pub favorite: bool,
	#[serde(default)] pub env: BTreeMap<String, String>,
	#[serde(rename = "healthPath")] pub health_path: Option<String>,
	#[serde(rename = "autoStart")] pub auto_start: bool,
}

/// App-wide settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
	#[serde(rename = "terminalApp")] pub terminal_app: String,
	#[serde(rename = "pollIntervalSec")] pub poll_interval_sec: u64,
	pub browser: String,
	#[serde(rename = "launchAtLogin")] pub launch_at_login: bool,
}
impl Default for Settings {
	fn default() -> Self {
		Self { terminal_app: "Terminal".into(), poll_interval_sec: 3, browser: "default".into(), launch_at_login: false }
	}
}

/// The persisted configuration file shape.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
	#[serde(default)] pub settings: SettingsOrDefault,
	#[serde(default)] pub items: Vec<ManagedItem>,
}

/// Wrapper so a missing `settings` key still yields defaults.
pub type SettingsOrDefault = Settings;
impl Default for SettingsOrDefault { fn default() -> Self { Settings::default() } }

/// Status event payload pushed to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct ItemStatus {
	pub id: String,
	pub status: Status,
	#[serde(rename = "lastError")] pub last_error: Option<String>,
}

/// All recoverable errors surfaced to the frontend.
#[derive(Debug)]
pub enum AppError { Message(String) }
impl std::fmt::Display for AppError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self { AppError::Message(m) => write!(f, "{m}") }
	}
}
impl std::error::Error for AppError {}
impl Serialize for AppError {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_str(&self.to_string())
	}
}
impl From<std::io::Error> for AppError {
	fn from(e: std::io::Error) -> Self { AppError::Message(e.to_string()) }
}
```
Note: the `Default` derive on `AppConfig` requires `Settings: Default` (satisfied) — drop the `SettingsOrDefault` alias if the derive complains and use `#[serde(default)] pub settings: Settings` directly.

Add `mod model;` near the top of `lib.rs`.

- [ ] **Step 4: Run, verify pass**

Run: `cd src-tauri && cargo test model:: 2>&1 | tail -20`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add core data model types"
```

---

## Task 3: Config persistence (`store.rs`)

**Files:**
- Create: `src-tauri/src/store.rs`
- Modify: `lib.rs` (`mod store;`)

**Interfaces:**
- Consumes: `model::{AppConfig, AppError}`.
- Produces:
  - `fn config_dir() -> Result<PathBuf, AppError>` — ensures the app data dir exists, returns it.
  - `fn load_config(dir: &Path) -> AppConfig` — reads `config.json`; on missing returns default; on corrupt renames to `config.bad.json` and returns default.
  - `fn save_config(dir: &Path, cfg: &AppConfig) -> Result<(), AppError>` — atomic temp-write + rename.

- [ ] **Step 1: Write failing tests**

Create `src-tauri/src/store.rs` ending with:
```rust
#[cfg(test)]
mod tests {
	use super::*;
	use crate::model::AppConfig;

	#[test]
	fn save_then_load_roundtrips() {
		let dir = std::env::temp_dir().join(format!("msm-test-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&dir).unwrap();
		let mut cfg = AppConfig::default();
		cfg.settings.poll_interval_sec = 9;
		save_config(&dir, &cfg).unwrap();
		let loaded = load_config(&dir);
		assert_eq!(loaded.settings.poll_interval_sec, 9);
		std::fs::remove_dir_all(&dir).ok();
	}

	#[test]
	fn missing_config_yields_default() {
		let dir = std::env::temp_dir().join(format!("msm-test-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&dir).unwrap();
		let loaded = load_config(&dir);
		assert_eq!(loaded.settings.poll_interval_sec, 3);
		std::fs::remove_dir_all(&dir).ok();
	}

	#[test]
	fn corrupt_config_is_backed_up_and_defaults_returned() {
		let dir = std::env::temp_dir().join(format!("msm-test-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("config.json"), b"{not valid json").unwrap();
		let loaded = load_config(&dir);
		assert_eq!(loaded.settings.poll_interval_sec, 3);
		assert!(dir.join("config.bad.json").exists());
		std::fs::remove_dir_all(&dir).ok();
	}
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cd src-tauri && cargo test store:: 2>&1 | tail -20`
Expected: compile error (functions undefined).

- [ ] **Step 3: Implement**

Above the tests:
```rust
use crate::model::{AppConfig, AppError};
use std::path::{Path, PathBuf};

/// Ensure and return the app's data directory.
pub fn config_dir() -> Result<PathBuf, AppError> {
	let base = dirs::data_dir().ok_or_else(|| AppError::Message("no data dir".into()))?;
	let dir = base.join("com.abhi.menubar-service-manager");
	std::fs::create_dir_all(dir.join("logs"))?;
	Ok(dir)
}

/// Load config.json; default if missing, backup+default if corrupt.
pub fn load_config(dir: &Path) -> AppConfig {
	let path = dir.join("config.json");
	let Ok(text) = std::fs::read_to_string(&path) else { return AppConfig::default(); };
	match serde_json::from_str::<AppConfig>(&text) {
		Ok(cfg) => cfg,
		Err(_) => {
			let _ = std::fs::rename(&path, dir.join("config.bad.json"));
			AppConfig::default()
		}
	}
}

/// Atomically persist config.json (temp file + rename).
pub fn save_config(dir: &Path, cfg: &AppConfig) -> Result<(), AppError> {
	let tmp = dir.join("config.json.tmp");
	let text = serde_json::to_string_pretty(cfg).map_err(|e| AppError::Message(e.to_string()))?;
	std::fs::write(&tmp, text)?;
	std::fs::rename(&tmp, dir.join("config.json"))?;
	Ok(())
}
```
Add `mod store;` to `lib.rs`. Ensure `uuid` is a dev-usable dep (it's in Cargo via `cargo add uuid --features v4`).

- [ ] **Step 4: Run, verify pass**

Run: `cd src-tauri && cargo test store:: 2>&1 | tail -20`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add config persistence with atomic write and corrupt-file recovery"
```

---

## Task 4: Folder auto-detection (`detect.rs`)

**Files:**
- Create: `src-tauri/src/detect.rs`
- Modify: `lib.rs` (`mod detect;`)

**Interfaces:**
- Consumes: `model::ItemKind`.
- Produces:
  - `struct DetectResult { name: String, kind: ItemKind, start_cmd: Option<String>, port: Option<u16> }` (serde camelCase: `startCmd`).
  - `fn detect_folder(path: &Path) -> DetectResult` — pure-ish (reads files under `path`): name = dir basename; if `package.json` with a `dev`/`start` script → `npm run <that>`, kind Project; else if `pyproject.toml`/`requirements.txt` → `python main.py` guess, kind Project; port from a `.env` `PORT=` line if present.

- [ ] **Step 1: Write failing tests**

End `detect.rs` with:
```rust
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
```

- [ ] **Step 2: Run, verify fail**

Run: `cd src-tauri && cargo test detect:: 2>&1 | tail -20`
Expected: compile error.

- [ ] **Step 3: Implement**

Above the tests:
```rust
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
```
Add `mod detect;` to `lib.rs`.

- [ ] **Step 4: Run, verify pass**

Run: `cd src-tauri && cargo test detect:: 2>&1 | tail -20`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add folder auto-detection for add flow"
```

---

## Task 5: Brew service wrapper + parser (`brew.rs`)

**Files:**
- Create: `src-tauri/src/brew.rs`
- Modify: `lib.rs` (`mod brew;`)

**Interfaces:**
- Consumes: `model::{Status, AppError}`.
- Produces:
  - `fn parse_brew_list(output: &str) -> std::collections::HashMap<String, Status>` — pure; maps formula → Status from `brew services list` text (`started`→Running, `stopped`/`none`→Stopped, `error`→Error).
  - `fn brew_status(formula: &str) -> Status` — runs `brew services list`, parses, looks up (Stopped if absent or brew missing).
  - `fn brew_start(formula: &str) -> Result<(), AppError>` / `fn brew_stop(formula: &str) -> Result<(), AppError>` — run `brew services start|stop <formula>`, error carries stderr.

- [ ] **Step 1: Write failing test (pure parser only)**

End `brew.rs` with:
```rust
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
```

- [ ] **Step 2: Run, verify fail**

Run: `cd src-tauri && cargo test brew:: 2>&1 | tail -20`
Expected: compile error.

- [ ] **Step 3: Implement**

Above the tests:
```rust
use crate::model::{AppError, Status};
use std::collections::HashMap;
use std::process::Command;

/// Parse `brew services list` output into formula → Status.
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

/// Current status of a brew formula (Stopped if brew missing/absent).
pub fn brew_status(formula: &str) -> Status {
	let Ok(out) = Command::new("brew").args(["services", "list"]).output() else { return Status::Stopped; };
	let text = String::from_utf8_lossy(&out.stdout);
	parse_brew_list(&text).get(formula).copied().unwrap_or(Status::Stopped)
}

/// Start a brew formula's background service.
pub fn brew_start(formula: &str) -> Result<(), AppError> { run_brew("start", formula) }
/// Stop a brew formula's background service.
pub fn brew_stop(formula: &str) -> Result<(), AppError> { run_brew("stop", formula) }

fn run_brew(action: &str, formula: &str) -> Result<(), AppError> {
	let out = Command::new("brew").args(["services", action, formula]).output()
		.map_err(|e| AppError::Message(format!("brew not found: {e}")))?;
	if out.status.success() { Ok(()) }
	else { Err(AppError::Message(String::from_utf8_lossy(&out.stderr).trim().to_string())) }
}
```
Add `mod brew;` to `lib.rs`.

- [ ] **Step 4: Run, verify pass**

Run: `cd src-tauri && cargo test brew:: 2>&1 | tail -20`
Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add brew services wrapper and list parser"
```

---

## Task 6: Background process supervisor (`supervisor.rs`)

**Files:**
- Create: `src-tauri/src/supervisor.rs`
- Modify: `lib.rs` (`mod supervisor;`)

**Interfaces:**
- Consumes: `model::{ManagedItem, AppError}`, `store::config_dir`.
- Produces:
  - `struct Running { pub pid: u32, child: Child, pub log_path: PathBuf }`
  - `fn spawn_background(item: &ManagedItem, logs_dir: &Path) -> Result<Running, AppError>` — runs `zsh -lc "<startCmd>"` in its own process group (setsid via `pre_exec`), cwd=`dir`, env merged, stdout+stderr appended to `logs/<id>.log`.
  - `fn stop(running: &mut Running) -> Result<(), AppError>` — SIGTERM the process group, wait up to 5s, SIGKILL group if still alive.
  - `fn is_alive(running: &mut Running) -> bool` — `try_wait()` says still running.

- [ ] **Step 1: Write failing test**

End `supervisor.rs` with:
```rust
#[cfg(test)]
mod tests {
	use super::*;
	use crate::model::{ItemKind, ManagedItem, RunMode};
	use std::collections::BTreeMap;

	fn item(cmd: &str, dir: &str) -> ManagedItem {
		ManagedItem {
			id: "test-id".into(), name: "t".into(), kind: ItemKind::Project,
			dir: Some(dir.into()), start_cmd: Some(cmd.into()), stop_cmd: None,
			port: None, run_mode: RunMode::Background, brew_formula: None, order: 0,
			favorite: false, env: BTreeMap::new(), health_path: None, auto_start: false,
		}
	}

	#[test]
	fn spawn_then_stop_kills_process() {
		let logs = std::env::temp_dir().join(format!("msm-sup-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&logs).unwrap();
		let it = item("sleep 30", "/tmp");
		let mut r = spawn_background(&it, &logs).unwrap();
		assert!(is_alive(&mut r));
		stop(&mut r).unwrap();
		assert!(!is_alive(&mut r));
		std::fs::remove_dir_all(&logs).ok();
	}

	#[test]
	fn writes_log_file() {
		let logs = std::env::temp_dir().join(format!("msm-sup-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&logs).unwrap();
		let it = item("echo hello-marker", "/tmp");
		let mut r = spawn_background(&it, &logs).unwrap();
		std::thread::sleep(std::time::Duration::from_millis(400));
		let _ = stop(&mut r);
		let log = std::fs::read_to_string(logs.join("test-id.log")).unwrap();
		assert!(log.contains("hello-marker"));
		std::fs::remove_dir_all(&logs).ok();
	}
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cd src-tauri && cargo test supervisor:: 2>&1 | tail -20`
Expected: compile error.

- [ ] **Step 3: Implement**

Above the tests:
```rust
use crate::model::{AppError, ManagedItem};
use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// A spawned background child + its log file path.
pub struct Running {
	pub pid: u32,
	child: Child,
	pub log_path: PathBuf,
}

/// Spawn a background item via login shell in its own process group, logging to file.
pub fn spawn_background(item: &ManagedItem, logs_dir: &Path) -> Result<Running, AppError> {
	let cmd_str = item.start_cmd.clone().ok_or_else(|| AppError::Message("no start command".into()))?;
	let dir = item.dir.clone().ok_or_else(|| AppError::Message("no directory".into()))?;
	let log_path = logs_dir.join(format!("{}.log", item.id));
	let log = OpenOptions::new().create(true).append(true).open(&log_path)?;
	let log_err = log.try_clone()?;

	let mut cmd = Command::new("zsh");
	cmd.arg("-lc").arg(&cmd_str).current_dir(&dir)
		.stdout(Stdio::from(log)).stderr(Stdio::from(log_err)).stdin(Stdio::null());
	for (k, v) in &item.env { cmd.env(k, v); }
	// New session/process group so we can signal the whole tree.
	unsafe { cmd.pre_exec(|| { libc::setsid(); Ok(()) }); }

	let child = cmd.spawn().map_err(|e| AppError::Message(format!("spawn failed: {e}")))?;
	let pid = child.id();
	Ok(Running { pid, child, log_path })
}

/// SIGTERM the process group, escalate to SIGKILL after 5s.
pub fn stop(running: &mut Running) -> Result<(), AppError> {
	let pgid = running.pid as i32;
	unsafe { libc::kill(-pgid, libc::SIGTERM); }
	for _ in 0..50 {
		if !is_alive(running) { return Ok(()); }
		std::thread::sleep(std::time::Duration::from_millis(100));
	}
	unsafe { libc::kill(-pgid, libc::SIGKILL); }
	let _ = running.child.wait();
	Ok(())
}

/// True if the child has not yet exited.
pub fn is_alive(running: &mut Running) -> bool {
	matches!(running.child.try_wait(), Ok(None))
}
```
Add `mod supervisor;` to `lib.rs`.

- [ ] **Step 4: Run, verify pass**

Run: `cd src-tauri && cargo test supervisor:: 2>&1 | tail -30`
Expected: 2 tests pass (spawn/stop + log file).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add background process supervisor with process-group kill"
```

---

## Task 7: Health-decision logic (`health.rs`, pure fn first)

**Files:**
- Create: `src-tauri/src/health.rs`
- Modify: `lib.rs` (`mod health;`)

**Interfaces:**
- Consumes: `model::Status`.
- Produces:
  - `struct Probe { pub pid_alive: bool, pub has_port: bool, pub port_open: bool }`
  - `fn decide_status(p: &Probe) -> Status` — pure: PID dead → Error; PID alive + no port → Running; PID alive + port open → Running; PID alive + port closed → Starting.
  - `fn port_open(port: u16) -> bool` — TCP connect to `127.0.0.1:port`, 300ms timeout.
  - `fn http_ok(port: u16, path: &str) -> bool` — `ureq` GET, true iff 2xx.

- [ ] **Step 1: Write failing test**

End `health.rs` with:
```rust
#[cfg(test)]
mod tests {
	use super::*;
	use crate::model::Status;

	#[test]
	fn dead_pid_is_error() {
		assert_eq!(decide_status(&Probe { pid_alive: false, has_port: true, port_open: false }), Status::Error);
	}
	#[test]
	fn alive_no_port_is_running() {
		assert_eq!(decide_status(&Probe { pid_alive: true, has_port: false, port_open: false }), Status::Running);
	}
	#[test]
	fn alive_port_open_is_running() {
		assert_eq!(decide_status(&Probe { pid_alive: true, has_port: true, port_open: true }), Status::Running);
	}
	#[test]
	fn alive_port_closed_is_starting() {
		assert_eq!(decide_status(&Probe { pid_alive: true, has_port: true, port_open: false }), Status::Starting);
	}
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cd src-tauri && cargo test health:: 2>&1 | tail -20`
Expected: compile error.

- [ ] **Step 3: Implement (pure fn + probes)**

Above the tests:
```rust
use crate::model::Status;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Inputs for a status decision.
pub struct Probe { pub pid_alive: bool, pub has_port: bool, pub port_open: bool }

/// Decide a background item's status from a probe. Pure.
pub fn decide_status(p: &Probe) -> Status {
	if !p.pid_alive { return Status::Error; }
	if !p.has_port { return Status::Running; }
	if p.port_open { Status::Running } else { Status::Starting }
}

/// True if a TCP connection to 127.0.0.1:port succeeds within 300ms.
pub fn port_open(port: u16) -> bool {
	let Ok(mut addrs) = format!("127.0.0.1:{port}").to_socket_addrs() else { return false; };
	addrs.next().map(|a| TcpStream::connect_timeout(&a, Duration::from_millis(300)).is_ok()).unwrap_or(false)
}

/// True if an HTTP GET to the port+path returns a 2xx.
pub fn http_ok(port: u16, path: &str) -> bool {
	let url = format!("http://127.0.0.1:{port}{path}");
	matches!(ureq::get(&url).timeout(Duration::from_millis(500)).call(), Ok(r) if r.status() < 300)
}
```
Add `mod health;` to `lib.rs`.

- [ ] **Step 4: Run, verify pass**

Run: `cd src-tauri && cargo test health:: 2>&1 | tail -20`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add health status decision logic and port/http probes"
```

---

## Task 8: Shared app state + terminal module (`state.rs`, `terminal.rs`)

**Files:**
- Create: `src-tauri/src/state.rs`, `src-tauri/src/terminal.rs`
- Modify: `lib.rs` (`mod state; mod terminal;`)

**Interfaces:**
- Produces:
  - `state::AppState { pub dir: PathBuf, pub config: Mutex<AppConfig>, pub running: Mutex<HashMap<String, supervisor::Running>>, pub statuses: Mutex<HashMap<String, Status>>, pub errors: Mutex<HashMap<String, String>> }`.
  - `terminal::open_folder(app_name: &str, dir: &str) -> Result<(), AppError>` — `osascript` to open Terminal/iTerm at `cd <dir>`.
  - `terminal::run_in_terminal(app_name: &str, dir: &str, env: &BTreeMap<String,String>, cmd: &str) -> Result<(), AppError>` — open terminal and run `cd <dir> && <exports> && <cmd>`.

- [ ] **Step 1: Implement `state.rs`** (no unit test — it's a data holder; verified via commands later)

```rust
use crate::model::{AppConfig, Status};
use crate::supervisor::Running;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// All shared, mutable app state behind locks.
pub struct AppState {
	pub dir: PathBuf,
	pub config: Mutex<AppConfig>,
	pub running: Mutex<HashMap<String, Running>>,
	pub statuses: Mutex<HashMap<String, Status>>,
	pub errors: Mutex<HashMap<String, String>>,
}
```

- [ ] **Step 2: Write failing test for the AppleScript builder**

End `terminal.rs` with:
```rust
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
```

- [ ] **Step 3: Run, verify fail**

Run: `cd src-tauri && cargo test terminal:: 2>&1 | tail -20`
Expected: compile error.

- [ ] **Step 4: Implement `terminal.rs`**

Above the tests:
```rust
use crate::model::AppError;
use std::collections::BTreeMap;
use std::process::Command;

/// Build the shell line run inside the terminal: cd + env exports + command.
pub fn build_command_line(dir: &str, env: &BTreeMap<String, String>, cmd: &str) -> String {
	let mut parts = vec![format!("cd '{}'", dir.replace('\'', "'\\''"))];
	for (k, v) in env {
		parts.push(format!("export {}='{}'", k, v.replace('\'', "'\\''")));
	}
	parts.push(cmd.to_string());
	parts.join(" && ")
}

/// Open a terminal window already cd'd into `dir`.
pub fn open_folder(app_name: &str, dir: &str) -> Result<(), AppError> {
	run_in_terminal(app_name, dir, &BTreeMap::new(), "clear")
}

/// Open a terminal window and run a command in `dir` with env exports.
pub fn run_in_terminal(app_name: &str, dir: &str, env: &BTreeMap<String, String>, cmd: &str) -> Result<(), AppError> {
	let line = build_command_line(dir, env, cmd);
	let script = match app_name {
		"iTerm" => format!(
			"tell application \"iTerm\"\n create window with default profile\n tell current session of current window to write text \"{}\"\nend tell",
			line.replace('\\', "\\\\").replace('"', "\\\"")
		),
		_ => format!(
			"tell application \"Terminal\"\n activate\n do script \"{}\"\nend tell",
			line.replace('\\', "\\\\").replace('"', "\\\"")
		),
	};
	let out = Command::new("osascript").arg("-e").arg(&script).output()
		.map_err(|e| AppError::Message(format!("osascript failed: {e}")))?;
	if out.status.success() { Ok(()) }
	else { Err(AppError::Message(String::from_utf8_lossy(&out.stderr).trim().to_string())) }
}
```
Add `mod state; mod terminal;` to `lib.rs`.

- [ ] **Step 5: Run, verify pass**

Run: `cd src-tauri && cargo test terminal:: 2>&1 | tail -20`
Expected: 1 test passes.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: add shared app state and terminal (osascript) integration"
```

---

## Task 9: Item CRUD commands + state wiring (`commands.rs`, manage `AppState`)

**Files:**
- Create: `src-tauri/src/commands.rs`
- Modify: `lib.rs` (manage state in setup, register handlers)

**Interfaces:**
- Consumes: everything above.
- Produces (Tauri commands; all `Result<_, AppError>`):
  - `get_items(state) -> Vec<ManagedItem>`
  - `add_item(state, item: ManagedItem) -> ManagedItem` (assigns `id` via uuid if empty, persists)
  - `update_item(state, item: ManagedItem) -> ()`
  - `delete_item(state, id: String) -> ()`
  - `reorder(state, ids: Vec<String>) -> ()`
  - `toggle_favorite(state, id: String) -> ()`
  - `detect_folder_cmd(path: String) -> detect::DetectResult`
  - `get_settings(state) -> Settings`, `update_settings(state, settings: Settings) -> ()`

- [ ] **Step 1: Implement commands**

Create `commands.rs`:
```rust
use crate::detect::{self, DetectResult};
use crate::model::{AppConfig, AppError, ManagedItem, Settings};
use crate::state::AppState;
use crate::store;
use tauri::State;

/// Persist the current in-memory config to disk.
fn persist(state: &AppState) -> Result<(), AppError> {
	let cfg = state.config.lock().unwrap();
	store::save_config(&state.dir, &cfg)
}

#[tauri::command]
pub fn get_items(state: State<AppState>) -> Vec<ManagedItem> {
	state.config.lock().unwrap().items.clone()
}

#[tauri::command]
pub fn add_item(state: State<AppState>, mut item: ManagedItem) -> Result<ManagedItem, AppError> {
	if item.id.is_empty() { item.id = uuid::Uuid::new_v4().to_string(); }
	{
		let mut cfg = state.config.lock().unwrap();
		item.order = cfg.items.len() as u32;
		cfg.items.push(item.clone());
	}
	persist(&state)?;
	Ok(item)
}

#[tauri::command]
pub fn update_item(state: State<AppState>, item: ManagedItem) -> Result<(), AppError> {
	{
		let mut cfg = state.config.lock().unwrap();
		if let Some(slot) = cfg.items.iter_mut().find(|i| i.id == item.id) { *slot = item; }
	}
	persist(&state)
}

#[tauri::command]
pub fn delete_item(state: State<AppState>, id: String) -> Result<(), AppError> {
	{
		let mut cfg = state.config.lock().unwrap();
		cfg.items.retain(|i| i.id != id);
	}
	persist(&state)
}

#[tauri::command]
pub fn reorder(state: State<AppState>, ids: Vec<String>) -> Result<(), AppError> {
	{
		let mut cfg = state.config.lock().unwrap();
		for (idx, id) in ids.iter().enumerate() {
			if let Some(it) = cfg.items.iter_mut().find(|i| &i.id == id) { it.order = idx as u32; }
		}
		cfg.items.sort_by_key(|i| i.order);
	}
	persist(&state)
}

#[tauri::command]
pub fn toggle_favorite(state: State<AppState>, id: String) -> Result<(), AppError> {
	{
		let mut cfg = state.config.lock().unwrap();
		if let Some(it) = cfg.items.iter_mut().find(|i| i.id == id) { it.favorite = !it.favorite; }
	}
	persist(&state)
}

#[tauri::command]
pub fn detect_folder_cmd(path: String) -> DetectResult {
	detect::detect_folder(std::path::Path::new(&path))
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
	state.config.lock().unwrap().settings.clone()
}

#[tauri::command]
pub fn update_settings(state: State<AppState>, settings: Settings) -> Result<(), AppError> {
	{ state.config.lock().unwrap().settings = settings; }
	persist(&state)
}

/// Build initial AppState by loading config from disk.
pub fn init_state(dir: std::path::PathBuf) -> AppState {
	let config = store::load_config(&dir);
	AppState {
		dir,
		config: std::sync::Mutex::new(config),
		running: std::sync::Mutex::new(std::collections::HashMap::new()),
		statuses: std::sync::Mutex::new(std::collections::HashMap::new()),
		errors: std::sync::Mutex::new(std::collections::HashMap::new()),
	}
}
```
Silence the unused `AppConfig` import if the compiler warns (remove it).

- [ ] **Step 2: Wire state + handlers into `lib.rs`**

In `lib.rs` `setup`, before building the tray, add:
```rust
let dir = store::config_dir()?;
app.manage(commands::init_state(dir));
```
Add `mod commands;` and extend the builder:
```rust
.invoke_handler(tauri::generate_handler![
	commands::get_items, commands::add_item, commands::update_item,
	commands::delete_item, commands::reorder, commands::toggle_favorite,
	commands::detect_folder_cmd, commands::get_settings, commands::update_settings
])
```

- [ ] **Step 3: Verify it builds**

Run: `cd src-tauri && cargo build 2>&1 | tail -20`
Expected: builds (warnings ok).

- [ ] **Step 4: Smoke test via frontend console**

Run `npm run tauri dev`, open the popover, in the webview devtools console run:
```js
const { invoke } = window.__TAURI__.core;
await invoke('add_item', { item: { id:'', name:'demo', kind:'project', dir:'/tmp', startCmd:'sleep 30', stopCmd:null, port:null, runMode:'background', brewFormula:null, order:0, favorite:false, env:{}, healthPath:null, autoStart:false }});
await invoke('get_items');
```
Expected: `get_items` returns the demo item; `~/Library/Application Support/com.abhi.menubar-service-manager/config.json` now contains it.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add item CRUD commands and shared app state"
```

---

## Task 10: Start/stop commands + status events + poll loop

**Files:**
- Modify: `src-tauri/src/commands.rs` (add start/stop/stop_all/open commands), `src-tauri/src/health.rs` (add `spawn_poll_loop`), `lib.rs` (start loop, quit handler)

**Interfaces:**
- Consumes: `supervisor`, `brew`, `terminal`, `health`, `state`.
- Produces commands: `start_item`, `stop_item`, `stop_all`, `open_browser`, `open_terminal`, `tail_log`. Plus `health::spawn_poll_loop(app: AppHandle)` emitting `status_changed` events with `ItemStatus` payloads, and a `set_status` helper that emits only on change.

- [ ] **Step 1: Add a status-emit helper + start/stop to `commands.rs`**

Append to `commands.rs`:
```rust
use crate::model::{ItemKind, ItemStatus, RunMode, Status};
use crate::{brew, health, supervisor, terminal};
use tauri::{AppHandle, Emitter, Manager};

/// Set an item's status and emit `status_changed` only if it changed.
pub fn set_status(app: &AppHandle, id: &str, status: Status) {
	let state = app.state::<AppState>();
	let changed = {
		let mut map = state.statuses.lock().unwrap();
		map.insert(id.to_string(), status) != Some(status)
	};
	if changed {
		let last_error = state.errors.lock().unwrap().get(id).cloned();
		let _ = app.emit("status_changed", ItemStatus { id: id.to_string(), status, last_error });
	}
}

fn find_item(state: &AppState, id: &str) -> Option<ManagedItem> {
	state.config.lock().unwrap().items.iter().find(|i| i.id == id).cloned()
}

#[tauri::command]
pub fn start_item(app: AppHandle, id: String) -> Result<(), AppError> {
	let state = app.state::<AppState>();
	let item = find_item(&state, &id).ok_or_else(|| AppError::Message("no such item".into()))?;
	state.errors.lock().unwrap().remove(&id);
	match item.kind {
		ItemKind::Brew => {
			let f = item.brew_formula.clone().ok_or_else(|| AppError::Message("no formula".into()))?;
			brew::brew_start(&f)?;
			set_status(&app, &id, Status::Running);
		}
		_ => match item.run_mode {
			RunMode::Background => {
				let logs = state.dir.join("logs");
				let running = supervisor::spawn_background(&item, &logs)?;
				state.running.lock().unwrap().insert(id.clone(), running);
				set_status(&app, &id, Status::Starting);
			}
			RunMode::Terminal => {
				let dir = item.dir.clone().ok_or_else(|| AppError::Message("no dir".into()))?;
				let cmd = item.start_cmd.clone().ok_or_else(|| AppError::Message("no cmd".into()))?;
				let app_name = state.config.lock().unwrap().settings.terminal_app.clone();
				terminal::run_in_terminal(&app_name, &dir, &item.env, &cmd)?;
				set_status(&app, &id, Status::Running);
			}
		},
	}
	Ok(())
}

#[tauri::command]
pub fn stop_item(app: AppHandle, id: String) -> Result<(), AppError> {
	let state = app.state::<AppState>();
	let item = find_item(&state, &id).ok_or_else(|| AppError::Message("no such item".into()))?;
	if let ItemKind::Brew = item.kind {
		if let Some(f) = &item.brew_formula { brew::brew_stop(f)?; }
	} else if let Some(mut r) = state.running.lock().unwrap().remove(&id) {
		supervisor::stop(&mut r)?;
	}
	set_status(&app, &id, Status::Stopped);
	Ok(())
}

#[tauri::command]
pub fn stop_all(app: AppHandle) -> Result<(), AppError> {
	let ids: Vec<String> = {
		let state = app.state::<AppState>();
		let running: Vec<String> = state.running.lock().unwrap().keys().cloned().collect();
		let brews: Vec<String> = state.config.lock().unwrap().items.iter()
			.filter(|i| matches!(i.kind, ItemKind::Brew)).map(|i| i.id.clone()).collect();
		running.into_iter().chain(brews).collect()
	};
	for id in ids { let _ = stop_item(app.clone(), id); }
	Ok(())
}

#[tauri::command]
pub fn open_browser(app: AppHandle, id: String) -> Result<(), AppError> {
	let state = app.state::<AppState>();
	let item = find_item(&state, &id).ok_or_else(|| AppError::Message("no such item".into()))?;
	let port = item.port.ok_or_else(|| AppError::Message("no port".into()))?;
	std::process::Command::new("open").arg(format!("http://localhost:{port}")).spawn()
		.map_err(|e| AppError::Message(e.to_string()))?;
	Ok(())
}

#[tauri::command]
pub fn open_terminal(app: AppHandle, id: String) -> Result<(), AppError> {
	let state = app.state::<AppState>();
	let item = find_item(&state, &id).ok_or_else(|| AppError::Message("no such item".into()))?;
	let dir = item.dir.clone().ok_or_else(|| AppError::Message("no dir".into()))?;
	let app_name = state.config.lock().unwrap().settings.terminal_app.clone();
	terminal::open_folder(&app_name, &dir)
}

#[tauri::command]
pub fn tail_log(app: AppHandle, id: String, lines: usize) -> Result<String, AppError> {
	let state = app.state::<AppState>();
	let path = state.dir.join("logs").join(format!("{id}.log"));
	let text = std::fs::read_to_string(&path).unwrap_or_default();
	let tail: Vec<&str> = text.lines().rev().take(lines).collect();
	Ok(tail.into_iter().rev().collect::<Vec<_>>().join("\n"))
}
```

- [ ] **Step 2: Add the poll loop to `health.rs`**

Append to `health.rs` (above tests):
```rust
use crate::commands::set_status;
use crate::model::{ItemKind, RunMode};
use crate::state::AppState;
use tauri::{AppHandle, Manager};

/// Background thread: poll every item's status and emit changes.
pub fn spawn_poll_loop(app: AppHandle) {
	std::thread::spawn(move || loop {
		let interval = {
			let st = app.state::<AppState>();
			let s = st.config.lock().unwrap().settings.poll_interval_sec.max(1);
			s
		};
		poll_once(&app);
		std::thread::sleep(std::time::Duration::from_secs(interval));
	});
}

/// One poll pass over all items.
pub fn poll_once(app: &AppHandle) {
	let state = app.state::<AppState>();
	let items = state.config.lock().unwrap().items.clone();
	for item in items {
		let current = state.statuses.lock().unwrap().get(&item.id).copied();
		if matches!(current, None | Some(Status::Stopped)) && !matches!(item.kind, ItemKind::Brew) {
			continue; // not started; leave as-is
		}
		let status = match item.kind {
			ItemKind::Brew => item.brew_formula.as_deref().map(crate::brew::brew_status).unwrap_or(Status::Stopped),
			_ => match item.run_mode {
				RunMode::Background => {
					let mut running = state.running.lock().unwrap();
					match running.get_mut(&item.id) {
						Some(r) => {
							let alive = crate::supervisor::is_alive(r);
							if !alive {
								state.errors.lock().unwrap().insert(item.id.clone(), "process exited".into());
							}
							let has_port = item.port.is_some();
							let port_open = match (item.port, item.health_path.as_deref()) {
								(Some(p), Some(path)) => http_ok(p, path),
								(Some(p), None) => port_open(p),
								_ => false,
							};
							decide_status(&Probe { pid_alive: alive, has_port, port_open })
						}
						None => Status::Stopped,
					}
				}
				RunMode::Terminal => match item.port {
					Some(p) => if port_open(p) { Status::Running } else { Status::Starting },
					None => current.unwrap_or(Status::Stopped),
				},
			},
		};
		set_status(app, &item.id, status);
	}
}
```

- [ ] **Step 3: Register new commands, start loop, add quit handler in `lib.rs`**

Add the six new commands to `generate_handler!`: `start_item, stop_item, stop_all, open_browser, open_terminal, tail_log`. In `setup`, after `app.manage(...)`:
```rust
health::spawn_poll_loop(app.handle().clone());
// auto-start items flagged autoStart
{
	let app_handle = app.handle().clone();
	let ids: Vec<String> = {
		let st = app_handle.state::<state::AppState>();
		let cfg = st.config.lock().unwrap();
		cfg.items.iter().filter(|i| i.auto_start).map(|i| i.id.clone()).collect()
	};
	for id in ids { let _ = commands::start_item(app_handle.clone(), id); }
}
```
Add an exit handler that SIGTERMs owned children. After `.run(...)` is not reachable; instead use `app.run(|app_handle, event| { ... })` form, or register on `RunEvent::ExitRequested`. Replace the final `.run(generate_context!())...` with:
```rust
	.build(tauri::generate_context!())
	.expect("error building app")
	.run(|app_handle, event| {
		if let tauri::RunEvent::ExitRequested { .. } = event {
			let st = app_handle.state::<state::AppState>();
			let mut running = st.running.lock().unwrap();
			for (_, r) in running.iter_mut() { let _ = supervisor::stop(r); }
		}
	});
```

- [ ] **Step 4: Verify build + manual run**

Run: `cd src-tauri && cargo build 2>&1 | tail -20` (expected: builds).
Run `npm run tauri dev`. Via devtools console: add an item with `startCmd:'python3 -m http.server 8000'`, `port:8000`, then `await invoke('start_item',{id})`. Within a few seconds `await invoke('get_items')` plus listening to events shows status going `starting` → `running`. `await invoke('open_browser',{id})` opens the page. `await invoke('stop_item',{id})` stops it.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add start/stop/open commands, status events, and poll loop"
```

---

## Task 11: Frontend types + pure UI helpers (`model.ts`, vitest)

**Files:**
- Create: `src/model.ts`, `src/model.test.ts`
- Modify: `package.json` (add vitest), `vite.config.ts` (test config if needed)

**Interfaces:**
- Produces:
  - `interface ManagedItem`, `interface Settings`, `type Status`, `interface ItemStatus` — mirror of the Rust shapes (camelCase keys).
  - `statusDot(status: Status): string` → emoji/char + class (●/◐/○/✖ mapping).
  - `matchesSearch(item: ManagedItem, query: string): boolean` — name/kind/port substring, case-insensitive.
  - `splitFavorites(items: ManagedItem[]): { favorites: ManagedItem[]; others: ManagedItem[] }` — preserves `order`.

- [ ] **Step 1: Add vitest**

```bash
npm install -D vitest
```
Add to `package.json` scripts: `"test": "vitest run"`.

- [ ] **Step 2: Write failing tests**

Create `src/model.test.ts`:
```ts
import { describe, it, expect } from 'vitest';
import { matchesSearch, splitFavorites, statusDot, type ManagedItem } from './model';

const base: ManagedItem = {
	id: '1', name: 'myapp', kind: 'project', dir: '/x', startCmd: 'npm run dev',
	stopCmd: null, port: 5173, runMode: 'background', brewFormula: null, order: 0,
	favorite: false, env: {}, healthPath: null, autoStart: false,
};

describe('model helpers', () => {
	it('matchesSearch on name, kind, port', () => {
		expect(matchesSearch(base, 'myap')).toBe(true);
		expect(matchesSearch(base, 'project')).toBe(true);
		expect(matchesSearch(base, '5173')).toBe(true);
		expect(matchesSearch(base, 'zzz')).toBe(false);
	});
	it('splitFavorites separates and preserves order', () => {
		const a = { ...base, id: 'a', favorite: true, order: 1 };
		const b = { ...base, id: 'b', favorite: false, order: 0 };
		const { favorites, others } = splitFavorites([a, b]);
		expect(favorites.map(i => i.id)).toEqual(['a']);
		expect(others.map(i => i.id)).toEqual(['b']);
	});
	it('statusDot maps each status', () => {
		expect(statusDot('running')).toContain('running');
		expect(statusDot('error')).toContain('error');
	});
});
```

- [ ] **Step 3: Run, verify fail**

Run: `npm test 2>&1 | tail -20`
Expected: FAIL (module `./model` not found).

- [ ] **Step 4: Implement `src/model.ts`**

```ts
/** Live status of an item — mirrors the Rust enum. */
export type Status = 'stopped' | 'starting' | 'running' | 'error';
export type ItemKind = 'project' | 'brew' | 'agent';
export type RunMode = 'background' | 'terminal';

/** A registered service — mirrors Rust `ManagedItem`. */
export interface ManagedItem {
	id: string;
	name: string;
	kind: ItemKind;
	dir: string | null;
	startCmd: string | null;
	stopCmd: string | null;
	port: number | null;
	runMode: RunMode;
	brewFormula: string | null;
	order: number;
	favorite: boolean;
	env: Record<string, string>;
	healthPath: string | null;
	autoStart: boolean;
}

export interface Settings {
	terminalApp: string;
	pollIntervalSec: number;
	browser: string;
	launchAtLogin: boolean;
}

export interface ItemStatus { id: string; status: Status; lastError: string | null; }

/** Return a status glyph + CSS class string. */
export function statusDot(status: Status): string {
	const map: Record<Status, string> = {
		running: '● running', starting: '◐ starting', stopped: '○ stopped', error: '✖ error',
	};
	return map[status];
}

/** Case-insensitive match across name, kind, and port. */
export function matchesSearch(item: ManagedItem, query: string): boolean {
	const q = query.trim().toLowerCase();
	if (!q) return true;
	return (
		item.name.toLowerCase().includes(q) ||
		item.kind.includes(q) ||
		(item.port != null && String(item.port).includes(q))
	);
}

/** Split items into favorites and others, each sorted by `order`. */
export function splitFavorites(items: ManagedItem[]): { favorites: ManagedItem[]; others: ManagedItem[] } {
	const sorted = [...items].sort((a, b) => a.order - b.order);
	return {
		favorites: sorted.filter(i => i.favorite),
		others: sorted.filter(i => !i.favorite),
	};
}
```

- [ ] **Step 5: Run, verify pass**

Run: `npm test 2>&1 | tail -20`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: add frontend model types and pure UI helpers with tests"
```

---

## Task 12: IPC wrapper (`ipc.ts`)

**Files:**
- Create: `src/ipc.ts`

**Interfaces:**
- Consumes: `@tauri-apps/api/core` (`invoke`), `@tauri-apps/api/event` (`listen`), `model.ts` types.
- Produces typed async fns: `getItems`, `addItem`, `updateItem`, `deleteItem`, `reorder`, `toggleFavorite`, `startItem`, `stopItem`, `stopAll`, `openBrowser`, `openTerminal`, `tailLog`, `detectFolder`, `getSettings`, `updateSettings`, and `onStatusChanged(cb)`.

- [ ] **Step 1: Implement (verified via Task 13 wiring, no unit test — thin pass-through)**

```ts
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { ManagedItem, Settings, ItemStatus } from './model';

export const getItems = () => invoke<ManagedItem[]>('get_items');
export const addItem = (item: ManagedItem) => invoke<ManagedItem>('add_item', { item });
export const updateItem = (item: ManagedItem) => invoke<void>('update_item', { item });
export const deleteItem = (id: string) => invoke<void>('delete_item', { id });
export const reorder = (ids: string[]) => invoke<void>('reorder', { ids });
export const toggleFavorite = (id: string) => invoke<void>('toggle_favorite', { id });
export const startItem = (id: string) => invoke<void>('start_item', { id });
export const stopItem = (id: string) => invoke<void>('stop_item', { id });
export const stopAll = () => invoke<void>('stop_all');
export const openBrowser = (id: string) => invoke<void>('open_browser', { id });
export const openTerminal = (id: string) => invoke<void>('open_terminal', { id });
export const tailLog = (id: string, lines: number) => invoke<string>('tail_log', { id, lines });
export const detectFolder = (path: string) => invoke('detect_folder_cmd', { path });
export const getSettings = () => invoke<Settings>('get_settings');
export const updateSettings = (settings: Settings) => invoke<void>('update_settings', { settings });

/** Subscribe to backend status changes. Returns an unlisten fn. */
export function onStatusChanged(cb: (s: ItemStatus) => void): Promise<UnlistenFn> {
	return listen<ItemStatus>('status_changed', (e) => cb(e.payload));
}
```

- [ ] **Step 2: Verify it type-checks**

Run: `npx tsc --noEmit 2>&1 | tail -20`
Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat: add typed IPC wrapper for backend commands and events"
```

---

## Task 13: Render list + rows + live status (`list.ts`, `row.ts`, `main.ts`)

**Files:**
- Create: `src/list.ts`, `src/row.ts`
- Modify: `src/main.ts`, `src/styles.css`

**Interfaces:**
- Consumes: `ipc.ts`, `model.ts`.
- Produces:
  - `row.ts`: `renderRow(item, status): HTMLElement` with status dot, name, port/kind label, action buttons (start/stop, open-browser if port, open-terminal if dir), and an expandable panel (log tail, edit, delete, favorite, autoStart toggles).
  - `list.ts`: `renderList(container, items, statuses, opts)` — search box, Stop-all, favorites section + collapsible "More (n)".
  - `main.ts`: bootstrap — load items + settings, render, subscribe to `status_changed`, refresh on changes.

- [ ] **Step 1: Implement `row.ts`**

```ts
import { statusDot, type ManagedItem, type Status } from './model';
import { startItem, stopItem, openBrowser, openTerminal, toggleFavorite, deleteItem, tailLog } from './ipc';

/** Render a single item row element. `onChange` re-renders the list. */
export function renderRow(item: ManagedItem, status: Status, onChange: () => void): HTMLElement {
	const row = document.createElement('div');
	row.className = 'row';
	const dot = statusDot(status).split(' ')[0];
	const cls = statusDot(status).split(' ')[1];
	const label = item.kind === 'brew' ? 'brew' : (item.port != null ? `:${item.port}` : item.runMode);
	const running = status === 'running' || status === 'starting';

	row.innerHTML = `
		<span class="dot ${cls}">${dot}</span>
		<span class="name">${item.name}</span>
		<span class="meta">${label}</span>
		<span class="actions"></span>`;
	const actions = row.querySelector('.actions')!;

	const btn = (text: string, title: string, fn: () => Promise<unknown>) => {
		const b = document.createElement('button');
		b.textContent = text; b.title = title;
		b.onclick = async (e) => { e.stopPropagation(); try { await fn(); } catch (err) { alert(String(err)); } onChange(); };
		actions.appendChild(b);
	};

	btn(running ? '■' : '▶', running ? 'Stop' : 'Start', () => running ? stopItem(item.id) : startItem(item.id));
	if (item.port != null) btn('↗', 'Open in browser', () => openBrowser(item.id));
	if (item.dir) btn('>_', 'Open terminal', () => openTerminal(item.id));

	// Expand panel on row-body click.
	row.onclick = async () => {
		const existing = row.querySelector('.expand');
		if (existing) { existing.remove(); return; }
		const panel = document.createElement('div');
		panel.className = 'expand';
		const log = await tailLog(item.id, 20).catch(() => '');
		panel.innerHTML = `<pre class="log">${log || '(no log)'}</pre>`;
		const fav = document.createElement('button');
		fav.textContent = item.favorite ? '★ Unfavorite' : '☆ Favorite';
		fav.onclick = async (e) => { e.stopPropagation(); await toggleFavorite(item.id); onChange(); };
		const del = document.createElement('button');
		del.textContent = 'Delete';
		del.onclick = async (e) => { e.stopPropagation(); if (confirm(`Delete ${item.name}?`)) { await deleteItem(item.id); onChange(); } };
		panel.append(fav, del);
		row.appendChild(panel);
	};
	return row;
}
```

- [ ] **Step 2: Implement `list.ts`**

```ts
import { matchesSearch, splitFavorites, type ManagedItem, type Status } from './model';
import { renderRow } from './row';
import { stopAll } from './ipc';

interface ListOpts { onChange: () => void; onAdd: () => void; onSettings: () => void; }

/** Render the full two-tier list into `container`. */
export function renderList(
	container: HTMLElement, items: ManagedItem[], statuses: Map<string, Status>, opts: ListOpts,
) {
	container.innerHTML = '';
	const header = document.createElement('div');
	header.className = 'header';
	const search = document.createElement('input');
	search.placeholder = 'Search…'; search.className = 'search';
	const stop = document.createElement('button');
	stop.textContent = '■ Stop all';
	stop.onclick = async () => { if (confirm('Stop all running services?')) { await stopAll(); opts.onChange(); } };
	header.append(search, stop);
	container.appendChild(header);

	const body = document.createElement('div');
	body.className = 'body';
	container.appendChild(body);

	const draw = () => {
		body.innerHTML = '';
		const q = search.value;
		const filtered = items.filter(i => matchesSearch(i, q));
		const { favorites, others } = splitFavorites(filtered);
		const status = (i: ManagedItem) => statuses.get(i.id) ?? 'stopped';

		if (favorites.length) {
			const h = document.createElement('div'); h.className = 'section'; h.textContent = 'FAVORITES';
			body.appendChild(h);
			favorites.forEach(i => body.appendChild(renderRow(i, status(i), opts.onChange)));
		}
		if (others.length) {
			if (q) { others.forEach(i => body.appendChild(renderRow(i, status(i), opts.onChange))); }
			else {
				const more = document.createElement('details');
				const sum = document.createElement('summary'); sum.textContent = `More (${others.length})`;
				more.appendChild(sum);
				others.forEach(i => more.appendChild(renderRow(i, status(i), opts.onChange)));
				body.appendChild(more);
			}
		}
	};
	search.oninput = draw;
	draw();

	const footer = document.createElement('div');
	footer.className = 'footer';
	const add = document.createElement('button'); add.textContent = '+ Add'; add.onclick = opts.onAdd;
	const set = document.createElement('button'); set.textContent = '⚙ Settings'; set.onclick = opts.onSettings;
	footer.append(add, set);
	container.appendChild(footer);
}
```

- [ ] **Step 3: Implement `main.ts` bootstrap**

```ts
import { getItems, onStatusChanged } from './ipc';
import { renderList } from './list';
import { openForm } from './form';
import { openSettings } from './settings';
import type { ManagedItem, Status } from './model';

const app = document.querySelector<HTMLDivElement>('#app')!;
const statuses = new Map<string, Status>();
let items: ManagedItem[] = [];

/** Reload items from backend and re-render. */
async function refresh() {
	items = await getItems();
	render();
}
function render() {
	renderList(app, items, statuses, {
		onChange: refresh,
		onAdd: () => openForm(null, refresh),
		onSettings: () => openSettings(refresh),
	});
}

onStatusChanged((s) => { statuses.set(s.id, s.status); render(); });
refresh();
```

- [ ] **Step 4: Add styles**

Append to `src/styles.css`:
```css
.header, .footer { display: flex; gap: 6px; padding: 6px; align-items: center; }
.search { flex: 1; }
.section { font-size: 10px; color: #888; padding: 4px 8px; }
.row { display: flex; align-items: center; gap: 6px; padding: 6px 8px; cursor: default; }
.row:hover { background: rgba(0,0,0,0.05); }
.name { flex: 1; font-weight: 500; }
.meta { color: #888; font-size: 11px; }
.dot.running { color: #2ecc40; } .dot.starting { color: #ffdc00; }
.dot.stopped { color: #aaa; } .dot.error { color: #ff4136; }
.actions button, .footer button, .header button { font-size: 11px; }
.expand { padding: 6px 8px; background: #f6f6f6; }
.log { max-height: 140px; overflow: auto; font: 10px monospace; white-space: pre-wrap; }
```
(Stub `form.ts`/`settings.ts` exports now so it compiles: create `src/form.ts` with `export function openForm(_i: unknown, _cb: () => void) {}` and `src/settings.ts` with `export function openSettings(_cb: () => void) {}` — replaced in Tasks 14–15.)

- [ ] **Step 5: Verify build + manual smoke**

Run: `npx tsc --noEmit 2>&1 | tail -20` (expected: clean).
Run `npm run tauri dev`. Add an item via console (Task 9 method), reopen popover: the row renders with a status dot and buttons; Start flips the dot to yellow→green; Stop-all confirms and stops; clicking a row expands the log panel.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: render service list, rows, actions, and live status updates"
```

---

## Task 14: Add/Edit form with detect prefill (`form.ts`)

**Files:**
- Modify: `src/form.ts` (replace stub)

**Interfaces:**
- Consumes: `ipc.ts` (`addItem`, `updateItem`, `detectFolder`), `@tauri-apps/plugin-dialog` `open` (folder picker), `model.ts`.
- Produces: `openForm(item: ManagedItem | null, onDone: () => void)` — modal overlay; folder pick triggers detect prefill; all fields editable (name, kind, dir, startCmd, stopCmd, port, runMode, env as KEY=VALUE lines, healthPath, favorite, autoStart, brewFormula); Save calls add or update.

- [ ] **Step 1: Add the dialog plugin**

```bash
cd src-tauri && cargo add tauri-plugin-dialog && cd ..
npm install @tauri-apps/plugin-dialog
```
Register in `lib.rs`: `.plugin(tauri_plugin_dialog::init())`. Add to `src-tauri/capabilities/default.json` permissions: `"dialog:allow-open"`.

- [ ] **Step 2: Implement `form.ts`**

```ts
import { open } from '@tauri-apps/plugin-dialog';
import { addItem, updateItem, detectFolder } from './ipc';
import type { ManagedItem, ItemKind, RunMode } from './model';

function blank(): ManagedItem {
	return {
		id: '', name: '', kind: 'project', dir: null, startCmd: null, stopCmd: null,
		port: null, runMode: 'background', brewFormula: null, order: 0, favorite: false,
		env: {}, healthPath: null, autoStart: false,
	};
}

/** Parse "KEY=VALUE" lines into an env record. */
function parseEnv(text: string): Record<string, string> {
	const env: Record<string, string> = {};
	for (const line of text.split('\n')) {
		const i = line.indexOf('=');
		if (i > 0) env[line.slice(0, i).trim()] = line.slice(i + 1).trim();
	}
	return env;
}
function envToText(env: Record<string, string>): string {
	return Object.entries(env).map(([k, v]) => `${k}=${v}`).join('\n');
}

/** Open the add/edit modal. `item` null = add. */
export function openForm(item: ManagedItem | null, onDone: () => void) {
	const data = item ? { ...item } : blank();
	const overlay = document.createElement('div');
	overlay.className = 'overlay';
	overlay.innerHTML = `
		<div class="modal">
			<label>Name <input id="f-name"></label>
			<label>Kind <select id="f-kind"><option value="project">project</option><option value="brew">brew</option><option value="agent">agent</option></select></label>
			<label>Folder <input id="f-dir" readonly><button id="f-pick">Pick…</button></label>
			<label>Start cmd <input id="f-cmd"></label>
			<label>Stop cmd <input id="f-stop"></label>
			<label>Port <input id="f-port" type="number"></label>
			<label>Run mode <select id="f-mode"><option value="background">background</option><option value="terminal">terminal</option></select></label>
			<label>Brew formula <input id="f-formula"></label>
			<label>Env (KEY=VALUE per line) <textarea id="f-env"></textarea></label>
			<label>Health path <input id="f-health" placeholder="/health"></label>
			<label><input type="checkbox" id="f-fav"> Favorite</label>
			<label><input type="checkbox" id="f-auto"> Auto-start on launch</label>
			<div class="modal-actions"><button id="f-cancel">Cancel</button><button id="f-save">Save</button></div>
		</div>`;
	document.body.appendChild(overlay);
	const $ = <T extends HTMLElement>(id: string) => overlay.querySelector<T>(id)!;

	const fill = (d: ManagedItem) => {
		$<HTMLInputElement>('#f-name').value = d.name;
		$<HTMLSelectElement>('#f-kind').value = d.kind;
		$<HTMLInputElement>('#f-dir').value = d.dir ?? '';
		$<HTMLInputElement>('#f-cmd').value = d.startCmd ?? '';
		$<HTMLInputElement>('#f-stop').value = d.stopCmd ?? '';
		$<HTMLInputElement>('#f-port').value = d.port != null ? String(d.port) : '';
		$<HTMLSelectElement>('#f-mode').value = d.runMode;
		$<HTMLInputElement>('#f-formula').value = d.brewFormula ?? '';
		$<HTMLTextAreaElement>('#f-env').value = envToText(d.env);
		$<HTMLInputElement>('#f-health').value = d.healthPath ?? '';
		$<HTMLInputElement>('#f-fav').checked = d.favorite;
		$<HTMLInputElement>('#f-auto').checked = d.autoStart;
	};
	fill(data);

	$<HTMLButtonElement>('#f-pick').onclick = async () => {
		const dir = await open({ directory: true });
		if (typeof dir !== 'string') return;
		const det = await detectFolder(dir) as { name: string; kind: ItemKind; startCmd: string | null; port: number | null };
		fill({ ...data, dir, name: data.name || det.name, kind: det.kind, startCmd: det.startCmd, port: det.port });
		data.dir = dir;
	};
	$<HTMLButtonElement>('#f-cancel').onclick = () => overlay.remove();
	$<HTMLButtonElement>('#f-save').onclick = async () => {
		const result: ManagedItem = {
			...data,
			name: $<HTMLInputElement>('#f-name').value,
			kind: $<HTMLSelectElement>('#f-kind').value as ItemKind,
			dir: $<HTMLInputElement>('#f-dir').value || null,
			startCmd: $<HTMLInputElement>('#f-cmd').value || null,
			stopCmd: $<HTMLInputElement>('#f-stop').value || null,
			port: $<HTMLInputElement>('#f-port').value ? Number($<HTMLInputElement>('#f-port').value) : null,
			runMode: $<HTMLSelectElement>('#f-mode').value as RunMode,
			brewFormula: $<HTMLInputElement>('#f-formula').value || null,
			env: parseEnv($<HTMLTextAreaElement>('#f-env').value),
			healthPath: $<HTMLInputElement>('#f-health').value || null,
			favorite: $<HTMLInputElement>('#f-fav').checked,
			autoStart: $<HTMLInputElement>('#f-auto').checked,
		};
		try { item ? await updateItem(result) : await addItem(result); overlay.remove(); onDone(); }
		catch (e) { alert(String(e)); }
	};
}
```

- [ ] **Step 3: Wire edit button**

In `row.ts` expand panel, add an Edit button that calls `openForm(item, onChange)` (import `openForm` from `./form`).

- [ ] **Step 4: Add modal styles**

Append to `styles.css`:
```css
.overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.3); display: flex; align-items: center; justify-content: center; }
.modal { background: #fff; padding: 12px; width: 320px; max-height: 90vh; overflow: auto; display: flex; flex-direction: column; gap: 6px; border-radius: 8px; }
.modal label { display: flex; flex-direction: column; font-size: 11px; gap: 2px; }
.modal-actions { display: flex; justify-content: flex-end; gap: 6px; margin-top: 8px; }
```

- [ ] **Step 5: Verify + smoke**

Run: `npx tsc --noEmit` (clean). Run `npm run tauri dev`: click **+ Add** → **Pick…** a Node project folder → name/cmd/port prefill → Save → row appears and persists across restart. Edit an item → change name → Save → updates.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: add/edit form with folder picker and auto-detect prefill"
```

---

## Task 15: Settings panel + launch-at-login (`settings.ts`)

**Files:**
- Modify: `src/settings.ts` (replace stub), `lib.rs` (autostart plugin)

**Interfaces:**
- Consumes: `ipc.ts` (`getSettings`, `updateSettings`).
- Produces: `openSettings(onDone)` modal — terminalApp select (Terminal/iTerm), pollIntervalSec number, launchAtLogin checkbox. Saving persists and toggles the macOS login item.

- [ ] **Step 1: Add autostart plugin**

```bash
cd src-tauri && cargo add tauri-plugin-autostart && cd ..
npm install @tauri-apps/plugin-autostart
```
Register in `lib.rs`: `.plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))`. Add capability `"autostart:allow-enable"`, `"autostart:allow-disable"`, `"autostart:allow-is-enabled"`.

- [ ] **Step 2: Implement `settings.ts`**

```ts
import { getSettings, updateSettings } from './ipc';
import { enable, disable } from '@tauri-apps/plugin-autostart';
import type { Settings } from './model';

/** Open the settings modal. */
export async function openSettings(onDone: () => void) {
	const s: Settings = await getSettings();
	const overlay = document.createElement('div');
	overlay.className = 'overlay';
	overlay.innerHTML = `
		<div class="modal">
			<label>Terminal app
				<select id="s-term"><option value="Terminal">Terminal</option><option value="iTerm">iTerm</option></select></label>
			<label>Poll interval (sec) <input id="s-poll" type="number" min="1"></label>
			<label><input type="checkbox" id="s-login"> Launch at login</label>
			<div class="modal-actions"><button id="s-cancel">Cancel</button><button id="s-save">Save</button></div>
		</div>`;
	document.body.appendChild(overlay);
	const $ = <T extends HTMLElement>(id: string) => overlay.querySelector<T>(id)!;
	$<HTMLSelectElement>('#s-term').value = s.terminalApp;
	$<HTMLInputElement>('#s-poll').value = String(s.pollIntervalSec);
	$<HTMLInputElement>('#s-login').checked = s.launchAtLogin;

	$<HTMLButtonElement>('#s-cancel').onclick = () => overlay.remove();
	$<HTMLButtonElement>('#s-save').onclick = async () => {
		const next: Settings = {
			...s,
			terminalApp: $<HTMLSelectElement>('#s-term').value,
			pollIntervalSec: Number($<HTMLInputElement>('#s-poll').value) || 3,
			launchAtLogin: $<HTMLInputElement>('#s-login').checked,
		};
		try {
			await updateSettings(next);
			next.launchAtLogin ? await enable() : await disable();
			overlay.remove(); onDone();
		} catch (e) { alert(String(e)); }
	};
}
```

- [ ] **Step 3: Verify + smoke**

Run: `npx tsc --noEmit` (clean). Run `npm run tauri dev`: open **⚙ Settings**, switch terminal to iTerm, change poll interval to 2, toggle launch-at-login, Save. Confirm an item's open-terminal now uses the chosen app, and `config.json` reflects the new settings.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: add settings panel with terminal choice, poll interval, launch-at-login"
```

---

## Task 16: Brew item add flow + final polish

**Files:**
- Modify: `src/form.ts` (formula picker hint), `src-tauri/src/commands.rs` (list available brew formulae), `lib.rs` (register), `src/ipc.ts`

**Interfaces:**
- Produces: `list_brew_formulae() -> Vec<String>` command (parse `brew services list` names) so the add form can offer a datalist when kind=brew.

- [ ] **Step 1: Add the command**

In `commands.rs`:
```rust
/// List formula names known to `brew services`.
#[tauri::command]
pub fn list_brew_formulae() -> Vec<String> {
	let Ok(out) = std::process::Command::new("brew").args(["services", "list"]).output() else { return vec![]; };
	let text = String::from_utf8_lossy(&out.stdout);
	brew::parse_brew_list(&text).into_keys().collect()
}
```
Register `list_brew_formulae` in `generate_handler!`. Add to `ipc.ts`: `export const listBrewFormulae = () => invoke<string[]>('list_brew_formulae');`

- [ ] **Step 2: Use it in the form**

In `form.ts`, when `#f-kind` changes to `brew`, populate a `<datalist>` bound to `#f-formula` from `listBrewFormulae()`, and clear the folder requirement (brew needs no `dir`). Add a `change` listener on `#f-kind`.

- [ ] **Step 3: Smoke test**

Run `npm run tauri dev`: Add → kind=brew → formula field suggests installed formulae (e.g. `mysql`) → Save → row shows brew status from `brew services list`; Start/Stop drive `brew services`.

- [ ] **Step 4: Run the full test suite**

Run: `cd src-tauri && cargo test 2>&1 | tail -20` (expected: all Rust tests pass).
Run: `npm test 2>&1 | tail -10` (expected: all frontend tests pass).

- [ ] **Step 5: Manual smoke checklist (from spec)**

Walk through and confirm each:
1. Add a real Node app → Start → dot yellow→green → ↗ opens browser → >_ opens terminal → Stop.
2. Add brew MySQL → Start/Stop reflects in `brew services list`.
3. Add an agent (kind=agent, runMode=terminal, e.g. `claude`) → Start opens a terminal window running it.
4. Quit the app (tray → right-click Quit, or Cmd-Q on focus) → confirm background children are gone (`pgrep -f` the command).
5. Corrupt `config.json` (write garbage) → relaunch → app loads empty, `config.bad.json` exists.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: add brew formula picker and finalize manual smoke pass"
```

---

## Self-Review (completed by plan author)

**Spec coverage:**
- Add app folders → Tasks 9, 14. Start/stop + status → Tasks 6, 7, 10. Background no-foreground-terminal → Task 6. Open browser → Task 10. Open terminal in folder → Tasks 8, 13. Brew services → Tasks 5, 16. Terminal agents (terminal run mode) → Tasks 8, 10. Unified model → Task 2. Per-item run mode → Tasks 2, 10. Status (process+port, HTTP health) → Task 7. Auto-detect add → Tasks 4, 14. Configurable terminal → Tasks 8, 15. Webview popover UI → Tasks 1, 13. Favorites + two-tier + search + Stop-all → Tasks 11, 13, 10. env/healthPath/autoStart → Tasks 2, 10, 14. Persistence + corrupt recovery → Task 3. Error handling (per-row, never crash, confirm destructive) → Tasks 10, 13, 14. Testing → every task is TDD where logic is pure; manual smoke → Task 16.
- Quit kills children → Task 10. Launch-at-login → Task 15.

**Placeholder scan:** No "TBD/TODO"; the only stubs (`form.ts`/`settings.ts` in Task 13) are explicitly replaced in Tasks 14–15 with full code.

**Type consistency:** Status strings `stopped|starting|running|error`, kinds `project|brew|agent`, run modes `background|terminal` used verbatim across Rust (`#[serde(rename_all="lowercase")]`) and TS. Command names match between `commands.rs` registration and `ipc.ts` invokes (`detect_folder_cmd`, `list_brew_formulae`, etc.). camelCase JSON keys (`startCmd`, `runMode`, `healthPath`, `autoStart`, `brewFormula`, `terminalApp`, `pollIntervalSec`, `launchAtLogin`) consistent via serde `rename` and TS interfaces.

**Known follow-ups (not blocking v1):** drag-to-reorder UI (command exists, Task 9; wiring deferred); aggregate tray-icon color tint; immediate (non-poll) child-exit detection — poll loop catches exits within one interval.
