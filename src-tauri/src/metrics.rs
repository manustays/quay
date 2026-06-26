//! Per-process CPU% and memory sampling, gated on popover visibility.
//!
//! Unlike the always-on health poll ([`crate::health`]), this loop only does work
//! while the popover is open (`AppState.visible`). Each pass takes a self-contained
//! pair of `sysinfo` refreshes 200 ms apart so CPU% is a valid instantaneous delta,
//! then aggregates each service's whole process tree (root PIDs + descendants) so
//! wrapper processes (e.g. `npm` → `node`) report the real consumption.

use crate::model::Status;
use crate::state::AppState;
use crate::supervisor;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::time::Duration;
use sysinfo::{MINIMUM_CPU_UPDATE_INTERVAL, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter, Manager};

/// Resource usage for one item, pushed to the frontend via `metrics_changed`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ItemMetrics {
	pub id: String,
	/// Summed CPU usage across the process tree, in percent (can exceed 100 on
	/// multi-core machines — it is the sum of per-core percentages).
	#[serde(rename = "cpuPercent")]
	pub cpu_percent: f32,
	/// Summed resident memory across the process tree, in bytes.
	#[serde(rename = "memoryBytes")]
	pub memory_bytes: u64,
}

/// Sum cpu% + memory over the union of subtrees rooted at `roots`, deduping
/// shared PIDs. Pure — `children` maps parent PID → child PIDs, `samples` maps
/// PID → (cpu%, memory bytes). PIDs absent from `samples` contribute nothing.
pub fn aggregate_tree(
	roots: &[u32],
	children: &HashMap<u32, Vec<u32>>,
	samples: &HashMap<u32, (f32, u64)>,
) -> (f32, u64) {
	let mut seen: HashSet<u32> = HashSet::new();
	let mut stack: Vec<u32> = roots.to_vec();
	let (mut cpu, mut mem) = (0.0_f32, 0_u64);
	while let Some(pid) = stack.pop() {
		if !seen.insert(pid) {
			continue;
		}
		if let Some(&(c, m)) = samples.get(&pid) {
			cpu += c;
			mem += m;
		}
		if let Some(kids) = children.get(&pid) {
			stack.extend(kids.iter().copied());
		}
	}
	(cpu, mem)
}

/// Sample metrics for every running/starting item. Returns one entry per item
/// that resolves to at least one live PID; an empty vec when nothing is running
/// (the frontend replaces its whole map per event, so this clears stale rows).
pub fn collect(app: &AppHandle) -> Vec<ItemMetrics> {
	let state = app.state::<AppState>();

	// Snapshot everything we need under locks, then release before blocking I/O
	// (lsof in `pids_listening`, the 200 ms CPU sample) happens lock-free.
	let items = state.config.lock().unwrap().items.clone();
	let statuses = state.statuses.lock().unwrap().clone();
	let tracked: HashMap<String, u32> = state
		.running
		.lock()
		.unwrap()
		.iter()
		.map(|(id, r)| (id.clone(), r.pid))
		.collect();

	// Resolve root PIDs per item. A port is resolved at most once per pass.
	let mut port_cache: HashMap<u16, Vec<u32>> = HashMap::new();
	let mut item_roots: Vec<(String, Vec<u32>)> = Vec::new();
	for item in &items {
		match statuses.get(&item.id) {
			Some(Status::Running | Status::Starting) => {}
			_ => continue,
		}
		let mut roots: Vec<u32> = Vec::new();
		if let Some(&pid) = tracked.get(&item.id) {
			roots.push(pid);
		}
		// Port-resolved listeners cover terminal/brew items and background
		// servers whose tracked launcher PID has been reparented away.
		if let Some(port) = item.port {
			let pids = port_cache
				.entry(port)
				.or_insert_with(|| supervisor::pids_listening(port));
			roots.extend(pids.iter().copied());
		}
		roots.sort_unstable();
		roots.dedup();
		if !roots.is_empty() {
			item_roots.push((item.id.clone(), roots));
		}
	}
	if item_roots.is_empty() {
		return Vec::new();
	}

	// Two refreshes 200 ms apart give cpu_usage() a valid delta to measure.
	let mut sys = System::new();
	sys.refresh_processes(ProcessesToUpdate::All, true);
	std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
	sys.refresh_processes(ProcessesToUpdate::All, true);

	let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
	let mut samples: HashMap<u32, (f32, u64)> = HashMap::new();
	for (pid, proc_) in sys.processes() {
		let pid_u = pid.as_u32();
		samples.insert(pid_u, (proc_.cpu_usage(), proc_.memory()));
		if let Some(parent) = proc_.parent() {
			children.entry(parent.as_u32()).or_default().push(pid_u);
		}
	}

	item_roots
		.into_iter()
		.map(|(id, roots)| {
			let (cpu_percent, memory_bytes) = aggregate_tree(&roots, &children, &samples);
			ItemMetrics { id, cpu_percent, memory_bytes }
		})
		.collect()
}

/// Spawn the visibility-gated metrics loop. Idle-ticks every 500 ms while hidden
/// (≤0.5 s latency to first sample on open) and does no sampling work; while
/// visible it samples, re-checks visibility, then emits `metrics_changed`.
pub fn spawn_metrics_loop(app: AppHandle) {
	std::thread::spawn(move || loop {
		let visible = app.state::<AppState>().visible.load(Ordering::Relaxed);
		if !visible {
			std::thread::sleep(Duration::from_millis(500));
			continue;
		}
		let metrics = collect(&app);
		// Re-check after the (~200 ms + lsof) collection: the popover may have
		// closed meanwhile, in which case skip the emit.
		if app.state::<AppState>().visible.load(Ordering::Relaxed) {
			let _ = app.emit("metrics_changed", &metrics);
		}
		let interval = app
			.state::<AppState>()
			.config
			.lock()
			.unwrap()
			.settings
			.metrics_interval_sec
			.max(1);
		std::thread::sleep(Duration::from_secs(interval));
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	fn samples(pairs: &[(u32, f32, u64)]) -> HashMap<u32, (f32, u64)> {
		pairs.iter().map(|&(p, c, m)| (p, (c, m))).collect()
	}

	#[test]
	fn sums_root_plus_descendants() {
		// 100 → 200 → 300 (npm → node → worker)
		let children: HashMap<u32, Vec<u32>> =
			HashMap::from([(100, vec![200]), (200, vec![300])]);
		let s = samples(&[(100, 1.0, 10), (200, 5.0, 50), (300, 2.0, 20)]);
		assert_eq!(aggregate_tree(&[100], &children, &s), (8.0, 80));
	}

	#[test]
	fn dedupes_overlapping_roots() {
		// tracked PID 100 and a port-resolved child 200 both passed as roots.
		let children: HashMap<u32, Vec<u32>> = HashMap::from([(100, vec![200])]);
		let s = samples(&[(100, 1.0, 10), (200, 5.0, 50)]);
		assert_eq!(aggregate_tree(&[100, 200], &children, &s), (6.0, 60));
	}

	#[test]
	fn missing_samples_contribute_nothing() {
		let children: HashMap<u32, Vec<u32>> = HashMap::new();
		let s = samples(&[]);
		assert_eq!(aggregate_tree(&[999], &children, &s), (0.0, 0));
	}
}
