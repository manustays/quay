use crate::model::{AppConfig, Status, UpdateInfo};
use crate::supervisor::Running;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Popover-visibility signal for the two visibility-gated loops (metrics, radar).
///
/// `generation` bumps on every real change, so a loop that was busy collecting can
/// tell that a hide→show happened while it worked: a `notify_all` it wasn't waiting
/// for is otherwise lost, and it would sleep out its whole interval before noticing.
#[derive(Clone, Copy, Default)]
pub struct Wake {
	pub visible: bool,
	pub generation: u64,
}

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
	/// Mirrors `visible` under a lock, so the gated loops can block on a condvar
	/// instead of idle-ticking. Writers must go through [`AppState::set_visible`].
	pub wake: (Mutex<Wake>, Condvar),
	/// When the orphaned-waiting-file sweep last ran. Monotonic on purpose — a
	/// wall-clock rollback must not make the rate limit fire every tick (or never).
	pub last_prune: Mutex<Instant>,
}

impl AppState {
	/// Path of the item's log file — the single source of the log layout.
	pub fn log_path(&self, id: &str) -> PathBuf {
		self.dir.join("logs").join(format!("{id}.log"))
	}

	/// Set popover visibility, waking the gated loops. The atomic is stored *under*
	/// the lock so a show landing between a loop's check and its wait can't be lost.
	/// The generation only moves on a real change, so a repeated `false` (hide, then
	/// hide-on-blur) doesn't cost a redundant scan.
	pub fn set_visible(&self, vis: bool) {
		let (lock, cv) = &self.wake;
		let mut w = lock.lock().unwrap();
		self.visible.store(vis, Ordering::Relaxed);
		if w.visible != vis {
			w.visible = vis;
			w.generation = w.generation.wrapping_add(1);
			cv.notify_all();
		}
	}

	/// Block until the popover is visible; returns the generation observed. A hidden
	/// popover costs zero wakeups (no idle tick), and a show is picked up immediately.
	pub fn wait_visible(&self) -> u64 {
		let (lock, cv) = &self.wake;
		let mut w = lock.lock().unwrap();
		while !w.visible {
			w = cv.wait(w).unwrap();
		}
		w.generation
	}

	/// Sleep up to `dur`, returning early if visibility changed since `generation`.
	/// Checking the generation *under the lock* covers a hide/show that happened
	/// while the caller was collecting, which a bare `sleep` would sit through.
	pub fn wait_interval(&self, generation: u64, dur: Duration) {
		let (lock, cv) = &self.wake;
		let mut w = lock.lock().unwrap();
		let deadline = Instant::now() + dur;
		while w.visible && w.generation == generation {
			let Some(left) = deadline.checked_duration_since(Instant::now()) else { break };
			if left.is_zero() {
				break;
			}
			w = cv.wait_timeout(w, left).unwrap().0;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::Arc;

	fn state() -> AppState {
		let dir = std::env::temp_dir().join(format!("msm-wake-{}", uuid::Uuid::new_v4()));
		std::fs::create_dir_all(&dir).unwrap();
		crate::commands::init_state(dir)
	}

	#[test]
	fn hide_then_show_during_collection_is_not_slept_through() {
		// The loop reads the generation, collects (slow), and only then waits. A
		// hide→show inside that window notifies a condvar nobody is waiting on, so
		// the generation check is what must cut the wait short.
		let st = state();
		st.set_visible(true);
		let generation = st.wait_visible();
		st.set_visible(false);
		st.set_visible(true);
		let started = Instant::now();
		st.wait_interval(generation, Duration::from_secs(30));
		assert!(started.elapsed() < Duration::from_secs(1), "stale generation must return at once");
	}

	#[test]
	fn repeated_hide_does_not_bump_the_generation() {
		// hide_popover and the hide-on-blur handler can both fire; only a real
		// change should cost the loops a fresh pass.
		let st = state();
		st.set_visible(true);
		let generation = st.wait_visible();
		st.set_visible(false);
		st.set_visible(false);
		st.set_visible(true);
		assert_eq!(st.wait_visible(), generation.wrapping_add(2));
	}

	#[test]
	fn wait_visible_blocks_until_shown() {
		let st = Arc::new(state());
		assert!(!st.visible.load(Ordering::Relaxed));
		let writer = Arc::clone(&st);
		let handle = std::thread::spawn(move || {
			std::thread::sleep(Duration::from_millis(50));
			writer.set_visible(true);
		});
		let started = Instant::now();
		st.wait_visible();
		assert!(st.visible.load(Ordering::Relaxed));
		assert!(started.elapsed() >= Duration::from_millis(40), "returned before the show");
		handle.join().unwrap();
	}

	#[test]
	fn wait_interval_returns_when_hidden_mid_wait() {
		let st = Arc::new(state());
		st.set_visible(true);
		let generation = st.wait_visible();
		let writer = Arc::clone(&st);
		let handle = std::thread::spawn(move || {
			std::thread::sleep(Duration::from_millis(50));
			writer.set_visible(false);
		});
		let started = Instant::now();
		st.wait_interval(generation, Duration::from_secs(30));
		assert!(started.elapsed() < Duration::from_secs(1), "a hide must cut the interval short");
		handle.join().unwrap();
	}
}
