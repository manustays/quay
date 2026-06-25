use crate::detect::{self, DetectResult};
use crate::model::{AppError, ManagedItem, Settings};
use crate::state::AppState;
use crate::store;
use tauri::State;

/// Persist the current in-memory config to disk.
fn persist(state: &AppState) -> Result<(), AppError> {
	let cfg = state.config.lock().unwrap();
	store::save_config(&state.dir, &cfg)
}

/// Return all registered items in their current order.
#[tauri::command]
pub fn get_items(state: State<AppState>) -> Vec<ManagedItem> {
	state.config.lock().unwrap().items.clone()
}

/// Add a new item; assigns a uuid if `item.id` is empty, then persists.
#[tauri::command]
pub fn add_item(state: State<AppState>, mut item: ManagedItem) -> Result<ManagedItem, AppError> {
	if item.id.is_empty() {
		item.id = uuid::Uuid::new_v4().to_string();
	}
	{
		let mut cfg = state.config.lock().unwrap();
		item.order = cfg.items.len() as u32;
		cfg.items.push(item.clone());
	}
	persist(&state)?;
	Ok(item)
}

/// Replace an existing item by `id`, then persist.
#[tauri::command]
pub fn update_item(state: State<AppState>, item: ManagedItem) -> Result<(), AppError> {
	{
		let mut cfg = state.config.lock().unwrap();
		if let Some(slot) = cfg.items.iter_mut().find(|i| i.id == item.id) {
			*slot = item;
		}
	}
	persist(&state)
}

/// Remove the item with `id` and persist.
#[tauri::command]
pub fn delete_item(state: State<AppState>, id: String) -> Result<(), AppError> {
	{
		let mut cfg = state.config.lock().unwrap();
		cfg.items.retain(|i| i.id != id);
	}
	persist(&state)
}

/// Reorder items to match `ids` (frontend drag-drop), then persist.
#[tauri::command]
pub fn reorder(state: State<AppState>, ids: Vec<String>) -> Result<(), AppError> {
	{
		let mut cfg = state.config.lock().unwrap();
		for (idx, id) in ids.iter().enumerate() {
			if let Some(it) = cfg.items.iter_mut().find(|i| &i.id == id) {
				it.order = idx as u32;
			}
		}
		cfg.items.sort_by_key(|i| i.order);
	}
	persist(&state)
}

/// Toggle the `favorite` flag on the item with `id`, then persist.
#[tauri::command]
pub fn toggle_favorite(state: State<AppState>, id: String) -> Result<(), AppError> {
	{
		let mut cfg = state.config.lock().unwrap();
		if let Some(it) = cfg.items.iter_mut().find(|i| i.id == id) {
			it.favorite = !it.favorite;
		}
	}
	persist(&state)
}

/// Inspect a folder path and return a suggested item config.
#[tauri::command]
pub fn detect_folder_cmd(path: String) -> DetectResult {
	detect::detect_folder(std::path::Path::new(&path))
}

/// Return the current app-wide settings.
#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
	state.config.lock().unwrap().settings.clone()
}

/// Replace app-wide settings and persist.
#[tauri::command]
pub fn update_settings(state: State<AppState>, settings: Settings) -> Result<(), AppError> {
	{
		state.config.lock().unwrap().settings = settings;
	}
	persist(&state)
}

/// Build initial `AppState` by loading config from disk (returns default if missing).
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

// ── Start / stop commands ────────────────────────────────────────────────────

use crate::model::{ItemKind, ItemStatus, RunMode, Status};
use crate::{brew, supervisor, terminal};
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

/// Look up a `ManagedItem` by id in the current config.
fn find_item(state: &AppState, id: &str) -> Option<ManagedItem> {
	state.config.lock().unwrap().items.iter().find(|i| i.id == id).cloned()
}

/// Start a managed item; sets status to Starting (background) or Running (brew/terminal).
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
				std::fs::create_dir_all(&logs)?;
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

/// Stop a managed item and set its status to Stopped.
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

/// Stop all running items (background + brew).
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

/// Open `http://localhost:<port>` in the system browser.
#[tauri::command]
pub fn open_browser(app: AppHandle, id: String) -> Result<(), AppError> {
	let state = app.state::<AppState>();
	let item = find_item(&state, &id).ok_or_else(|| AppError::Message("no such item".into()))?;
	let port = item.port.ok_or_else(|| AppError::Message("no port".into()))?;
	std::process::Command::new("open").arg(format!("http://localhost:{port}")).spawn()
		.map_err(|e| AppError::Message(e.to_string()))?;
	Ok(())
}

/// Open a terminal window cd'd into the item's directory.
#[tauri::command]
pub fn open_terminal(app: AppHandle, id: String) -> Result<(), AppError> {
	let state = app.state::<AppState>();
	let item = find_item(&state, &id).ok_or_else(|| AppError::Message("no such item".into()))?;
	let dir = item.dir.clone().ok_or_else(|| AppError::Message("no dir".into()))?;
	let app_name = state.config.lock().unwrap().settings.terminal_app.clone();
	terminal::open_folder(&app_name, &dir)
}

/// Return the last `lines` lines from the item's log file (empty string if none).
#[tauri::command]
pub fn tail_log(app: AppHandle, id: String, lines: usize) -> Result<String, AppError> {
	let state = app.state::<AppState>();
	let path = state.dir.join("logs").join(format!("{id}.log"));
	let text = std::fs::read_to_string(&path).unwrap_or_default();
	let tail: Vec<&str> = text.lines().rev().take(lines).collect();
	Ok(tail.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

/// List formula names known to `brew services`.
///
/// Runs `brew services list`, parses the output via `brew::parse_brew_list`, and
/// returns only the formula name keys. Returns an empty vec if brew is unavailable.
#[tauri::command]
pub fn list_brew_formulae() -> Vec<String> {
	let Ok(out) = std::process::Command::new("brew").args(["services", "list"]).output() else {
		return vec![];
	};
	let text = String::from_utf8_lossy(&out.stdout);
	brew::parse_brew_list(&text).into_keys().collect()
}
