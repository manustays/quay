/** Live status of an item — mirrors the Rust `Status` enum (serde rename_all = "snake_case"). */
export type Status = 'stopped' | 'starting' | 'running' | 'error';

/** Kind of managed item — mirrors the Rust `ItemKind` enum. */
export type ItemKind = 'project' | 'brew' | 'agent';

/** How a process is launched — mirrors the Rust `RunMode` enum. */
export type RunMode = 'background' | 'terminal';

/**
 * A registered service — mirrors the Rust `ManagedItem` struct
 * with serde rename_all = "camelCase".
 */
export interface ManagedItem {
	id: string;
	name: string;
	kind: ItemKind;
	dir: string | null;
	startCmd: string | null;
	stopCmd: string | null;
	port: number | null;
	runMode: RunMode;
	brewFormula: string | null;
	order: number;
	favorite: boolean;
	env: Record<string, string>;
	healthPath: string | null;
	autoStart: boolean;
}

/**
 * Global app settings — mirrors the Rust `Settings` struct
 * with serde rename_all = "camelCase".
 */
export interface Settings {
	terminalApp: string;
	pollIntervalSec: number;
	metricsIntervalSec: number;
	browser: string;
	launchAtLogin: boolean;
}

/**
 * Per-item resource usage — mirrors the Rust `ItemMetrics` struct.
 * `cpuPercent` is summed across the process tree (may exceed 100 on multi-core);
 * `memoryBytes` is summed resident memory in bytes.
 */
export interface ItemMetrics {
	id: string;
	cpuPercent: number;
	memoryBytes: number;
}

/**
 * Live status snapshot for a single item — mirrors the Rust `ItemStatus` struct
 * with serde rename_all = "camelCase".
 */
export interface ItemStatus {
	id: string;
	status: Status;
	lastError: string | null;
}

/** Suggested item config returned by the detect_folder_cmd backend command. Mirrors Rust `DetectResult`. */
export interface DetectResult {
	name: string;
	kind: ItemKind;
	startCmd: string | null;
	port: number | null;
}

/**
 * Return a glyph + CSS class string for a given status.
 * Each string contains the status name so callers can test with `.toContain`.
 */
export function statusDot(status: Status): string {
	const map: Record<Status, string> = {
		running: '● running',
		starting: '◐ starting',
		stopped: '○ stopped',
		error: '✖ error',
	};
	return map[status];
}

/**
 * Format a byte count as a compact human string (e.g. 134217728 → "128 MB").
 * Uses binary units; falls back to "0 B" for zero/negatives.
 */
export function formatBytes(bytes: number): string {
	if (bytes <= 0) return '0 B';
	const units = ['B', 'KB', 'MB', 'GB', 'TB'];
	const exp = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
	const value = bytes / 1024 ** exp;
	return `${value < 10 && exp > 0 ? value.toFixed(1) : Math.round(value)} ${units[exp]}`;
}

/**
 * Case-insensitive substring match across an item's name, kind, and port.
 * An empty query matches everything.
 */
export function matchesSearch(item: ManagedItem, query: string): boolean {
	const q = query.trim().toLowerCase();
	if (!q) return true;
	return (
		item.name.toLowerCase().includes(q) ||
		item.kind.includes(q) ||
		(item.port != null && String(item.port).includes(q))
	);
}

/**
 * Split items into favorites and others, each sub-list sorted ascending by `order`.
 */
export function splitFavorites(
	items: ManagedItem[],
): { favorites: ManagedItem[]; others: ManagedItem[] } {
	const sorted = [...items].sort((a, b) => a.order - b.order);
	return {
		favorites: sorted.filter(i => i.favorite),
		others: sorted.filter(i => !i.favorite),
	};
}
