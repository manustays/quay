use crate::model::{AppConfig, Status};
use crate::supervisor::Running;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

/// All shared, mutable app state behind locks.
pub struct AppState {
	pub dir: PathBuf,
	pub config: Mutex<AppConfig>,
	pub running: Mutex<HashMap<String, Running>>,
	pub statuses: Mutex<HashMap<String, Status>>,
	pub errors: Mutex<HashMap<String, String>>,
	/// When `true`, the `Focused(false)` window-event handler skips hiding the
	/// popover. Set while a native dialog (e.g. folder picker) is open.
	pub suppress_hide: AtomicBool,
	/// `true` while the popover is actually shown. Gates the metrics loop so it
	/// does no sampling work while the popover is hidden. Set true on show,
	/// false only when the window is genuinely hidden (see `lib.rs`).
	pub visible: AtomicBool,
	/// `true` while an updater check/download/install is running. Guards the
	/// launch check and the "Check for Updates…" tray item against overlapping
	/// runs (duplicate dialogs, racing downloads). See `lib.rs::check_for_updates`.
	pub update_in_flight: AtomicBool,
}
