import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
	AgentKind,
	DetectResult,
	DiscoveredAgent,
	DiscoveredPort,
	HookStatus,
	ItemMetrics,
	ItemStatus,
	ManagedItem,
	Settings,
	UpdateInfo,
} from './model';

export const getItems = () => invoke<ManagedItem[]>('get_items');
export const addItem = (item: ManagedItem) => invoke<ManagedItem>('add_item', { item });
export const updateItem = (item: ManagedItem) => invoke<void>('update_item', { item });
export const deleteItem = (id: string) => invoke<void>('delete_item', { id });
export const reorder = (ids: string[]) => invoke<void>('reorder', { ids });
export const toggleFavorite = (id: string) => invoke<void>('toggle_favorite', { id });
export const startItem = (id: string) => invoke<void>('start_item', { id });
export const stopItem = (id: string) => invoke<void>('stop_item', { id });
/** Mark a crashed/killed item as normally stopped — no signal, no port kill. */
export const markStopped = (id: string) => invoke<void>('mark_stopped', { id });
export const stopAll = () => invoke<void>('stop_all');
export const openBrowser = (id: string) => invoke<void>('open_browser', { id });
export const openTerminal = (id: string) => invoke<void>('open_terminal', { id });
export const revealInFinder = (id: string) => invoke<void>('reveal_in_finder', { id });
export const tailLog = (id: string, lines: number) => invoke<string>('tail_log', { id, lines });
export const detectFolder = (path: string) => invoke<DetectResult>('detect_folder_cmd', { path });
export const getStatuses = () => invoke<ItemStatus[]>('get_statuses');
export const getSettings = () => invoke<Settings>('get_settings');
export const updateSettings = (settings: Settings) => invoke<void>('update_settings', { settings });

/** List terminal apps detected as installed, for the settings picker. */
export const getTerminals = () => invoke<string[]>('get_terminals');

/** Per-agent hook-install state for the Settings pane. */
export const getHookStatuses = () => invoke<HookStatus[]>('get_hook_statuses');
/** Install the quay-hook helper + one agent's hook config. */
export const installAgentHooks = (agent: AgentKind) =>
	invoke<void>('install_agent_hooks', { agent });
/** Remove one agent's hook config (leaves the shared helper binary). */
export const uninstallAgentHooks = (agent: AgentKind) =>
	invoke<void>('uninstall_agent_hooks', { agent });

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
 * Subscribe to `update_available` events. Fired when a background check finds a
 * newer release. The event can precede the webview mounting this listener (the
 * menubar app starts hidden), so also call {@link getPendingUpdate} on mount to
 * recover an update whose event was missed. Returns an unlisten function.
 */
export function onUpdateAvailable(cb: (u: UpdateInfo) => void): Promise<UnlistenFn> {
	return listen<UpdateInfo>('update_available', (e) => cb(e.payload));
}

/** The update the last backend check found, or null. Query on mount to backfill a missed event. */
export const getPendingUpdate = () => invoke<UpdateInfo | null>('get_pending_update');

/**
 * Download + install the pending update, then restart the app. Resolves only if the
 * remote is no longer newer (banner should clear); on success the app restarts and
 * this never resolves. Rejects with an error string on failure or if a check is
 * already running.
 */
export const installUpdate = () => invoke<void>('install_update');

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
 * Signal a discovered agent session (SIGTERM, or SIGKILL when `force`). The
 * backend revalidates that `pid` is still that agent session in that cwd
 * before signalling, so a stale radar row can't kill an unrelated process.
 */
export const killAgent = (pid: number, agent: string, cwd: string, force: boolean) =>
	invoke<void>('kill_agent', { pid, agent, cwd, force });

/**
 * Persistently hide all sessions of `agent` in `cwd` from the Agents section
 * (un-ignore in Settings).
 */
export const ignoreAgent = (agent: string, cwd: string) =>
	invoke<void>('ignore_agent', { agent, cwd });

/** Reveal a directory in Finder (agent rows carry a cwd, not an item id). */
export const revealPath = (path: string) => invoke<void>('reveal_path', { path });

/**
 * Focus the terminal window/tab hosting an agent session. The backend
 * revalidates pid identity AND that the pid still sits on `tty` before any
 * AppleScript runs, so a stale row can't focus someone else's window.
 */
export const jumpToSession = (pid: number, agent: string, cwd: string, tty: string) =>
	invoke<void>('jump_to_session', { pid, agent, cwd, tty });

/**
 * Subscribe to agent-radar snapshots. The callback receives the full list of
 * discovered agent sessions per scan pass (only while the popover is open);
 * replace state wholesale so ended sessions drop out. Returns an unlisten fn.
 */
export function onAgentsDiscovered(cb: (a: DiscoveredAgent[]) => void): Promise<UnlistenFn> {
	return listen<DiscoveredAgent[]>('agents_discovered', (e) => cb(e.payload));
}

/**
 * Suppress (or re-enable) hide-on-blur in the Rust backend.
 * Call with `true` before opening a native dialog and `false` in a `finally`
 * block after it closes, so the popover stays visible during the pick flow.
 */
export const setSuppressHide = (value: boolean) => invoke<void>('set_suppress_hide', { value });
