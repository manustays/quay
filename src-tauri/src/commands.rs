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

/// Snapshot the live `id → pid` map and write it to `pids.json`.
///
/// Called after the `running` map changes so a relaunched app can reattach to
/// processes that outlived it. The snapshot is taken under the lock, then the
/// (blocking) file write happens after the guard drops.
pub fn persist_pids(state: &AppState) {
	let snapshot: std::collections::HashMap<String, u32> = {
		let running = state.running.lock().unwrap();
		running.iter().map(|(id, r)| (id.clone(), r.pid)).collect()
	};
	let _ = store::save_pids(&state.dir, &snapshot);
}

/// Poll `pidfile` for a PID written by a freshly launched terminal shell.
///
/// The shell writes `echo $$ > pidfile` asynchronously after the launch command
/// (`osascript`/`open`) returns, so we retry briefly. Returns the parsed PID, or
/// `None` if it never appears within the budget (≈2 s).
fn read_pidfile(pidfile: &std::path::Path) -> Option<u32> {
	for _ in 0..20 {
		if let Ok(text) = std::fs::read_to_string(pidfile) {
			if let Ok(pid) = text.trim().parse::<u32>() {
				return Some(pid);
			}
		}
		std::thread::sleep(std::time::Duration::from_millis(100));
	}
	None
}

/// Stop every tracked process **except** terminal items on app shutdown.
///
/// Terminal items are user-owned windows: quitting Quay must not close the user's
/// Codeburn/Claude session. Their PIDs are kept in `running` and persisted to
/// `pids.json` so the next launch can reattach and restore Running. Background/brew/
/// docker entries are stopped as before. Replaces the old "stop everything + clear
/// pids.json" shutdown.
pub fn shutdown_stop_non_terminal(state: &AppState) {
	let terminal_ids: std::collections::HashSet<String> = {
		let cfg = state.config.lock().unwrap();
		cfg.items
			.iter()
			.filter(|i| matches!(i.run_mode, crate::model::RunMode::Terminal))
			.map(|i| i.id.clone())
			.collect()
	};
	// Drain the non-terminal entries out under a scoped lock; leave terminals in.
	let to_stop: Vec<supervisor::Running> = {
		let mut running = state.running.lock().unwrap();
		let ids: Vec<String> =
			running.keys().filter(|id| !terminal_ids.contains(*id)).cloned().collect();
		ids.into_iter().filter_map(|id| running.remove(&id)).collect()
	};
	for mut r in to_stop {
		let _ = supervisor::stop(&mut r);
	}
	// Persist the survivors (terminal items) for reattach on next launch.
	persist_pids(state);
}

/// Return all registered items in their current order.
#[tauri::command]
pub fn get_items(state: State<AppState>) -> Vec<ManagedItem> {
	state.config.lock().unwrap().items.clone()
}

/// Canonicalize the group label: trim whitespace, drop empty to `None` (so
/// "api", "api " and "" can't silently become distinct groups).
fn normalize_group(item: &mut ManagedItem) {
	item.group = item
		.group
		.take()
		.map(|g| g.trim().to_string())
		.filter(|g| !g.is_empty());
}

/// Add a new item; assigns a uuid if `item.id` is empty, then persists.
#[tauri::command]
pub fn add_item(state: State<AppState>, mut item: ManagedItem) -> Result<ManagedItem, AppError> {
	if item.id.is_empty() {
		item.id = uuid::Uuid::new_v4().to_string();
	}
	normalize_group(&mut item);
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
pub fn update_item(state: State<AppState>, mut item: ManagedItem) -> Result<(), AppError> {
	normalize_group(&mut item);
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
pub fn delete_item(app: AppHandle, state: State<AppState>, id: String) -> Result<(), AppError> {
	{
		let mut cfg = state.config.lock().unwrap();
		cfg.items.retain(|i| i.id != id);
	}
	// Drop live status/error too, so a deleted errored item can't keep the tray
	// beacon lit (the poll loop only iterates configured items and would never
	// clear it).
	state.statuses.lock().unwrap().remove(&id);
	state.errors.lock().unwrap().remove(&id);
	crate::update_tray_icon(&app);
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
		suppress_hide: std::sync::atomic::AtomicBool::new(false),
		visible: std::sync::atomic::AtomicBool::new(false),
		update_in_flight: std::sync::atomic::AtomicBool::new(false),
	}
}

/// Suppress (or re-enable) hide-on-blur, e.g. while a native dialog is open.
#[tauri::command]
pub fn set_suppress_hide(state: tauri::State<AppState>, value: bool) {
	state.suppress_hide.store(value, std::sync::atomic::Ordering::Relaxed);
}

// ── Start / stop commands ────────────────────────────────────────────────────

use crate::model::{ItemKind, ItemStatus, RunMode, Status};
use crate::{brew, docker, supervisor, terminal};
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
		crate::update_tray_icon(app);
	}
}

