/** Live status of an item — mirrors the Rust `Status` enum (serde rename_all = "snake_case"). */
export type Status = 'stopped' | 'starting' | 'running' | 'error';

/** Kind of managed item — mirrors the Rust `ItemKind` enum. */
export type ItemKind = 'project' | 'brew' | 'cli' | 'docker';

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
	/** Docker image "repo:tag" — drives add-form autofill only. */
	dockerImage: string | null;
	/** Container name — the join key for Docker status, stop, and metrics. */
	containerName: string | null;
	/** Detected tech stack keyword (e.g. "vite", "django") for the row icon. */
	stack: string | null;
	/** Optional group label — grouped items cluster and start/stop together. */
	group: string | null;
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
	/** Ports hidden from the Detected (port radar) section. */
	ignoredPorts: number[];
}

/**
 * An unmanaged TCP listener found by the backend port radar — mirrors the Rust
 * `DiscoveredPort` struct. `managedItemId` is set when the port belongs to a
 * registered item (a collision badge, not an adoptable listener).
 */
export interface DiscoveredPort {
	port: number;
	pid: number;
	name: string;
	command: string;
	cwd: string | null;
	stack: string | null;
	managedItemId: string | null;
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
	/** Seconds since the root process started (null for Docker items). */
	uptimeSec: number | null;
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
	/** Detected tech stack keyword (e.g. "vite", "django"), if recognizable. */
	stack: string | null;
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
 * Format an uptime in seconds as a compact human string
 * (42 → "42s", 3720 → "1h 2m", 90000 → "1d 1h").
 */
export function formatUptime(sec: number): string {
	if (sec < 60) return `${sec}s`;
	const minutes = Math.floor(sec / 60);
	if (minutes < 60) return `${minutes}m`;
	const hours = Math.floor(minutes / 60);
	if (hours < 24) return `${hours}h ${minutes % 60}m`;
	return `${Math.floor(hours / 24)}d ${hours % 24}h`;
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

/** Return a copy of `list` with the item at index `from` moved to index `to`. */
export function moveInList<T>(list: T[], from: number, to: number): T[] {
	const next = [...list];
	const [moved] = next.splice(from, 1);
	next.splice(to, 0, moved);
	return next;
}

/**
 * Split a list into named group clusters and ungrouped items. Groups are
 * ordered by their first member's position (i.e. min `order` for an
 * order-sorted input); members keep their relative order within the group.
 */
export function groupItems(items: ManagedItem[]): {
	groups: { name: string; items: ManagedItem[] }[];
	ungrouped: ManagedItem[];
} {
	const groups: { name: string; items: ManagedItem[] }[] = [];
	const byName = new Map<string, ManagedItem[]>();
	const ungrouped: ManagedItem[] = [];
	for (const item of items) {
		if (!item.group) {
			ungrouped.push(item);
			continue;
		}
		let members = byName.get(item.group);
		if (!members) {
			members = [];
			byName.set(item.group, members);
			groups.push({ name: item.group, items: members });
		}
		members.push(item);
	}
	return { groups, ungrouped };
}

/**
 * Aggregate member statuses for a group row dot:
 * any error > any starting > all running > stopped.
 */
export function aggregateGroupStatus(statuses: Status[]): Status {
	if (statuses.includes('error')) return 'error';
	if (statuses.includes('starting')) return 'starting';
	if (statuses.length > 0 && statuses.every((s) => s === 'running')) return 'running';
	return 'stopped';
}

/**
 * Aggregate member metrics for a group row: summed CPU% and memory, max
 * uptime. Returns null when no member has metrics (nothing running).
 */
export function aggregateGroupMetrics(
	list: ItemMetrics[],
): { cpuPercent: number; memoryBytes: number; uptimeSec: number | null } | null {
	if (list.length === 0) return null;
	const uptimes = list.map((m) => m.uptimeSec).filter((u): u is number => u != null);
	return {
		cpuPercent: list.reduce((sum, m) => sum + m.cpuPercent, 0),
		memoryBytes: list.reduce((sum, m) => sum + m.memoryBytes, 0),
		uptimeSec: uptimes.length > 0 ? Math.max(...uptimes) : null,
	};
}
