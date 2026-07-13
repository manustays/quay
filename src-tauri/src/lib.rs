pub mod agent_radar;
pub mod brew;
pub mod commands;
pub mod detect;
pub mod docker;
pub mod health;
pub mod hooks_install;
pub mod metrics;
pub mod model;
pub mod scanner;
pub mod state;
pub mod store;
pub mod supervisor;
pub mod terminal;

use tauri::{
	Emitter, Manager,
	menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem},
	tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
	WindowEvent,
};

/// How often the background loop re-checks for updates after the initial launch
/// check (seconds). Daily. The `update_in_flight` guard keeps a manual check from
/// overlapping this one.
const UPDATE_CHECK_INTERVAL_SECS: u64 = 86_400;

/// Reflect service health *and* waiting agents on the tray. Icon precedence:
/// any service `Error` (red) > any waiting agent (amber submerged-buoy glow) >
/// any service `Starting` (amber) > nominal (monochrome template, theme-adaptive).
/// Error stays top (a broken managed service is a hard failure); waiting beats
/// starting — it's the headline signal, and its distinct glyph separates it from
/// plain amber. When the `waitingTitleBadge` setting is on and agents are waiting,
/// the menubar title shows the count; otherwise the title is cleared.
///
/// Safe to call from any thread (e.g. the health-poll loop): tray mutation is
/// dispatched to the main thread, and the inputs are read inside the closure so the
/// icon reflects state at apply time, not at call time. The waiting count itself is
/// refreshed separately by [`refresh_waiting_badge`].
pub fn update_tray_icon(app: &tauri::AppHandle) {
	use std::sync::atomic::Ordering;
	let app = app.clone();
	let _ = app.clone().run_on_main_thread(move || {
		let st = app.state::<state::AppState>();
		// Two short, sequential (never nested) locks: aggregate the statuses, then
		// read the one settings flag. The waiting count is a lock-free atomic.
		let aggregate = {
			let statuses = st.statuses.lock().unwrap();
			health::aggregate_status(statuses.values().copied())
		};
		let waiting = st.waiting_count.load(Ordering::Relaxed);
		let title_badge = st.config.lock().unwrap().settings.waiting_title_badge;
		let (icon, is_template) = match (aggregate, waiting > 0) {
			(Some(model::Status::Error), _) => (tauri::include_image!("icons/tray-error.png"), false),
			(_, true) => (tauri::include_image!("icons/tray-waiting.png"), false),
			(Some(_), false) => (tauri::include_image!("icons/tray-starting.png"), false),
			(None, false) => (tauri::include_image!("icons/tray.png"), true),
		};
		if let Some(tray) = app.tray_by_id("main") {
			let _ = tray.set_icon_with_as_template(Some(icon), is_template);
			let title = (title_badge && waiting > 0).then(|| format!("{waiting}"));
			let _ = tray.set_title(title.as_deref());
		}
	});
}

/// Recompute the waiting-agent count from the hook-state files and refresh the tray.
/// Called from the always-on poll loop (`health::spawn_poll_loop`) so the menubar
/// reflects waiting agents even while the popover — and thus the heavy radar scan —
/// is closed. Cheap: a directory read of small JSON files plus a `kill(pid, 0)`
/// liveness check per waiting file (from the PIDs `scan` stamped), no `ps`/`sysinfo`.
pub fn refresh_waiting_badge(app: &tauri::AppHandle) {
	use std::sync::atomic::Ordering;
	let st = app.state::<state::AppState>();
	let ignored = st.config.lock().unwrap().settings.ignored_agents.clone();
	let dir = st.dir.join("agent-state");
	let count = {
		let pids = st.last_agent_pids.lock().unwrap().clone();
		agent_radar::waiting_count(&dir, &ignored, &pids)
	};
	// A phantom count — a `waiting` file whose session died without a clearing
	// hook — otherwise self-heals only on a popover scan. When the badge would
	// show, sweep orphaned waiting files (crashed/exited: no live process, event
	// older than the grace) so it clears without one. The process enumeration
	// runs only in this `count > 0` branch, so it's free while nothing waits.
	let count = if count > 0 {
		agent_radar::prune_orphan_hook_states(
			&dir,
			&agent_radar::live_agent_keys(&ignored),
			std::time::SystemTime::now(),
		);
		let pids = st.last_agent_pids.lock().unwrap().clone();
		agent_radar::waiting_count(&dir, &ignored, &pids)
	} else {
		count
	};
	if st.waiting_count.swap(count, Ordering::Relaxed) != count {
		update_tray_icon(app);
	}
}

