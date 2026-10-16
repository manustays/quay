pub mod agent_radar;
pub mod brew;
pub mod commands;
pub mod detect;
pub mod docker;
pub mod health;
pub mod hooks_install;
#[cfg(target_os = "macos")]
pub mod mac_power;
pub mod metrics;
pub mod model;
pub mod scanner;
pub mod state;
pub mod store;
pub mod supervisor;
pub mod terminal;

use tauri::{
	Emitter, Manager,
	menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder},
	tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
	WindowEvent,
};

/// Write a one-line diagnostic to stderr.
///
/// Quay ships no log file and no logging backend, so this is deliberately the cheapest
/// thing that works: it is visible under `npm run tauri dev` or when the bundled binary is
/// launched from a terminal (`/Applications/Quay.app/Contents/MacOS/quay`). A Finder or
/// login-item launch discards stderr — these lines are for reproducing a fault, not for
/// after-the-fact forensics. Reserved for genuine failures; the happy path stays silent.
fn log_warn(context: &str, detail: impl std::fmt::Display) {
	eprintln!("[quay] {context}: {detail}");
}

/// Opt-in trace, for state that is invisible by construction.
///
/// The power gate parks the app precisely when nobody is looking at it, so there is
/// no way to watch it work without being told. Silent unless `QUAY_TRACE` is set,
/// which keeps [`log_warn`]'s "the happy path stays silent" rule intact.
fn log_trace(context: &str, detail: impl std::fmt::Display) {
	if std::env::var_os("QUAY_TRACE").is_some() {
		eprintln!("[quay] {context}: {detail}");
	}
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
struct TrayAnchor {
	native_point: (f64, f64),
}

#[cfg(target_os = "macos")]
#[derive(Default)]
struct TrayAnchorState(std::sync::Mutex<Option<TrayAnchor>>);

#[cfg(target_os = "macos")]
#[derive(Default)]
struct PopoverMonitorState(std::sync::Mutex<Option<(i32, i32)>>);

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
struct MacScreenGeometry {
	origin: (f64, f64),
	visible_origin: (f64, f64),
	visible_size: (f64, f64),
}

/// Whether an `NSEvent::mouseLocation` point lies on a screen frame (AppKit
/// coordinates, y up). The cursor's y spans `(origin.y, origin.y + height]`:
/// the top pixel row reports `y == maxY` — exactly where a menubar click lands
/// after flinging the cursor to the top edge — while `y == origin.y` belongs to
/// the screen stacked below. x spans `[origin.x, origin.x + width)` as usual.
#[cfg(any(target_os = "macos", test))]
fn cocoa_frame_contains(origin: (f64, f64), size: (f64, f64), point: (f64, f64)) -> bool {
	let (x, y) = point;
	x >= origin.0 && x < origin.0 + size.0 && y > origin.1 && y <= origin.1 + size.1
}

#[cfg(target_os = "macos")]
fn mac_screen_geometry(anchor: TrayAnchor) -> Option<MacScreenGeometry> {
	use objc2_app_kit::NSScreen;
	use objc2_foundation::MainThreadMarker;

	let Some(mtm) = MainThreadMarker::new() else {
		log_warn("popover placement skipped", "NSScreen is only readable on the main thread");
		return None;
	};
	NSScreen::screens(mtm).iter().find_map(|screen| {
		let frame = screen.frame();
		let contains = cocoa_frame_contains(
			(frame.origin.x, frame.origin.y),
			(frame.size.width, frame.size.height),
			anchor.native_point,
		);
		if !contains { return None; }
		let visible = screen.visibleFrame();
		Some(MacScreenGeometry {
			origin: (frame.origin.x, frame.origin.y),
			visible_origin: (visible.origin.x, visible.origin.y),
			visible_size: (visible.size.width, visible.size.height),
		})
	})
}

/// How often the background loop re-checks for updates after the initial launch
/// check (seconds). Daily. The `update_in_flight` guard keeps a manual check from
/// overlapping this one.
const UPDATE_CHECK_INTERVAL_SECS: u64 = 86_400;

/// How often the always-on badge path sweeps orphaned `waiting` hook-state files.
///
/// Only files with no recorded `(pid, startedAt)` — written by a helper from before
/// identity stamping — can reach the expensive branch, and only they need this at
/// all: a file that names its own process is checked with a syscall, and
/// `waiting_count` already refuses to count one whose process is gone, so the badge
/// is correct without any sweep. What the sweep does is tidy the files up.
///
/// Ten minutes, not one, because the legacy branch enumerates every tty-attached
/// process to decide liveness. Measured on a machine with ~185 of them and three
/// stale files: 1.3 % CPU and ~100 idle wakeups/min, against 0.11 % and 0.4 once
/// they were gone — a burst of thousands of wakeups once a minute, averaged out.
/// A stale legacy file can also be immortal: the legacy rule only deletes one whose
/// `(agent, cwd)` has no live session, so a live session in the same folder pins it,
/// and a *waiting* session emits no further event to re-stamp it with an identity
/// until someone answers it.
pub const PRUNE_INTERVAL_SECS: u64 = 600;

/// Set popover visibility everywhere it matters: the shared flag the gated loops
/// block on, and the frontend event that pauses the always-running CSS animations
/// while the window is hidden. The single writer — see [`state::AppState::set_visible`].
fn set_popover_visible(app: &tauri::AppHandle, vis: bool) {
	app.state::<state::AppState>().set_visible(vis);
	emit_render_active(app);
}

/// Tell the frontend whether to animate. The event carries the *derived* state —
/// popover open **and** someone able to see it — because a popover left open when
/// the display sleeps would otherwise keep compositing its `animate-pulse` dots at
/// a dark screen. The frontend only ever uses this to set `html[data-hidden]`, so
/// the derived value is what it wanted all along; nothing under `src/` changes.
///
/// Every writer of the wake state calls this: the popover setter above and the
/// screen/lock observers in [`mac_power`].
pub fn emit_render_active(app: &tauri::AppHandle) {
	let active = app.state::<state::AppState>().is_active();
	let _ = app.emit("popover_visibility", active);
}

/// Current render-active state, for the frontend to seed itself on mount — a reload
/// while hidden would otherwise miss the event and keep animating.
#[tauri::command]
fn get_popover_visible(state: tauri::State<state::AppState>) -> bool {
	state.is_active()
}

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
			// macOS workaround: tray-icon 0.24.1's set_title(None) is a no-op — it
			// never clears the NSStatusItem button title, so a stale count would
			// linger after the badge is turned off. Pass an empty string to clear.
			// (Harmless elsewhere: Windows ignores the title, GTK sets it as given.)
			let title = if title_badge && waiting > 0 { waiting.to_string() } else { String::new() };
			let _ = tray.set_title(Some(title));
		}
	});
}

