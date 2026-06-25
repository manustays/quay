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
