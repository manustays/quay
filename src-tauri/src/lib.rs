pub mod brew;
pub mod commands;
pub mod detect;
pub mod health;
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

			// Start the background status-poll loop.
			health::spawn_poll_loop(app.handle().clone());

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
						let _ = window.hide();
					}
				}
			}
		})
		.build(tauri::generate_context!())
		.expect("error building app")
		.run(|app_handle, event| {
			if let tauri::RunEvent::ExitRequested { .. } = event {
				let st = app_handle.state::<state::AppState>();
				let mut running = st.running.lock().unwrap();
				for (_, r) in running.iter_mut() {
					let _ = supervisor::stop(r);
				}
			}
		});
}