/// Recompute the waiting-agent count from the hook-state files and refresh the tray.
/// Called from the always-on poll loop (`health::spawn_poll_loop`) so the menubar
/// reflects waiting agents even while the popover — and thus the heavy radar scan —
/// is closed. Cheap: a directory read of small JSON files plus a `kill(pid, 0)`
/// liveness check per waiting file (from the PIDs `scan` stamped), no `ps`/`sysinfo`.
///
/// `force_prune` controls the one *expensive* part, the orphan sweep (which forks
/// `ps`). The popover-open caller passes `true` — the user is looking, and `scan`
/// bails before stamping when no agent process exists, so that path is the only one
/// that clears a never-scanned phantom. The always-on caller passes `false` and is
/// rate-limited to [`PRUNE_INTERVAL_SECS`], because a 3 s `ps` fork forever is what
/// this costs otherwise.
pub fn refresh_waiting_badge(app: &tauri::AppHandle, force_prune: bool) {
	use std::sync::atomic::Ordering;
	let st = app.state::<state::AppState>();
	let (track_agents, ignored) = {
		let cfg = st.config.lock().unwrap();
		(cfg.settings.track_agents, cfg.settings.ignored_agents.clone())
	};
	// Tracking off: force the badge to zero rather than skipping the refresh, so a
	// count left over from before the toggle clears instead of sticking in the tray.
	// This is also the hook-event path (`health.rs`), which keeps firing regardless.
	if !track_agents {
		if st.waiting_count.swap(0, std::sync::atomic::Ordering::Relaxed) != 0 {
			update_tray_icon(app);
		}
		return;
	}
	let dir = st.dir.join("agent-state");
	let count = agent_radar::waiting_count(&dir, &ignored);
	// A phantom count — a `waiting` file whose session died without a clearing
	// hook — otherwise self-heals only on a popover scan. When the badge would
	// show, sweep orphaned waiting files (crashed/exited: no live process, event
	// older than the grace) so it clears without one. The process enumeration runs
	// only in this `count > 0` branch *and* only when the rate limit allows, since
	// "an agent is waiting on you" is a steady state, not a rare one.
	let count = if count > 0 && (force_prune || claim_prune_slot(&st)) {
		// Passed unevaluated: every state file written by a current helper names its
		// own process, so the `ps` fork behind this only happens if a file from an
		// older helper is still around.
		agent_radar::prune_orphan_hook_states(
			&dir,
			|| agent_radar::live_agent_keys(&ignored),
			std::time::SystemTime::now(),
		);
		agent_radar::waiting_count(&dir, &ignored)
	} else {
		count
	};
	if st.waiting_count.swap(count, Ordering::Relaxed) != count {
		update_tray_icon(app);
	}
}

