use crate::model::{AppConfig, Status, UpdateInfo};
use crate::supervisor::Running;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Everything that decides whether Quay should be doing work right now.
///
/// Three independent inputs, because they change independently: the popover can be
/// open while the screens sleep (display sleep does not defocus a window), and the
/// Mac can be locked with the screens still lit.
///
/// `generation` bumps on every real change, so a loop that was busy collecting can
/// tell that a hide→show happened while it worked: a `notify_all` it wasn't waiting
/// for is otherwise lost, and it would sleep out its whole interval before noticing.
#[derive(Clone, Copy, Default)]
pub struct Wake {
	pub visible: bool,
	/// All attached displays are asleep (`NSWorkspaceScreensDidSleep`).
	pub screens_asleep: bool,
	/// The session is locked (`com.apple.screenIsLocked`).
	pub locked: bool,
	pub generation: u64,
}

impl Wake {
	/// Can a human see the menubar at all? Gates the always-on health loop: with
	/// nothing on screen, its status pass and tray badge are computed for nobody.
	pub fn awake(&self) -> bool {
		!self.screens_asleep && !self.locked
	}

	/// Is the popover both open *and* visible to someone? Gates the heavy loops,
	/// and is what the frontend uses to decide whether to animate.
	pub fn active(&self) -> bool {
		self.visible && self.awake()
	}
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
	/// Lock-free mirror of [`Wake::active`] — popover shown *and* someone able to
	/// see it. Read on the emit paths, which must not take the wake lock. Written
	/// only from [`AppState::mutate`], under that lock.
	pub active: AtomicBool,
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
	/// The real state behind `active`, under a lock so the gated loops can block on
	/// a condvar instead of idle-ticking. Writers must go through the setters below.
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

	/// Apply a change to the wake state and notify anyone it unblocks.
	///
	/// `f` returns whether it actually changed anything: the generation only moves on
	/// a real change, so a repeated `false` (hide, then hide-on-blur) doesn't cost a
	/// redundant scan. The `active` mirror is stored *under* the lock so a change
	/// landing between a loop's check and its wait can't be lost.
	fn mutate(&self, f: impl FnOnce(&mut Wake) -> bool) {
		let (lock, cv) = &self.wake;
		let mut w = lock.lock().unwrap();
		let changed = f(&mut w);
		self.active.store(w.active(), Ordering::Relaxed);
		if changed {
			w.generation = w.generation.wrapping_add(1);
			cv.notify_all();
		}
	}

	/// Set popover visibility. See [`crate::set_popover_visible`], the single writer.
	pub fn set_visible(&self, vis: bool) {
		self.mutate(|w| std::mem::replace(&mut w.visible, vis) != vis);
	}

	/// All displays asleep / awake again. Driven by `mac_power`'s NSWorkspace observer.
	pub fn set_screens_asleep(&self, asleep: bool) {
		self.mutate(|w| std::mem::replace(&mut w.screens_asleep, asleep) != asleep);
	}

	/// Session locked / unlocked. Driven by `mac_power`'s distributed-notification observer.
	pub fn set_locked(&self, locked: bool) {
		self.mutate(|w| std::mem::replace(&mut w.locked, locked) != locked);
	}

	/// Lock-free read of [`Wake::active`], for the emit paths.
	pub fn is_active(&self) -> bool {
		self.active.load(Ordering::Relaxed)
	}

	/// Could a human see the menubar right now? Distinct from [`Self::is_active`],
	/// which also requires the popover to be open.
	pub fn is_awake(&self) -> bool {
		self.wake.0.lock().unwrap().awake()
	}

	/// Block until `ready`; returns the generation observed. While not ready this
	/// costs zero wakeups (no idle tick), and a change is picked up immediately.
	fn wait_until(&self, ready: fn(&Wake) -> bool) -> u64 {
		let (lock, cv) = &self.wake;
		let mut w = lock.lock().unwrap();
		while !ready(&w) {
			w = cv.wait(w).unwrap();
		}
		w.generation
	}

	/// Sleep up to `dur`, returning early if the state changed since `generation`.
	/// Checking the generation *under the lock* covers a change that happened while
	/// the caller was collecting, which a bare `sleep` would sit through.
	fn wait_interval_until(&self, generation: u64, dur: Duration, ready: fn(&Wake) -> bool) {
		let (lock, cv) = &self.wake;
		let mut w = lock.lock().unwrap();
		let deadline = Instant::now() + dur;
		while ready(&w) && w.generation == generation {
			let Some(left) = deadline.checked_duration_since(Instant::now()) else { break };
			if left.is_zero() {
				break;
			}
			w = cv.wait_timeout(w, left).unwrap().0;
		}
	}