/// Return the current live status (and last error) of every tracked item.
///
/// `status_changed` events are only emitted on *change*, so a frontend that
/// subscribes after the startup poll would otherwise miss the initial states.
/// The frontend calls this once on mount to seed its status map.
#[tauri::command]
pub fn get_statuses(state: State<AppState>) -> Vec<ItemStatus> {
	let statuses = state.statuses.lock().unwrap();
	let errors = state.errors.lock().unwrap();
	statuses
		.iter()
		.map(|(id, &status)| ItemStatus {
			id: id.clone(),
			status,
			last_error: errors.get(id).cloned(),
		})
		.collect()
}

/// Look up a `ManagedItem` by id in the current config.
fn find_item(state: &AppState, id: &str) -> Option<ManagedItem> {
	state.config.lock().unwrap().items.iter().find(|i| i.id == id).cloned()
}

/// If `item`'s configured port already has a live listener we can identify, adopt
/// that process (insert an adopted `Running`, persist pids, mark Running) and return
/// `true`. Returns `false` when there is no port, nothing is listening, or no PID can
/// be resolved — in which case the caller should spawn instead.
///
/// Liveness is a raw TCP check (`port_open`), not the HTTP health path: a socket in
/// LISTEN state is the right signal for "something already owns this port", and it is
/// not subject to a still-warming server returning non-2xx. We require a resolvable
/// PID so the adopted entry is always tracked in the `running` map — an untracked
/// "Running" item would be flipped back to Stopped by the next poll pass.
///
/// Shared by `start_item` and the launch-time port sweep. Caller must ensure the item
/// is `RunMode::Background` (terminal/brew items must not enter the `running` map).
pub fn adopt_if_listening(app: &AppHandle, item: &ManagedItem) -> bool {
	let Some(p) = item.port else { return false; };
	if !crate::health::port_open(p) { return false; }
	let Some(pid) = supervisor::pids_listening(p).first().copied() else { return false; };
	let state = app.state::<AppState>();
	let log_path = state.dir.join("logs").join(format!("{}.log", item.id));
	state.running.lock().unwrap().insert(item.id.clone(), supervisor::adopt(pid, log_path));
	persist_pids(&state);
	set_status(app, &item.id, Status::Running);
	true
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
		ItemKind::Docker => {
			// Fail safe if the daemon is down. The frontend pre-checks and prompts,
			// but auto-start (and any missed check) reaches here directly — the
			// `DOCKER_DAEMON_DOWN` sentinel lets the FE recover with the same prompt.
			if !docker::daemon_running() {
				return Err(AppError::Message("DOCKER_DAEMON_DOWN".into()));
			}
			docker::docker_start(&item)?;
			// Mark Starting; the poll loop flips to Running once `docker ps` confirms.
			set_status(&app, &id, Status::Starting);
		}
		_ => match item.run_mode {
			RunMode::Background => {
				// If the configured port is already serving — e.g. a previous instance
				// orphaned across an app restart — adopt it instead of spawning a
				// duplicate that can't bind and would leave two processes behind.
				if adopt_if_listening(&app, &item) {
					return Ok(());
				}
				let logs = state.dir.join("logs");
				std::fs::create_dir_all(&logs)?;
				let running = supervisor::spawn_background(&item, &logs)?;
				state.running.lock().unwrap().insert(id.clone(), running);
				persist_pids(&state);
				set_status(&app, &id, Status::Starting);
			}
			RunMode::Terminal => {
				// Already tracked and alive (e.g. reattached at launch, then hit again
				// by the auto-start loop) — don't open a second window.
				let already_alive = {
					let mut running = state.running.lock().unwrap();
					running.get_mut(&id).map(|r| supervisor::is_alive(r)).unwrap_or(false)
				};
				if already_alive {
					set_status(&app, &id, Status::Running);
					return Ok(());
				}
				let dir = item.dir.clone().ok_or_else(|| AppError::Message("no dir".into()))?;
				let cmd = item.start_cmd.clone().ok_or_else(|| AppError::Message("no cmd".into()))?;
				let app_name = state.config.lock().unwrap().settings.terminal_app.clone();
				let logs = state.dir.join("logs");
				std::fs::create_dir_all(&logs)?;
				// The terminal's shell writes its own PID here so we can poll liveness
				// (the process is unowned — launched inside Terminal/iTerm/etc.).
				let pidfile = logs.join(format!("{id}.term.pid"));
				let _ = std::fs::remove_file(&pidfile);
				terminal::run_in_terminal(&app_name, &dir, &item.env, &cmd, Some(&pidfile))?;
				// If the PID never lands (slow launch / write failure) we fall back to
				// the pre-fix behavior — Running, untracked. No worse than before.
				if let Some(pid) = read_pidfile(&pidfile) {
					let log_path = logs.join(format!("{id}.log"));
					state.running.lock().unwrap().insert(id.clone(), supervisor::adopt(pid, log_path));
					persist_pids(&state);
					let _ = std::fs::remove_file(&pidfile);
				}
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
	} else if let ItemKind::Docker = item.kind {
		if let Some(n) = item.container_name.as_deref().filter(|n| !n.trim().is_empty()) {
			docker::docker_stop(n)?;
		}
	} else {
		// Take the entry out under a scoped lock, then stop after the guard drops
		// so the mutex is never held across the (potentially blocking) stop call.
		let taken = { state.running.lock().unwrap().remove(&id) };
		if let Some(mut r) = taken {
			supervisor::stop(&mut r)?;
		}
		// If a port is configured and something is still listening — an orphan we
		// never owned, or a grandchild that escaped the process-group kill — free
		// the port so the user isn't left with an unkillable background service.
		if let Some(p) = item.port {
			if crate::health::port_open(p) {
				supervisor::stop_port(p);
			}
		}
		// Drop a terminal item's pid-capture file if one lingered.
		let _ = std::fs::remove_file(state.dir.join("logs").join(format!("{id}.term.pid")));
		persist_pids(&state);
	}
	set_status(&app, &id, Status::Stopped);
	Ok(())
}

/// Stop all running items (background + brew). Terminal items are excluded — they
/// are user-owned windows that Stop-All deliberately leaves open.
#[tauri::command]
pub fn stop_all(app: AppHandle) -> Result<(), AppError> {
	let ids: Vec<String> = {
		let state = app.state::<AppState>();
		let statuses = state.statuses.lock().unwrap();
		let cfg = state.config.lock().unwrap();
		// Terminal items live in `running` for liveness tracking but must not be torn
		// down by Stop-All (parity with leaving them open on app quit).
		let is_terminal = |id: &str| {
			cfg.items.iter().any(|i| i.id == id && matches!(i.run_mode, RunMode::Terminal))
		};
		let running: Vec<String> = state
			.running
			.lock()
			.unwrap()
			.keys()
			.filter(|id| !is_terminal(id))
			.cloned()
			.collect();
		// Brew: always considered (launchctl is the source of truth). Docker: only
		// containers we currently see Running/Starting, so Stop-All never tears down
		// a container that merely happens to be configured but was started elsewhere.
		let extra: Vec<String> = cfg.items.iter().filter(|i| match i.kind {
			ItemKind::Brew => true,
			ItemKind::Docker => matches!(
				statuses.get(&i.id),
				Some(Status::Running | Status::Starting)
			),
			_ => false,
		}).map(|i| i.id.clone()).collect();
		running.into_iter().chain(extra).collect()
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

/// List installed terminal apps for the settings picker. Terminal.app is always
/// present; others appear when their `.app` bundle or CLI binary is found.
#[tauri::command]
pub fn get_terminals() -> Vec<String> {
	terminal::installed_terminals()
}

/// List formula names known to `brew services`.
///
/// Runs `brew services list`, parses the output via `brew::parse_brew_list`, and
/// returns only the formula name keys. Returns an empty vec if brew is unavailable.
#[tauri::command]
pub fn list_brew_formulae() -> Vec<String> {
	let Some(text) = brew::services_list_raw() else {
		return vec![];
	};
	brew::parse_brew_list(&text).into_keys().collect()
}

/// List installed docker image "repo:tag" strings for the add-service autocomplete.
/// Empty when Docker is unavailable (CLI missing or daemon down).
#[tauri::command]
pub fn list_docker_images() -> Vec<String> {
	docker::list_images()
}

/// Signal a discovered (unmanaged) listener: SIGTERM, or SIGKILL when `force`.
///
/// The PID is revalidated against the port's current listeners immediately
/// before signalling, so a stale radar row (process exited, PID reused) can't
/// kill an unrelated process. Signals the PID only — never its process group,
/// which we did not create.
#[tauri::command]
pub fn kill_discovered(pid: u32, port: u16, force: bool) -> Result<(), AppError> {
	if !supervisor::pids_listening(port).contains(&pid) {
		return Err(AppError::Message(format!("process {pid} no longer listens on :{port}")));
	}
	let sig = if force { libc::SIGKILL } else { libc::SIGTERM };
	unsafe { libc::kill(pid as i32, sig) };
	Ok(())
}

/// Hide a port from the discovered-listeners section, persistently.
/// Un-ignoring happens in Settings (the whole `Settings` is saved back).
#[tauri::command]
pub fn ignore_port(state: State<AppState>, port: u16) -> Result<(), AppError> {
	{
		let mut cfg = state.config.lock().unwrap();
		if !cfg.settings.ignored_ports.contains(&port) {
			cfg.settings.ignored_ports.push(port);
			cfg.settings.ignored_ports.sort_unstable();
		}
	}
	persist(&state)
}

/// True if the Docker daemon is currently responding.
#[tauri::command]
pub fn docker_daemon_running() -> bool {
	docker::daemon_running()
}

/// Launch Docker Desktop and wait up to ~60s for the daemon. Returns Ok(true) if
/// it came up, Ok(false) on timeout, Err if Docker Desktop couldn't be launched.
#[tauri::command]
pub fn start_docker_daemon() -> Result<bool, AppError> {
	docker::start_daemon()?;
	Ok(docker::wait_for_daemon(std::time::Duration::from_secs(60)))
}