/// Reserve the next orphan-sweep slot: true at most once per [`PRUNE_INTERVAL_SECS`].
/// The new instant is written *before* the caller does the work and while the lock is
/// held, so two loops can't both decide it's their turn. `Instant` (not wall clock) so
/// a clock rollback can't stall the sweep forever or make it fire every tick.
fn claim_prune_slot(st: &state::AppState) -> bool {
	let mut last = st.last_prune.lock().unwrap();
	if last.elapsed() < std::time::Duration::from_secs(PRUNE_INTERVAL_SECS) {
		return false;
	}
	*last = std::time::Instant::now();
	true
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

/// Re-pin the popover under the latest clicked tray icon.
///
/// On macOS, keep the complete placement path in AppKit coordinates. Tauri's
/// reported physical monitor sizes and origins use inconsistent scaling on
/// mixed-DPI desktops, so translating its tray rectangle back into AppKit can
/// select the wrong display or strand the window off-screen. The mouse is over
/// the status item when the click arrives, making its native location a stable
/// anchor in the same coordinate space as `NSScreen` and `NSWindow`.
fn pin_under_tray(app: &tauri::AppHandle, win: &tauri::WebviewWindow) {
	#[cfg(not(target_os = "macos"))]
	if matches!(win.current_monitor(), Ok(Some(_))) {
		let _ = tauri_plugin_positioner::WindowExt::move_window(
			win,
			tauri_plugin_positioner::Position::TrayCenter,
		);
	}

	#[cfg(target_os = "macos")]
	unsafe {
		use objc2_app_kit::NSWindow;
		use objc2_foundation::NSPoint;

		// Each of these bails without placing the window, which also leaves
		// `PopoverMonitorState` stale and skews the next `tray_is_on_current_monitor`
		// check — worth a line rather than a silent return.
		let Some(anchor) = *app.state::<TrayAnchorState>().0.lock().unwrap() else {
			log_warn("popover placement skipped", "no tray anchor recorded yet");
			return;
		};
		let Some(screen) = mac_screen_geometry(anchor) else {
			log_warn(
				"popover placement skipped",
				format!("no screen contains anchor {:?}", anchor.native_point),
			);
			return;
		};
		let (mouse_x, _) = anchor.native_point;
		let popup_width = 380.0;
		let ideal_x = mouse_x - popup_width / 2.0;
		let x = ideal_x.clamp(
			screen.visible_origin.0,
			(screen.visible_origin.0 + screen.visible_size.0 - popup_width)
				.max(screen.visible_origin.0),
		);
		let cocoa_y = screen.visible_origin.1 + screen.visible_size.1;
		let Ok(ns_window) = win.ns_window() else {
			log_warn("popover placement skipped", "window has no NSWindow handle");
			return;
		};
		let ns_window: &NSWindow = &*ns_window.cast();
		ns_window.setFrameTopLeftPoint(NSPoint::new(x, cocoa_y));
		*app.state::<PopoverMonitorState>().0.lock().unwrap() =
			Some((screen.origin.0.round() as i32, screen.origin.1.round() as i32));
	}
}

#[cfg(target_os = "macos")]
fn tray_is_on_current_monitor(app: &tauri::AppHandle) -> bool {
	let Some(anchor) = *app.state::<TrayAnchorState>().0.lock().unwrap() else { return true; };
	let target_position = match mac_screen_geometry(anchor) {
		Some(screen) => (screen.origin.0.round() as i32, screen.origin.1.round() as i32),
		None => return false,
	};
	app.state::<PopoverMonitorState>()
		.0
		.lock()
		.unwrap()
		.as_ref()
		.is_some_and(|current| current == &target_position)
}

/// When hide-on-blur last closed the popover, so a tray click that arrives just
/// afterwards can tell it already did the closing. See [`press_closes_popover`].
#[cfg(target_os = "macos")]
static LAST_BLUR_HIDE: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Set on the press of a tray click that the popover's hide-on-blur already answered;
/// read again on the matching release, which is where the toggle happens.
#[cfg(target_os = "macos")]
static PRESS_CLOSED_POPOVER: std::sync::atomic::AtomicBool =
	std::sync::atomic::AtomicBool::new(false);

/// How stale a blur-hide may be and still count as caused by the click being handled.
/// Measured gap on macOS 27 is ~80 ms; this leaves a wide margin without being long
/// enough to swallow a deliberate second click.
#[cfg(target_os = "macos")]
const BLUR_CLICK_GRACE: std::time::Duration = std::time::Duration::from_millis(250);

/// Record that hide-on-blur, not the user, closed the popover just now.
#[cfg(target_os = "macos")]
fn note_blur_hide() {
	*LAST_BLUR_HIDE.lock().unwrap() = Some(std::time::Instant::now());
}

/// Decide, on the press half of a tray click, whether this click has *already* closed the
/// popover via hide-on-blur.
///
/// macOS 27 gives the status-item button key focus on mouse-down, so the popover resigns
/// key and the blur handler hides it ~80 ms *before* `TrayIconEvent::Click` reaches us. By
/// the time the release is handled the window looks hidden, so a naive toggle reopens it —
/// the user sees a flicker instead of a close. The verdict is latched here, on the press,
/// while the blur is still fresh, because a press held longer than the grace window would
/// otherwise age out before its release arrived.
#[cfg(target_os = "macos")]
fn press_closes_popover() {
	let closed = LAST_BLUR_HIDE
		.lock()
		.unwrap()
		.take()
		.is_some_and(|at| at.elapsed() < BLUR_CLICK_GRACE);
	PRESS_CLOSED_POPOVER.store(closed, std::sync::atomic::Ordering::Relaxed);
}

/// Take the verdict latched by [`press_closes_popover`], clearing it.
#[cfg(target_os = "macos")]
fn take_press_closed_popover() -> bool {
	PRESS_CLOSED_POPOVER.swap(false, std::sync::atomic::Ordering::Relaxed)
}

fn show_popover(app: &tauri::AppHandle, win: &tauri::WebviewWindow) {
	// Mirror `visible` to the actual outcome of the window op.
	match win.show() {
		Ok(()) => {
			// A click is not evidence about the display, but it is a good moment to
			// re-read it: if a screen/lock notification was ever missed, this is what
			// stops a stale flag wedging the always-on loop parked forever.
			#[cfg(target_os = "macos")]
			mac_power::reconcile(app);
			set_popover_visible(app, true);
			if let Err(e) = win.set_focus() {
				log_warn("popover focus failed", e);
			}
		}
		Err(e) => log_warn("popover show failed", e),
	}
}

fn hide_popover(app: &tauri::AppHandle, win: &tauri::WebviewWindow) {
	match win.hide() {
		Ok(()) => set_popover_visible(app, false),
		Err(e) => log_warn("popover hide failed", e),
	}
}

/// Toggle the popover window: show+focus if hidden, hide if visible.
///
/// `already_closed` reports that hide-on-blur beat this click to the punch (see
/// [`press_closes_popover`]), so the popover counts as open even though the window is
/// hidden — otherwise the click reopens what it was meant to close.
fn toggle_popover(app: &tauri::AppHandle, already_closed: bool) {
	let Some(win) = app.get_webview_window("main") else {
		log_warn("popover toggle failed", "no window labelled \"main\"");
		return;
	};
	if already_closed || win.is_visible().unwrap_or(false) {
		#[cfg(target_os = "macos")]
		if !tray_is_on_current_monitor(app) {
			// A click on another display moves the open popover there instead of
			// consuming the click as a close, so each menubar icon feels local.
			// `show_popover` because hide-on-blur may already have closed it.
			pin_under_tray(app, &win);
			show_popover(app, &win);
			return;
		}
		hide_popover(app, &win);
	} else {
		pin_under_tray(app, &win);
		show_popover(app, &win);
	}
}

/// Present the tray context menu, attaching the `NSMenu` only while it is on screen.
///
/// macOS 27 stopped forwarding status-item clicks to `tray-icon`'s tracking view whenever
/// an `NSMenu` is attached to the `NSStatusItem` — AppKit's menu tracking swallows them —
/// so leaving the menu attached kills the left-click → popover path outright. Attaching
/// on demand keeps both gestures. This mirrors the upstream fix (tauri-apps/tray-icon#365),
/// which Tauri cannot pull in yet: it shipped in tray-icon 0.25.1 while tauri 2.11 still
/// requires `^0.24`. Once Tauri depends on tray-icon >= 0.25.1, delete this function and
/// its event arm and restore the plain `.menu(&tray_menu)` on the builder.
///
/// Runs inline on purpose. Tray events are delivered on the main thread, and Tauri's
/// main-thread dispatch calls straight through when it is already there, so `show_menu`
/// (an `NSStatusBarButton::performClick`) runs AppKit's nested menu loop and returns only
/// once the menu has been dismissed — exactly when it should be detached again.
#[cfg(target_os = "macos")]
fn show_tray_menu(tray: &tauri::tray::TrayIcon, menu: &tauri::menu::Menu<tauri::Wry>) {
	if let Err(e) = tray.set_menu(Some(menu.clone())) {
		log_warn("tray menu attach failed", e);
		return;
	}
	if let Err(e) = tray.with_inner_tray_icon(|inner| inner.show_menu()) {
		log_warn("tray menu present failed", e);
	}
	// Detach unconditionally: a menu left attached is precisely what breaks clicks.
	if let Err(e) = tray.set_menu(None::<tauri::menu::Menu<tauri::Wry>>) {
		log_warn("tray menu detach failed", e);
	}
}

/// Resize the popover to fit its content and re-pin it under the tray.
/// The frontend measures its own shell height (clamped there to the tray
/// monitor's usable height) and reports it; we floor at the 520 default and
/// backstop the ceiling. Re-anchoring keeps the top edge fixed under the tray
/// so the window grows downward, whatever direction macOS's `set_size` would
/// otherwise pick.
#[tauri::command]
fn resize_popover(app: tauri::AppHandle, height: f64) {
	if let Some(win) = app.get_webview_window("main") {
		// ponytail: JS owns the real max (from the webview's screen); 1200 is a
		// sanity backstop, not the cap. Upgrade both to the tray monitor's
		// work-area if multi-monitor sizing ever matters.
		let h = height.clamp(520.0, 1200.0);
		let _ = win.set_size(tauri::LogicalSize::new(380.0, h));
		pin_under_tray(&app, &win);
	}
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
	tauri::Builder::default()
		// Replace Tauri's default macOS app menu: its "Quit Quay" ⌘Q fires whenever the
		// popover is key, so a stray ⌘Q meant for another app quit Quay. Keep only the
		// Edit items — WebKit text fields get ⌘C/⌘V/⌘X/⌘Z/⌘A through them. The menu is
		// never shown (Accessory policy), so one untitled-looking submenu is enough.
		.menu(|app| {
			let edit = SubmenuBuilder::new(app, "Edit")
				.undo()
				.redo()
				.separator()
				.cut()
				.copy()
				.paste()
				.select_all()
				.build()?;
			MenuBuilder::new(app).item(&edit).build()
		})
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
			commands::open_homepage,
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
			get_popover_visible,
			install_update,
		])
		.setup(|app| {
			// Menubar-only: hide the dock icon (and Cmd-Tab entry). Accessory keeps
			// the tray icon and lets the popover take focus when shown.
			#[cfg(target_os = "macos")]
			app.set_activation_policy(tauri::ActivationPolicy::Accessory);

			let dir = store::config_dir()?;
			app.manage(commands::init_state(dir));
			#[cfg(target_os = "macos")]
			app.manage(TrayAnchorState::default());
			#[cfg(target_os = "macos")]
			app.manage(PopoverMonitorState::default());

			// Refresh an already-installed quay-hook helper if the bundled bytes
			// changed (i.e. the app updated). Never creates it unsolicited —
			// installing hooks is opt-in from Settings; this only keeps an
			// existing install current. Off the main thread: pure fs work.
			{
				let app_handle = app.handle().clone();
				std::thread::spawn(move || {
					let st = app_handle.state::<state::AppState>();
					if !st.dir.join("bin/quay-hook").exists() {
						return;
					}
					if let Some(src) = hooks_install::bundled_hook(&app_handle) {
						let _ = hooks_install::install_helper(&src, &st.dir);
					}
					// The configs themselves also go stale: a corrected event matcher or
					// a fixed plugin would otherwise only reach someone who toggled the
					// hook off and on in Settings. Only refreshes agents already opted in.
					let Some(home) = dirs::home_dir() else { return };
					let refreshed = hooks_install::refresh_installed(&home, &st.dir);
					if !refreshed.is_empty() {
						log_trace("hooks refreshed", refreshed.join(", "));
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
			// Before the loops start, so none of them polls at a screen that is
			// already asleep (a relaunch into a locked Mac, say).
			#[cfg(target_os = "macos")]
			mac_power::spawn_observers(app.handle());
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

			// macOS 27 stops delivering status-item clicks to tray-icon's tracking view
			// while an NSMenu is attached, so on macOS the menu is attached only for as
			// long as it is on screen — see `show_tray_menu`. Every other platform keeps
			// it attached for the whole session as before.
			#[cfg(target_os = "macos")]
			let menu_for_right_click = tray_menu.clone();

			#[allow(unused_mut)]
			let mut tray = TrayIconBuilder::with_id("main")
				// Monochrome buoy glyph rendered as a macOS template image so it
				// auto-inverts (black/white) with the menubar's light/dark theme.
				// `update_tray_icon` swaps in colored (non-template) variants when
				// any service errors or is starting.
				.icon(tauri::include_image!("icons/tray.png"))
				.icon_as_template(true)
				// Only show the context menu on right-click; left-click toggles the popover.
				.show_menu_on_left_click(false);

			#[cfg(not(target_os = "macos"))]
			{
				tray = tray.menu(&tray_menu);
			}

			tray
				.on_tray_icon_event(move |tray, event| {
					tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
					match event {
						TrayIconEvent::Click {
							button: MouseButton::Left,
							button_state: MouseButtonState::Up,
							..
						} => {
							#[cfg(target_os = "macos")]
							{
								let point = objc2_app_kit::NSEvent::mouseLocation();
								*tray.app_handle().state::<TrayAnchorState>().0.lock().unwrap() =
									Some(TrayAnchor { native_point: (point.x, point.y) });
							}
							#[cfg(target_os = "macos")]
							let already_closed = take_press_closed_popover();
							#[cfg(not(target_os = "macos"))]
							let already_closed = false;
							toggle_popover(tray.app_handle(), already_closed);
						}
						// The press half only records whether this same click has already
						// closed the popover through hide-on-blur; the release toggles.
						#[cfg(target_os = "macos")]
						TrayIconEvent::Click {
							button: MouseButton::Left,
							button_state: MouseButtonState::Down,
							..
						} => press_closes_popover(),
						// On press, not release: that is when every other macOS menubar
						// item opens its menu, and it keeps press-drag-release selection.
						#[cfg(target_os = "macos")]
						TrayIconEvent::Click {
							button: MouseButton::Right,
							button_state: MouseButtonState::Down,
							..
						} => show_tray_menu(tray, &menu_for_right_click),
						_ => {}
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
							set_popover_visible(window.app_handle(), false);
							// A tray click arriving right after this is the click that
							// caused it, and must not reopen what it just closed.
							#[cfg(target_os = "macos")]
							note_blur_hide();
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

#[cfg(test)]
mod tests {
	use super::cocoa_frame_contains;

	#[test]
	fn cocoa_frame_contains_owns_top_edge_not_bottom() {
		// Lower 1512×982 screen at the origin; upper screen stacked directly above.
		let lower = ((0.0, 0.0), (1512.0, 982.0));
		let upper = ((0.0, 982.0), (3440.0, 1440.0));
		// Top pixel row of the lower screen reports y == maxY: lower owns it.
		assert!(cocoa_frame_contains(lower.0, lower.1, (400.0, 982.0)));
		assert!(!cocoa_frame_contains(upper.0, upper.1, (400.0, 982.0)));
		// Top edge of the upper (outermost) screen still lands on it.
		assert!(cocoa_frame_contains(upper.0, upper.1, (400.0, 2422.0)));
		// Horizontal bounds stay half-open.
		assert!(cocoa_frame_contains(lower.0, lower.1, (0.0, 500.0)));
		assert!(!cocoa_frame_contains(lower.0, lower.1, (1512.0, 500.0)));
	}
}