	/// Block until the popover is open *and* someone can see it — the metrics and
	/// radar gate. A screen sleep parks these loops even with the popover left open,
	/// which a bare visibility check would miss.
	pub fn wait_active(&self) -> u64 {
		self.wait_until(Wake::active)
	}

	/// Interval wait for the [`Self::wait_active`] loops.
	pub fn wait_interval(&self, generation: u64, dur: Duration) {
		self.wait_interval_until(generation, dur, Wake::active)
	}

	/// Block until a human could see the menubar — the always-on health loop's gate.
	/// Independent of the popover: the tray icon and its badge are on screen whether
	/// or not the popover is open, and on no screen at all when the display sleeps.
	pub fn wait_awake(&self) -> u64 {
		self.wait_until(Wake::awake)
	}

	/// Interval wait for the [`Self::wait_awake`] loop.
	pub fn wait_awake_interval(&self, generation: u64, dur: Duration) {
		self.wait_interval_until(generation, dur, Wake::awake)
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
		let generation = st.wait_active();
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
		let generation = st.wait_active();
		st.set_visible(false);
		st.set_visible(false);
		st.set_visible(true);
		assert_eq!(st.wait_active(), generation.wrapping_add(2));
	}

	#[test]
	fn wait_active_blocks_until_shown() {
		let st = Arc::new(state());
		assert!(!st.is_active());
		let writer = Arc::clone(&st);
		let handle = std::thread::spawn(move || {
			std::thread::sleep(Duration::from_millis(50));
			writer.set_visible(true);
		});
		let started = Instant::now();
		st.wait_active();
		assert!(st.is_active());
		assert!(started.elapsed() >= Duration::from_millis(40), "returned before the show");
		handle.join().unwrap();
	}

	#[test]
	fn wait_interval_returns_when_hidden_mid_wait() {
		let st = Arc::new(state());
		st.set_visible(true);
		let generation = st.wait_active();
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

	#[test]
	fn screens_asleep_parks_the_always_on_loop() {
		// The health loop's whole reason to exist is the tray icon and its badge.
		// With every display asleep there is no tray to look at.
		let st = Arc::new(state());
		st.set_screens_asleep(true);
		let writer = Arc::clone(&st);
		let handle = std::thread::spawn(move || {
			std::thread::sleep(Duration::from_millis(50));
			writer.set_screens_asleep(false);
		});
		let started = Instant::now();
		st.wait_awake();
		assert!(started.elapsed() >= Duration::from_millis(40), "returned before the wake");
		handle.join().unwrap();
	}

	#[test]
	fn sleep_and_lock_clear_independently_and_in_any_order() {
		// The two signals arrive from different notification centres and in either
		// order: locking usually sleeps the display shortly after, and unlocking
		// wakes it first. Work may only resume once *both* are clear.
		let st = state();
		st.set_screens_asleep(true);
		st.set_locked(true);
		st.set_screens_asleep(false);
		assert!(!st.is_active(), "still locked");
		let woke = Arc::new(AtomicBool::new(false));
		let flag = Arc::clone(&woke);
		// A waiter must still be parked with only one of the two cleared.
		std::thread::scope(|scope| {
			let st = &st;
			scope.spawn(move || {
				st.wait_awake();
				flag.store(true, Ordering::Relaxed);
			});
			std::thread::sleep(Duration::from_millis(50));
			assert!(!woke.load(Ordering::Relaxed), "resumed while still locked");
			st.set_locked(false);
		});
		assert!(woke.load(Ordering::Relaxed), "both clear must resume");
	}

	#[test]
	fn screen_sleep_releases_the_gated_loops_with_the_popover_open() {
		// Display sleep does not defocus a window, so hide-on-blur never fires and
		// the popover stays "visible". Without the awake term the metrics and radar
		// loops would keep forking lsof/ps at a dark screen.
		let st = Arc::new(state());
		st.set_visible(true);
		let generation = st.wait_active();
		let writer = Arc::clone(&st);
		let handle = std::thread::spawn(move || {
			std::thread::sleep(Duration::from_millis(50));
			writer.set_screens_asleep(true);
		});
		let started = Instant::now();
		st.wait_interval(generation, Duration::from_secs(30));
		assert!(started.elapsed() < Duration::from_secs(1), "a screen sleep must cut the interval short");
		assert!(!st.is_active(), "a dark screen is not active");
		handle.join().unwrap();
	}

	#[test]
	fn a_lock_mid_interval_cuts_the_wait_short() {
		let st = Arc::new(state());
		let generation = st.wait_awake();
		let writer = Arc::clone(&st);
		let handle = std::thread::spawn(move || {
			std::thread::sleep(Duration::from_millis(50));
			writer.set_locked(true);
		});
		let started = Instant::now();
		st.wait_awake_interval(generation, Duration::from_secs(30));
		assert!(started.elapsed() < Duration::from_secs(1), "a lock must cut the interval short");
		handle.join().unwrap();
	}
}
