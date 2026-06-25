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
