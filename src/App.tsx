import { useCallback, useEffect, useMemo, useState } from 'react';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { TooltipProvider } from '@/components/ui/tooltip';
import { detectFolder, getItems, getSettings, getStatuses, onAgentsDiscovered, onMetricsChanged, onPortsDiscovered, onStatusChanged } from './ipc';
import { blankItem, type DiscoveredAgent, type DiscoveredPort, type ItemMetrics, type ManagedItem, type Status } from './model';
import { Popup } from './components/Popup';
import { ServiceForm } from './components/ServiceForm';
import { SettingsDialog } from './components/SettingsDialog';

/** Keep the `.dark` class on <html> in sync with the macOS system appearance. */
function useSystemTheme(): void {
	useEffect(() => {
		const mql = window.matchMedia('(prefers-color-scheme: dark)');
		const apply = (dark: boolean) =>
			document.documentElement.classList.toggle('dark', dark);
		apply(mql.matches);
		const onChange = (e: MediaQueryListEvent) => apply(e.matches);
		mql.addEventListener('change', onChange);
		return () => mql.removeEventListener('change', onChange);
	}, []);
}

/**
 * Root component. Owns the live item list, per-item status/error maps, and the
 * add/edit/settings dialog state. Subscribes once to backend `status_changed`
 * events and tears the listener down on unmount.
 */
export function App(): React.JSX.Element {
	const [items, setItems] = useState<ManagedItem[]>([]);
	const [statuses, setStatuses] = useState<Map<string, Status>>(new Map());
	const [lastErrors, setLastErrors] = useState<Map<string, string>>(new Map());
	const [metrics, setMetrics] = useState<Map<string, ItemMetrics>>(new Map());
	const [discovered, setDiscovered] = useState<DiscoveredPort[]>([]);
	const [agents, setAgents] = useState<DiscoveredAgent[]>([]);
	// Radar filter toggle, mirrored from persisted settings; re-read on save.
	// Defaults on (matches the Rust default) so the first paint before settings
	// load doesn't flash the unfiltered list.
	const [radarDevOnly, setRadarDevOnly] = useState(true);

	// Dialog state: `editing` is undefined when closed, null for "add new",
	// or the item being edited; `settingsOpen` toggles the settings dialog.
	const [editing, setEditing] = useState<ManagedItem | null | undefined>(undefined);
	const [settingsOpen, setSettingsOpen] = useState(false);

	useSystemTheme();

	const refresh = useCallback(async () => {
		setItems(await getItems());
	}, []);

	// Pull the radar filter flag from persisted settings. Called on mount and
	// after the settings dialog saves so the toggle takes effect immediately.
	const reloadSettings = useCallback(async () => {
		setRadarDevOnly((await getSettings()).radarDevOnly);
	}, []);

	/**
	 * Open the add form prefilled from a discovered listener. When the process
	 * cwd is known, folder detection refines the prefill (manifest start script
	 * beats raw argv, which may not restart cleanly outside its launcher).
	 */
	const adopt = useCallback(async (entry: DiscoveredPort) => {
		const detected = entry.cwd ? await detectFolder(entry.cwd).catch(() => null) : null;
		// blankItem() carries the empty id that marks an add-mode draft.
		setEditing({
			...blankItem(),
			name: detected?.name ?? entry.name,
			dir: entry.cwd,
			startCmd: detected?.startCmd ?? entry.command,
			port: entry.port,
			stack: detected?.stack ?? entry.stack,
		});
	}, []);

	/**
	 * Optimistically drop a discovered port's rows after a successful kill or
	 * ignore — the next radar snapshot (≤5 s away) is the source of truth and
	 * restores anything actually still listening.
	 */
	const dismissDiscovered = useCallback((entry: DiscoveredPort) => {
		setDiscovered((prev) => prev.filter((d) => d.port !== entry.port));
	}, []);

	useEffect(() => {
		void refresh();
		void reloadSettings();

		// onStatusChanged resolves to an unlisten fn asynchronously; guard against
		// the effect being torn down before the subscription resolves.
		let cancelled = false;
		const unlisteners: UnlistenFn[] = [];
		// Register an unlisten fn, or call it immediately if we already unmounted
		// (the listen() promise can resolve after teardown).
		const track = (fn: UnlistenFn) => {
			if (cancelled) fn();
			else unlisteners.push(fn);
		};

		void onStatusChanged((s) => {
			setStatuses((prev) => new Map(prev).set(s.id, s.status));
			setLastErrors((prev) => {
				const next = new Map(prev);
				if (s.lastError != null) next.set(s.id, s.lastError);
				else if (s.status !== 'error') next.delete(s.id);
				return next;
			});
		}).then(track);

		// Metrics arrive as a full snapshot per tick; rebuild the map wholesale so
		// stopped/removed items drop out (no stale CPU/memory lingers).
		void onMetricsChanged((list) => {
			setMetrics(new Map(list.map((m) => [m.id, m])));
		}).then(track);

		// Port-radar and agent-radar snapshots likewise replace wholesale per pass.
		void onPortsDiscovered(setDiscovered).then(track);
		void onAgentsDiscovered(setAgents).then(track);

		// Seed current statuses once. `status_changed` only fires on change, so a
		// status set by the backend's startup poll (before this listener attached)
		// would never arrive otherwise. Gap-fill only: any id already updated by a
		// live event that raced ahead of this fetch keeps its newer value.
		void getStatuses().then((initial) => {
			if (cancelled || initial.length === 0) return;
			setStatuses((prev) => {
				const next = new Map(prev);
				for (const s of initial) if (!next.has(s.id)) next.set(s.id, s.status);
				return next;
			});
			setLastErrors((prev) => {
				const next = new Map(prev);
				for (const s of initial) {
					if (s.lastError != null && !next.has(s.id)) next.set(s.id, s.lastError);
				}
				return next;
			});
		});

		return () => {
			cancelled = true;
			for (const fn of unlisteners) fn();
		};
	}, [refresh, reloadSettings]);

	// Recomputed only when items change, not on every metrics/radar tick.
	const groups = useMemo(
		() => Array.from(new Set(items.map((i) => i.group).filter((g): g is string => !!g))),
		[items],
	);

	return (
		<TooltipProvider delayDuration={300}>
			<Popup
				items={items}
				statuses={statuses}
				lastErrors={lastErrors}
				metrics={metrics}
				discovered={discovered}
				agents={agents}
				radarDevOnly={radarDevOnly}
				onChange={refresh}
				onAdd={() => setEditing(null)}
				onEdit={(item) => setEditing(item)}
				onAdopt={(entry) => void adopt(entry)}
				onDismissDiscovered={dismissDiscovered}
				onSettings={() => setSettingsOpen(true)}
			/>
			<ServiceForm
				open={editing !== undefined}
				item={editing ?? null}
				groups={groups}
				onOpenChange={(open) => { if (!open) setEditing(undefined); }}
				onSaved={refresh}
			/>
			<SettingsDialog
				open={settingsOpen}
				onOpenChange={setSettingsOpen}
				onSaved={() => { void refresh(); void reloadSettings(); }}
			/>
		</TooltipProvider>
	);
}
