pub mod brew;
pub mod detect;
pub mod health;
pub mod model;
pub mod store;
pub mod supervisor;

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
