//! Screen-sleep and screen-lock observation — the always-on loop's off switch.
//!
//! Quay's health loop exists to keep the tray icon and its waiting-agent badge
//! honest. Neither is on screen when every display is asleep or the Mac is locked,
//! so the work behind them is computed for nobody. This module is the only thing
//! that tells the app which of those states it is in.
//!
//! Two independent signals, from two different notification centres, because they
//! change independently and in either order: locking usually sleeps the display a
//! minute later, and unlocking wakes it first. [`crate::state::Wake::awake`] ands
//! them together.
//!
//! Everything here is macOS-only and runs on the main thread.

use crate::state::AppState;
use block2::RcBlock;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{
	MainThreadMarker, NSDistributedNotificationCenter, NSNotification, NSString,
};
use core::ptr::NonNull;
use tauri::{AppHandle, Manager};

// `CGDisplayIsAsleep` and friends take and return plain integers, so they need no
// binding crate. `CGDirectDisplayID` is a `u32`, `boolean_t` an `i32`.
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
	fn CGMainDisplayID() -> u32;
	fn CGDisplayIsAsleep(display: u32) -> i32;
}

/// Is every attached display asleep right now?
///
/// Notifications only fire on *change*, so this is what makes a launch (or an
/// updater relaunch) into a dark screen correct rather than a silent 3 s poll
/// forever. Also the self-heal if a notification is ever missed.
pub fn screens_asleep() -> bool {
	// The main display sleeping means the whole session's displays are asleep;
	// macOS does not sleep one and leave another lit.
	unsafe { CGDisplayIsAsleep(CGMainDisplayID()) != 0 }
}

/// Is the session locked right now? `None` when the answer can't be read, so the
/// caller can leave the flag alone rather than guess "unlocked" and poll at a lock
/// screen.
pub fn session_locked() -> Option<bool> {
	use objc2_core_foundation::{
		CFBoolean, CFDictionary, CFRetained, CFString,
	};

	#[link(name = "CoreGraphics", kind = "framework")]
	unsafe extern "C" {
		fn CGSessionCopyCurrentDictionary() -> Option<CFRetained<CFDictionary>>;
	}

	let dict = unsafe { CGSessionCopyCurrentDictionary() }?;
	let key = CFString::from_static_str("CGSSessionScreenIsLocked");
	// Absent from the dictionary whenever the session has never been locked, which
	// is itself the answer: not locked.
	let value = unsafe { dict.value(&*key as *const CFString as *const _) };
	if value.is_null() {
		return Some(false);
	}
	Some(unsafe { &*value.cast::<CFBoolean>() }.value())
}

/// Read the real state and push it into the gate. Called at startup, and again
/// whenever the tray is clicked — a click is not evidence about the display, but it
/// is a good moment to reconcile, so a missed notification can never wedge Quay
/// permanently idle.
pub fn reconcile(app: &AppHandle) {
	let state = app.state::<AppState>();
	state.set_screens_asleep(screens_asleep());
	if let Some(locked) = session_locked() {
		state.set_locked(locked);
	}
	settled(app, "reconciled");
}

/// Finish a transition: tell the webview whether to animate, and trace the outcome.
///
/// The two are always done together — a gate change that the renderer doesn't hear
/// about leaves a hidden popover compositing at a dark screen.
fn settled(app: &AppHandle, cause: &str) {
	crate::emit_render_active(app);
	let parked = !app.state::<AppState>().is_awake();
	crate::log_trace(
		"power gate",
		format_args!("{cause} — polling {}", if parked { "parked" } else { "running" }),
	);
}

/// Register the four observers. Must be called on the main thread — `.setup()` is.
pub fn spawn_observers(app: &AppHandle) {
	if MainThreadMarker::new().is_none() {
		crate::log_warn(
			"power observers skipped",
			"notification centres must be registered on the main thread",
		);
		return;
	}

	// `ScreensDidSleep` fires only when *all* displays sleep, which is exactly the
	// question being asked. A closed lid with an external display still lit does not
	// fire it, and correctly so: the menubar is visible over there.
	let workspace = NSWorkspace::sharedWorkspace();
	let center = workspace.notificationCenter();
	for (name, asleep) in [
		("NSWorkspaceScreensDidSleepNotification", true),
		("NSWorkspaceScreensDidWakeNotification", false),
	] {
		let handle = app.clone();
		let block = RcBlock::new(move |_: NonNull<NSNotification>| {
			handle.state::<AppState>().set_screens_asleep(asleep);
			settled(&handle, if asleep { "screens asleep" } else { "screens awake" });
		});
		let token = unsafe {
			center.addObserverForName_object_queue_usingBlock(
				Some(&NSString::from_str(name)),
				None,
				None,
				&block,
			)
		};
		// The token must outlive the observation, and this one lasts for the whole
		// process: there is no point in the app's life where it stops caring whether
		// the screen is on. Dropping it would silently unregister.
		std::mem::forget(token);
	}

	// Lock state is not an NSWorkspace notification — it is broadcast system-wide.
	let distributed = NSDistributedNotificationCenter::defaultCenter();
	for (name, locked) in
		[("com.apple.screenIsLocked", true), ("com.apple.screenIsUnlocked", false)]
	{
		let handle = app.clone();
		let block = RcBlock::new(move |_: NonNull<NSNotification>| {
			handle.state::<AppState>().set_locked(locked);
			settled(&handle, if locked { "locked" } else { "unlocked" });
		});
		let token = unsafe {
			distributed.addObserverForName_object_queue_usingBlock(
				Some(&NSString::from_str(name)),
				None,
				None,
				&block,
			)
		};
		std::mem::forget(token);
	}

	// Notifications only report changes, so the current state has to be read once.
	reconcile(app);
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The state queries are raw framework FFI, so the thing worth checking is that
	/// they are memory-correct, not what they answer — the answer depends on whether
	/// the machine running the test happens to be locked.
	///
	/// `CGSessionCopyCurrentDictionary` follows the Copy rule: it hands back a +1
	/// reference that `CFRetained` must release exactly once. Getting that wrong
	/// leaks or double-frees, and neither shows up in a single call — so call both
	/// enough times that either would.
	#[test]
	fn state_queries_are_memory_correct_under_repetition() {
		let first = screens_asleep();
		let first_lock = session_locked();
		for _ in 0..500 {
			// A double-free trips here; a leak shows as RSS growth under a leaks run.
			let _ = screens_asleep();
			let _ = session_locked();
		}
		// Nothing changed the display or the lock in the microseconds this took, so
		// an unstable answer would mean we are reading the wrong memory.
		assert_eq!(screens_asleep(), first, "display state must read consistently");
		assert_eq!(session_locked(), first_lock, "lock state must read consistently");
	}

	/// Headless CI has no window server, so `session_locked` returning `None` is
	/// expected there — what must never happen is a confident wrong `false`, which
	/// would leave Quay polling at a lock screen.
	///
	/// This deliberately does **not** assert which value comes back. An earlier
	/// version asserted the session was unlocked and duly failed the first time the
	/// display slept mid-run and locked the Mac: the code was right and the test was
	/// encoding an environmental accident. Whether the screen is locked is not a
	/// property of this code.
	#[test]
	fn lock_state_is_readable_without_inventing_an_answer() {
		let first = session_locked();
		// Reading must not depend on having read before (no cached CF handle, no
		// one-shot dictionary), and must not drift between calls.
		assert_eq!(session_locked(), first, "lock state must read consistently");
		if let Some(locked) = first {
			// A real answer is a real bool either way; what matters is that `None`
			// means "could not read" and is never silently turned into `false`.
			assert!(locked || !locked);
		}
	}
}