/// Check GitHub for a newer release; if the user agrees, download, install, and
/// restart. `silent` suppresses the "up to date" and check-failure dialogs so the
/// on-launch check stays quiet when nothing is new or the network is down — the
/// manual "Check for Updates…" tray item passes `silent = false`. An install
/// failure is always surfaced (the user explicitly clicked Install). Guarded by
/// `update_in_flight` so a launch check and a manual check can't overlap.
async fn check_for_updates(app: tauri::AppHandle, silent: bool) {
	use std::sync::atomic::Ordering;

	// Claim the in-flight slot; bail if a check is already running.
	{
		let st = app.state::<state::AppState>();
		if st
			.update_in_flight
			.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
			.is_err()
		{
			return;
		}
	}

	run_update_check(&app, silent).await;

	app.state::<state::AppState>()
		.update_in_flight
		.store(false, Ordering::Release);
}

/// Inner update flow, factored out so `check_for_updates` can always release the
/// `update_in_flight` guard regardless of which branch returns.
async fn run_update_check(app: &tauri::AppHandle, silent: bool) {
	use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
	use tauri_plugin_updater::UpdaterExt;

	let updater = match app.updater() {
		Ok(u) => u,
		Err(e) => {
			if !silent {
				app.dialog()
					.message(format!("Couldn't start the updater: {e}"))
					.title("Update Error")
					.kind(MessageDialogKind::Error)
					.blocking_show();
			}
			return;
		}
	};

	match updater.check().await {
		Ok(Some(update)) => {
			// Cache + broadcast so the popover can show its banner. The event may
			// fire before the (hidden) webview has mounted its listener, so the
			// cached copy is the source of truth the frontend pulls on mount.
			let info = model::UpdateInfo {
				version: update.version.clone(),
				current_version: update.current_version.clone(),
				notes: update.body.clone().unwrap_or_default(),
			};
			*app.state::<state::AppState>().pending_update.lock().unwrap() = Some(info.clone());
			let _ = app.emit("update_available", info);

			// Silent (launch + daily) checks defer entirely to the in-app banner.
			if silent {
				return;
			}
			// Manual check keeps a native confirm dialog for instant action. Version
			// only — native dialogs render multiline release notes badly; the banner
			// owns the changelog.
			let accepted = app
				.dialog()
				.message(format!(
					"Quay {} is available (you have {}).\n\nDownload and install it now?",
					update.version, update.current_version
				))
				.title("Update Available")
				.buttons(MessageDialogButtons::OkCancelCustom(
					"Install & Restart".into(),
					"Later".into(),
				))
				.blocking_show();
			if !accepted {
				return;
			}
			match update.download_and_install(|_, _| {}, || {}).await {
				Ok(_) => {
					*app.state::<state::AppState>().pending_update.lock().unwrap() = None;
					app.restart();
				}
				Err(e) => {
					app.dialog()
						.message(format!("The update failed to install: {e}"))
						.title("Update Error")
						.kind(MessageDialogKind::Error)
						.blocking_show();
				}
			}
		}
		Ok(None) => {
			// Remote no longer advertises a newer release — drop any stale banner.
			*app.state::<state::AppState>().pending_update.lock().unwrap() = None;
			if !silent {
				app.dialog()
					.message("You're running the latest version of Quay.")
					.title("No Updates")
					.blocking_show();
			}
		}
		Err(e) => {
			if !silent {
				app.dialog()
					.message(format!("Couldn't check for updates: {e}"))
					.title("Update Error")
					.kind(MessageDialogKind::Error)
					.blocking_show();
			}
		}
	}
}

/// The update the last check found, if any. The popover calls this on mount to
/// recover a pending update whose `update_available` event it may have missed (the
/// menubar app starts hidden, so the webview can mount after the launch check emits).
#[tauri::command]
fn get_pending_update(state: tauri::State<state::AppState>) -> Option<model::UpdateInfo> {
	state.pending_update.lock().unwrap().clone()
}

