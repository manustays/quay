use crate::model::{AppConfig, Status, UpdateInfo};
use crate::supervisor::Running;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize};

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
	/// The latest update the backend found, if any. Set when a check finds a newer
	/// release; the `update_available` event may be emitted before the popover
	/// webview has mounted its listener (menubar app starts hidden), so the frontend
	/// also pulls this on mount via `get_pending_update`.
	pub pending_update: Mutex<Option<UpdateInfo>>,
	/// Number of distinct waiting agent (agent, cwd) pairs, refreshed by the
	/// always-on poll loop (`health::spawn_poll_loop`) from the hook-state files so
	/// the menubar reflects waiting agents even while the popover is closed. Read by
	/// `update_tray_icon` to pick the waiting glyph and the title-badge count.
	pub waiting_count: AtomicUsize,
	/// Live PIDs the radar last saw per `(agent, cwd)`, stamped by `agent_radar::scan`
	/// (popover-open only). The always-on badge path (`waiting_count`) consults this to
	/// drop a waiting file whose every seen PID is now dead — a crashed-while-waiting
	/// session — without doing its own `ps`. A key absent here was never scanned, so it
	/// still counts (fallback). Cleared on restart; repopulated on the next scan.
	pub last_agent_pids: Mutex<HashMap<(String, String), HashSet<u32>>>,
}

impl AppState {
	/// Path of the item's log file — the single source of the log layout.
	pub fn log_path(&self, id: &str) -> PathBuf {
		self.dir.join("logs").join(format!("{id}.log"))
	}
}
