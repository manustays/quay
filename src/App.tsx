import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { TooltipProvider } from '@/components/ui/tooltip';
import { detectFolder, getItems, getPendingUpdate, getPopoverVisible, getSettings, getStatuses, onAgentsDiscovered, onMetricsChanged, onPopoverVisibility, onPortsDiscovered, onStatusChanged, onUpdateAvailable } from './ipc';
import { blankItem, type DiscoveredAgent, type DiscoveredPort, type ItemMetrics, type ManagedItem, type Status, type UpdateInfo } from './model';
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

	// A pending app update to show as a banner. `dismissedVersionRef` remembers the
	// version the user dismissed so the daily re-check (which re-emits the same
	// version) doesn't resurrect the banner this session; a restart re-shows it.
	const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
	const dismissedVersionRef = useRef<string | null>(null);

	const receiveUpdate = useCallback((u: UpdateInfo) => {
		if (u.version === dismissedVersionRef.current) return;
		setUpdateInfo(u);
	}, []);

	const dismissUpdate = useCallback(() => {
		setUpdateInfo((cur) => {
			if (cur) dismissedVersionRef.current = cur.version;
			return null;
		});
	}, []);

	useSystemTheme();

	const refresh = useCallback(async () => {
		setItems(await getItems());
	}, []);

	// Pull the radar filter flag from persisted settings. Called on mount and
	// after the settings dialog saves so the toggle takes effect immediately.
	const reloadSettings = useCallback(async () => {
		const settings = await getSettings();
		setRadarDevOnly(settings.radarDevOnly);
		// Tracking off: the backend stops emitting `agents_discovered`, so rows already
		// on screen would linger. Clearing here also hides the Agents section, which
		// renders only when there are agents.
		if (!settings.trackAgents) setAgents([]);
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

	/**
	 * Optimistically drop an agent row (pid-keyed) after a successful kill or
	 * ignore — the next radar snapshot (≤5 s away) is the source of truth and
	 * also hides any sibling sessions covered by a new ignore.
	 */
	const dismissAgent = useCallback((entry: DiscoveredAgent) => {
		setAgents((prev) => prev.filter((a) => a.pid !== entry.pid));
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

		// The popover is hidden, not unmounted, so its infinite `animate-pulse` dots
		// would keep compositing off-screen. Pause every animation while hidden.
		let visibilitySeeded = false;
		const applyVisibility = (visible: boolean) => {
			if (visible) delete document.documentElement.dataset.hidden;
			// An empty string still matches `[data-hidden]`; 'false' would too.
			else document.documentElement.dataset.hidden = '';
		};
		void onPopoverVisibility((visible) => {
			visibilitySeeded = true;
			applyVisibility(visible);
		}).then(track);
		void getPopoverVisible().then((visible) => {
			// A live event that landed while this was in flight is newer — an
			// out-of-order response must not resurrect the stale value.
			if (!cancelled && !visibilitySeeded) applyVisibility(visible);
		});

		// Update banner: subscribe to live events, and backfill any update whose
		// event fired before this listener mounted (the app starts hidden).
		void onUpdateAvailable(receiveUpdate).then(track);
		void getPendingUpdate().then((u) => {
			if (!cancelled && u) receiveUpdate(u);
		});

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
	}, [refresh, reloadSettings, receiveUpdate]);

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
				onDismissAgent={dismissAgent}
				onSettings={() => setSettingsOpen(true)}
				updateInfo={updateInfo}
				onDismissUpdate={dismissUpdate}
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
