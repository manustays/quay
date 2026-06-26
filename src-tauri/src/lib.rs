pub mod brew;
pub mod commands;
pub mod detect;
pub mod health;
pub mod metrics;
pub mod model;
pub mod state;
pub mod store;
pub mod supervisor;
pub mod terminal;

use tauri::{
	Manager,
	menu::{MenuBuilder, MenuItemBuilder},
	tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
	WindowEvent,
};

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
			let _ = tauri_plugin_positioner::WindowExt::move_window(
				&win,
				tauri_plugin_positioner::Position::TrayCenter,
			);
			if win.show().is_ok() {
				app.state::<state::AppState>().visible.store(true, Ordering::Relaxed);
				let _ = win.set_focus();
			}
		}
	}
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
	tauri::Builder::default()
		.plugin(tauri_plugin_dialog::init())
		.plugin(tauri_plugin_positioner::init())
		.plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
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
			commands::stop_all,
			commands::open_browser,
			commands::open_terminal,
			commands::tail_log,
			commands::list_brew_formulae,
			commands::set_suppress_hide,
		])
		.setup(|app| {
			let dir = store::config_dir()?;
			app.manage(commands::init_state(dir));

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
					let log_path = st.dir.join("logs").join(format!("{id}.log"));
					st.running.lock().unwrap().insert(id.clone(), supervisor::adopt(*pid, log_path));
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
					if matches!(item.kind, model::ItemKind::Brew) { continue; }
					let Some(p) = item.port else { continue; };
					let already = st.running.lock().unwrap().contains_key(&item.id);
					if already { claimed_ports.insert(p); continue; }
					if claimed_ports.contains(&p) { continue; }
					if commands::adopt_if_listening(&app_handle, item) {
						claimed_ports.insert(p);
					}
				}

				commands::persist_pids(&st);
			}

			// Start the background status-poll loop.
			health::spawn_poll_loop(app.handle().clone());

			// Start the metrics loop (only samples while the popover is visible).
			metrics::spawn_metrics_loop(app.handle().clone());

			// Auto-start any items flagged with auto_start = true.
			{
				let app_handle = app.handle().clone();
				let ids: Vec<String> = {
					let st = app_handle.state::<state::AppState>();
					let cfg = st.config.lock().unwrap();
					cfg.items.iter().filter(|i| i.auto_start).map(|i| i.id.clone()).collect()
				};
				for id in ids {
					let _ = commands::start_item(app_handle.clone(), id);
				}
			}

			// Build a tray context menu with a single "Quit" item.
			let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
			let tray_menu = MenuBuilder::new(app).items(&[&quit]).build()?;

			TrayIconBuilder::new()
				.icon(app.default_window_icon().unwrap().clone())
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
				.on_menu_event(|app, event| {
					if event.id().as_ref() == "quit" {
						// Drain running children, drop the lock, then stop each one
						// so we don't hold the mutex across a blocking stop call.
						let children: Vec<_> = {
							let st = app.state::<state::AppState>();
							let mut map = st.running.lock().unwrap();
							map.drain().collect()
						};
						for (_, mut r) in children {
							let _ = supervisor::stop(&mut r);
						}
						// Children are stopped; clear pids.json so the next launch
						// doesn't try to reattach to processes we just killed.
						let dir = app.state::<state::AppState>().dir.clone();
						let _ = store::save_pids(&dir, &std::collections::HashMap::new());
						app.exit(0);
					}
				})
				.build(app)?;
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
				let st = app_handle.state::<state::AppState>();
				{
					let mut running = st.running.lock().unwrap();
					for (_, r) in running.iter_mut() {
						let _ = supervisor::stop(r);
					}
				}
				// Match the quit handler: clear persisted PIDs after stopping.
				let _ = store::save_pids(&st.dir, &std::collections::HashMap::new());
			}
		});
}