/// Download and install the pending update, then restart. Triggered by the banner's
/// Install button. Re-checks rather than caching the (non-`Send`) `Update` handle
/// across the IPC boundary — one extra round-trip on an explicit click. Returns an
/// error string the banner surfaces; on success the process restarts and never
/// returns. `Ok(())` with no restart means the remote is no longer newer (banner
/// should clear).
#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
	use std::sync::atomic::Ordering;
	use tauri_plugin_updater::UpdaterExt;

	// Share the `update_in_flight` guard with the check flows so an install can't
	// race a concurrent launch/daily/manual check.
	{
		let st = app.state::<state::AppState>();
		if st
			.update_in_flight
			.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
			.is_err()
		{
			return Err("An update check is already running. Try again in a moment.".into());
		}
	}

	let result = async {
		let updater = app
			.updater()
			.map_err(|e| format!("Couldn't start the updater: {e}"))?;
		match updater.check().await {
			Ok(Some(update)) => {
				update
					.download_and_install(|_, _| {}, || {})
					.await
					.map_err(|e| format!("The update failed to install: {e}"))?;
				*app.state::<state::AppState>().pending_update.lock().unwrap() = None;
				app.restart();
			}
			Ok(None) => {
				*app.state::<state::AppState>().pending_update.lock().unwrap() = None;
				Ok(())
			}
			Err(e) => Err(format!("Couldn't check for updates: {e}")),
		}
	}
	.await;

	app.state::<state::AppState>()
		.update_in_flight
		.store(false, Ordering::Release);
	result
}

/// Re-pin the popover under the tray icon.
///
/// The positioner plugin unwraps `current_monitor()`; a window macOS
/// considers off-screen (e.g. hidden/ordered-out — a late `resize_popover`
/// after hide-on-blur hits this) has no monitor, and moving it would panic
/// the main thread and kill the app. Skip the re-pin instead: the next
/// `toggle_popover` re-anchors before showing.
fn pin_under_tray(win: &tauri::WebviewWindow) {
	if matches!(win.current_monitor(), Ok(Some(_))) {
		let _ = tauri_plugin_positioner::WindowExt::move_window(
			win,
			tauri_plugin_positioner::Position::TrayCenter,
		);
	}
}

/// Toggle the popover window: show+focus if hidden, hide if visible.
fn toggle_popover(app: &tauri::AppHandle) {
	use std::sync::atomic::Ordering;
	if let Some(win) = app.get_webview_window("main") {
		if win.is_visible().unwrap_or(false) {
			// Mirror `visible` to the actual outcome of the window op.
			if win.hide().is_ok() {
				app.state::<state::AppState>().visible.store(false, Ordering::Relaxed);
			}
		} else {
			pin_under_tray(&win);
			if win.show().is_ok() {
				app.state::<state::AppState>().visible.store(true, Ordering::Relaxed);
				let _ = win.set_focus();
			}
		}
	}
}

