pub mod brew;
pub mod commands;
pub mod detect;
pub mod docker;
pub mod health;
pub mod metrics;
pub mod model;
pub mod scanner;
pub mod state;
pub mod store;
pub mod supervisor;
pub mod terminal;

use tauri::{
	Manager,
	menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem},
	tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
	WindowEvent,
};

/// Reflect the aggregate service status on the tray icon: the buoy's beacon glows
/// red (any error) or amber (any starting) via colored non-template variants;
/// otherwise the monochrome template icon (theme-adaptive) is restored.
///
/// Safe to call from any thread (e.g. the health-poll loop): tray mutation is
/// dispatched to the main thread, and the aggregate is computed inside the closure
/// so the icon reflects the statuses at apply time, not at call time.
pub fn update_tray_icon(app: &tauri::AppHandle) {
	let app = app.clone();
	let _ = app.clone().run_on_main_thread(move || {
		let aggregate = {
			let st = app.state::<state::AppState>();
			let statuses = st.statuses.lock().unwrap();
			health::aggregate_status(statuses.values().copied())
		};
		let (icon, is_template) = match aggregate {
			Some(model::Status::Error) => (tauri::include_image!("icons/tray-error.png"), false),
			Some(_) => (tauri::include_image!("icons/tray-starting.png"), false),
			None => (tauri::include_image!("icons/tray.png"), true),
		};
		if let Some(tray) = app.tray_by_id("main") {
			let _ = tray.set_icon_with_as_template(Some(icon), is_template);
		}
	});
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
				Ok(_) => app.restart(),
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
			commands::stop_all,
			commands::open_browser,
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
		])
		.setup(|app| {
			// Menubar-only: hide the dock icon (and Cmd-Tab entry). Accessory keeps
			// the tray icon and lets the popover take focus when shown.
			#[cfg(target_os = "macos")]
			app.set_activation_policy(tauri::ActivationPolicy::Accessory);

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
					// Brew + Docker items must not enter the running map: brew is tracked
					// via launchctl, Docker via `docker ps` (containers have no host PID,
					// and a published port maps through docker-proxy, not the container).
					if matches!(item.kind, model::ItemKind::Brew | model::ItemKind::Docker) { continue; }
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

			// Silent update check shortly after launch — delayed a few seconds so the
			// tray + popover are settled before any "update available" dialog appears.
			{
				let handle = app.handle().clone();
				std::thread::spawn(move || {
					std::thread::sleep(std::time::Duration::from_secs(3));
					tauri::async_runtime::spawn(check_for_updates(handle, true));
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
