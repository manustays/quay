import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
	DetectResult,
	DiscoveredPort,
	ItemMetrics,
	ItemStatus,
	ManagedItem,
	Settings,
} from './model';

export const getItems = () => invoke<ManagedItem[]>('get_items');
export const addItem = (item: ManagedItem) => invoke<ManagedItem>('add_item', { item });
export const updateItem = (item: ManagedItem) => invoke<void>('update_item', { item });
export const deleteItem = (id: string) => invoke<void>('delete_item', { id });
export const reorder = (ids: string[]) => invoke<void>('reorder', { ids });
export const toggleFavorite = (id: string) => invoke<void>('toggle_favorite', { id });
export const startItem = (id: string) => invoke<void>('start_item', { id });
export const stopItem = (id: string) => invoke<void>('stop_item', { id });
export const stopAll = () => invoke<void>('stop_all');
export const openBrowser = (id: string) => invoke<void>('open_browser', { id });
export const openTerminal = (id: string) => invoke<void>('open_terminal', { id });
export const tailLog = (id: string, lines: number) => invoke<string>('tail_log', { id, lines });
export const detectFolder = (path: string) => invoke<DetectResult>('detect_folder_cmd', { path });
export const getStatuses = () => invoke<ItemStatus[]>('get_statuses');
export const getSettings = () => invoke<Settings>('get_settings');
export const updateSettings = (settings: Settings) => invoke<void>('update_settings', { settings });

/** List terminal apps detected as installed, for the settings picker. */
export const getTerminals = () => invoke<string[]>('get_terminals');

/**
 * List formula names known to `brew services`.
 * Returns an empty array when Homebrew is unavailable.
 */
export const listBrewFormulae = () => invoke<string[]>('list_brew_formulae');

/**
 * List installed Docker image "repo:tag" strings for add-service autocomplete.
 * Returns an empty array when Docker is unavailable (CLI missing or daemon down).
 */
export const listDockerImages = () => invoke<string[]>('list_docker_images');

/** True if the Docker daemon is currently responding. */
export const dockerDaemonRunning = () => invoke<boolean>('docker_daemon_running');

/**
 * Launch Docker Desktop and wait for the daemon. Resolves `true` if it came up
 * within the backend timeout (~60s), `false` on timeout; rejects if Docker
 * Desktop could not be launched.
 */
export const startDockerDaemon = () => invoke<boolean>('start_docker_daemon');

/**
 * Subscribe to backend status-changed events.
 * The callback receives an {@link ItemStatus} payload each time a service
 * transitions state. Returns a Promise that resolves to an unlisten function —
 * call it to stop receiving events (e.g. on component unmount).
 */
export function onStatusChanged(cb: (s: ItemStatus) => void): Promise<UnlistenFn> {
	return listen<ItemStatus>('status_changed', (e) => cb(e.payload));
}

/**
 * Subscribe to backend metrics events. The callback receives the full set of
 * {@link ItemMetrics} for every running item on each sampling tick (the backend
 * only emits while the popover is open). Replace your map wholesale per event so
 * stopped/removed items drop out. Returns an unlisten function.
 */
export function onMetricsChanged(cb: (m: ItemMetrics[]) => void): Promise<UnlistenFn> {
	return listen<ItemMetrics[]>('metrics_changed', (e) => cb(e.payload));
}

/**
 * Signal an unmanaged discovered listener (SIGTERM, or SIGKILL when `force`).
 * The backend revalidates that `pid` still owns `port` before signalling, so a
 * stale radar row can't kill an unrelated process.
 */
export const killDiscovered = (pid: number, port: number, force: boolean) =>
	invoke<void>('kill_discovered', { pid, port, force });

/** Persistently hide `port` from the Detected section (un-ignore in Settings). */
export const ignorePort = (port: number) => invoke<void>('ignore_port', { port });

/**
 * Subscribe to port-radar snapshots. The callback receives the full list of
 * discovered listeners per scan pass (only while the popover is open); replace
 * state wholesale so vanished listeners drop out. Returns an unlisten function.
 */
export function onPortsDiscovered(cb: (d: DiscoveredPort[]) => void): Promise<UnlistenFn> {
	return listen<DiscoveredPort[]>('ports_discovered', (e) => cb(e.payload));
}

/**
 * Suppress (or re-enable) hide-on-blur in the Rust backend.
 * Call with `true` before opening a native dialog and `false` in a `finally`
 * block after it closes, so the popover stays visible during the pick flow.
 */
export const setSuppressHide = (value: boolean) => invoke<void>('set_suppress_hide', { value });