/// Resize the popover to fit its content and re-pin it under the tray.
/// The frontend measures its own shell height (clamped there to the tray
/// monitor's usable height) and reports it; we floor at the 520 default and
/// backstop the ceiling. Re-anchoring with TrayCenter keeps the top edge fixed
/// under the tray so the window grows downward, whatever direction macOS's
/// `set_size` would otherwise pick.
#[tauri::command]
fn resize_popover(app: tauri::AppHandle, height: f64) {
	if let Some(win) = app.get_webview_window("main") {
		// ponytail: JS owns the real max (from the webview's screen); 1200 is a
		// sanity backstop, not the cap. Upgrade both to the tray monitor's
		// work-area if multi-monitor sizing ever matters.
		let h = height.clamp(520.0, 1200.0);
		let _ = win.set_size(tauri::LogicalSize::new(380.0, h));
		pin_under_tray(&win);
	}
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
	tauri::Builder::default()
		.plugin(tauri_plugin_dialog::init())
		.plugin(tauri_plugin_positioner::init())
		.plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
		.plugin(tauri_plugin_updater::Builder::new().build())
		.invoke_handler(tauri::generate_handler![
			commands::get_items,
			commands::add_item,
			commands::update_item,
			commands::delete_item,
			commands::reorder,
			commands::toggle_favorite,
			commands::detect_folder_cmd,
			commands::get_settings,
			commands::update_settings,
			commands::get_statuses,
			commands::start_item,
			commands::stop_item,
			commands::mark_stopped,
			commands::stop_all,
			commands::open_browser,
			commands::open_releases,
			commands::open_terminal,
			commands::get_terminals,
			commands::tail_log,
			commands::list_brew_formulae,
			commands::list_docker_images,
			commands::docker_daemon_running,
			commands::start_docker_daemon,
			commands::set_suppress_hide,
			commands::kill_discovered,
			commands::ignore_port,
			commands::kill_agent,
			commands::ignore_agent,
			commands::jump_to_session,
			commands::reveal_in_finder,
			commands::reveal_path,
			commands::get_hook_statuses,
			commands::install_agent_hooks,
			commands::uninstall_agent_hooks,
			resize_popover,
			get_pending_update,
			install_update,
		])
		.setup(|app| {
			// Menubar-only: hide the dock icon (and Cmd-Tab entry). Accessory keeps
			// the tray icon and lets the popover take focus when shown.
			#[cfg(target_os = "macos")]
			app.set_activation_policy(tauri::ActivationPolicy::Accessory);

			let dir = store::config_dir()?;
			app.manage(commands::init_state(dir));

			// Refresh an already-installed quay-hook helper if the bundled bytes
			// changed (i.e. the app updated). Never creates it unsolicited —
			// installing hooks is opt-in from Settings; this only keeps an
			// existing install current. Off the main thread: pure fs work.
			{
				let app_handle = app.handle().clone();
				std::thread::spawn(move || {
					let st = app_handle.state::<state::AppState>();
					if st.dir.join("bin/quay-hook").exists() {
						if let Some(src) = hooks_install::bundled_hook(&app_handle) {
							let _ = hooks_install::install_helper(&src, &st.dir);
						}
					}
				});
			}

			// Reattach to background services that outlived a previous app session
			// (e.g. a crash, force-quit, or relaunch after sleep). For each persisted
			// PID, adopt it only if the process is still alive AND — when a port is
			// configured — that PID is actually listening on it, which guards against
			// PID reuse pointing us at an unrelated process. Dead/mismatched entries
			// are dropped when we rewrite pids.json at the end.
			{
				let app_handle = app.handle().clone();
				let st = app_handle.state::<state::AppState>();
				let pids = store::load_pids(&st.dir);
				let items = st.config.lock().unwrap().items.clone();
				for (id, pid) in &pids {
					let Some(item) = items.iter().find(|i| &i.id == id) else { continue; };
					let alive = unsafe { libc::kill(*pid as i32, 0) == 0 };
					if !alive { continue; }
					let identity_ok = match item.port {
						Some(p) => supervisor::pids_listening(p).contains(pid),
						None => true, // portless: best-effort liveness only
					};
					if !identity_ok { continue; }
					st.running.lock().unwrap().insert(id.clone(), supervisor::adopt(*pid, st.log_path(id)));
					commands::set_status(&app_handle, id, model::Status::Running);
				}

				// Port sweep: for background items not already reattached above, probe
				// the configured port once and adopt any live listener — so services
				// started outside the app (or after pids.json was cleared on a clean
				// quit) show Running on launch without a manual Start. Background-only
				// (terminal/brew items must not enter the running map), and each port is
				// claimed once so two items sharing a port can't both adopt the same
				// listener.
				let mut claimed_ports = std::collections::HashSet::new();
				for item in &items {
					if !matches!(item.run_mode, model::RunMode::Background) { continue; }
					// Brew + Docker + Command items must not enter the running map: brew
					// is tracked via launchctl, Docker via `docker ps`, and Command is a
					// detached daemon tracked purely by its port (adopting it would give
					// it a phantom running-map entry Quay would then try to signal).
					if matches!(item.kind, model::ItemKind::Brew | model::ItemKind::Docker | model::ItemKind::Command) { continue; }
					let Some(p) = item.port else { continue; };
					let already = st.running.lock().unwrap().contains_key(&item.id);
					if already { claimed_ports.insert(p); continue; }
					if claimed_ports.contains(&p) { continue; }
					if commands::adopt_if_listening(&app_handle, item) {
						claimed_ports.insert(p);
					}
				}

				commands::persist_pids(&st);

				// Sweep stale terminal pid-capture files left by a mid-launch kill.
				if let Ok(entries) = std::fs::read_dir(st.dir.join("logs")) {
					for e in entries.flatten() {
						if e.file_name().to_string_lossy().ends_with(".term.pid") {
							let _ = std::fs::remove_file(e.path());
						}
					}
				}
			}

			// Start the background status-poll loop.
			health::spawn_poll_loop(app.handle().clone());

			// Start the metrics loop (only samples while the popover is visible).
			metrics::spawn_metrics_loop(app.handle().clone());

			// Start the port radar (only scans while the popover is visible).
			scanner::spawn_scan_loop(app.handle().clone());

			// Auto-start any items flagged with auto_start = true.
			{
				let app_handle = app.handle().clone();
				let auto: Vec<(String, bool)> = {
					let st = app_handle.state::<state::AppState>();
					let cfg = st.config.lock().unwrap();
					cfg.items.iter().filter(|i| i.auto_start)
						.map(|i| (i.id.clone(), matches!(i.kind, model::ItemKind::Docker)))
						.collect()
				};
				// If a Docker item wants to auto-start but the daemon is down, launch
				// Docker Desktop and wait once — there is no UI to prompt at launch.
				let needs_docker = auto.iter().any(|(_, is_docker)| *is_docker);
				if needs_docker && !docker::daemon_running() {
					if docker::start_daemon().is_ok() {
						docker::wait_for_daemon(std::time::Duration::from_secs(60));
					}
				}
				for (id, _) in auto {
					let _ = commands::start_item(app_handle.clone(), id);
				}
			}

			// Get app name and version dynamically
            let app_name = &app.package_info().name;
            let app_version = &app.package_info().version;
            let label_text = format!("{} v{}", app_name, app_version);

			// Build the static title item for the tray context menu and disable it
            let title_item = MenuItemBuilder::new(&label_text)
                .enabled(false) // 👈 This makes it static and unclickable!
                .build(app)?;

			// Build the tray context menu: a manual update check above Quit.
			let check_updates =
				MenuItemBuilder::with_id("check_updates", "Check for Updates…").build(app)?;
			let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
			let divider = PredefinedMenuItem::separator(app)?;

			let tray_menu = MenuBuilder::new(app).items(&[&title_item, &check_updates, &divider, &quit]).build()?;

			TrayIconBuilder::with_id("main")
				// Monochrome buoy glyph rendered as a macOS template image so it
				// auto-inverts (black/white) with the menubar's light/dark theme.
				// `update_tray_icon` swaps in colored (non-template) variants when
				// any service errors or is starting.
				.icon(tauri::include_image!("icons/tray.png"))
				.icon_as_template(true)
				.menu(&tray_menu)
				// Only show the context menu on right-click; left-click toggles the popover.
				.show_menu_on_left_click(false)
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
				.on_menu_event(|app, event| match event.id().as_ref() {
					"check_updates" => {
						// Manual check: surface "up to date" and errors (silent = false).
						tauri::async_runtime::spawn(check_for_updates(app.clone(), false));
					}
					"quit" => {
						// Stop background/brew/docker children but leave terminal windows
						// open; their PIDs are persisted so the next launch reattaches.
						let st = app.state::<state::AppState>();
						commands::shutdown_stop_non_terminal(&st);
						app.exit(0);
					}
					_ => {}
				})
				.build(app)?;

			// Reflect statuses set before the tray existed (reattach / auto-start
			// above): `set_status` only fires on change, so without this the beacon
			// would stay monochrome until the next actual transition.
			update_tray_icon(app.handle());

			// Silent update checks: one shortly after launch (delayed so the tray +
			// popover are settled), then daily for the app's lifetime. Silent, so a
			// found update surfaces only via the in-app banner. Clones the app handle
			// only; the thread ends with the process on quit.
			{
				let handle = app.handle().clone();
				std::thread::spawn(move || {
					std::thread::sleep(std::time::Duration::from_secs(3));
					tauri::async_runtime::spawn(check_for_updates(handle.clone(), true));
					loop {
						std::thread::sleep(std::time::Duration::from_secs(UPDATE_CHECK_INTERVAL_SECS));
						tauri::async_runtime::spawn(check_for_updates(handle.clone(), true));
					}
				});
			}

			Ok(())
		})
		.on_window_event(|window, event| {
			// Hide the popover when it loses focus (menubar-app behavior).
			// Gate on the "main" window only, and skip if a native dialog is open.
			if let WindowEvent::Focused(false) = event {
				if window.label() == "main" {
					let suppress = window
						.app_handle()
						.state::<state::AppState>()
						.suppress_hide
						.load(std::sync::atomic::Ordering::Relaxed);
					if !suppress {
						// Only clear `visible` when the popover is genuinely hidden —
						// during a native dialog (suppress_hide) it stays open, so the
						// metrics loop must keep sampling.
						if window.hide().is_ok() {
							window
								.app_handle()
								.state::<state::AppState>()
								.visible
								.store(false, std::sync::atomic::Ordering::Relaxed);
						}
					}
				}
			}
		})
		.build(tauri::generate_context!())
		.expect("error building app")
		.run(|app_handle, event| {
			if let tauri::RunEvent::ExitRequested { .. } = event {
				// Match the quit handler: stop non-terminal children, keep terminal
				// windows open, and persist their PIDs for reattach.
				let st = app_handle.state::<state::AppState>();
				commands::shutdown_stop_non_terminal(&st);
			}
		});
}
