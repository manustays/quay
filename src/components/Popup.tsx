import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ChevronRight, CirclePower, Play, Plus, Search, Settings, Square, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from '@/components/ui/collapsible';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { aggregateGroupMetrics, aggregateGroupStatus, groupItems, matchesSearch, moveInList, splitFavorites, type DiscoveredPort, type GroupStatus, type ItemMetrics, type ManagedItem, type Status } from '../model';
import { cn } from '@/lib/utils';
import { ensureDockerDaemon } from '@/lib/docker';
import { reorder, startItem, stopAll, stopItem } from '../ipc';
import { BuoyMark } from './BuoyMark';
import { DetectedRow } from './DetectedRow';
import { IconAction, MetricsText } from './RowBits';
import { ServiceRow, STATUS_ACCENT } from './ServiceRow';

interface PopupProps {
	items: ManagedItem[];
	statuses: Map<string, Status>;
	lastErrors: Map<string, string>;
	metrics: Map<string, ItemMetrics>;
	discovered: DiscoveredPort[];
	/** When true, hide detected listeners with no recognized dev stack. */
	radarDevOnly: boolean;
	onChange: () => void;
	onAdd: () => void;
	onEdit: (item: ManagedItem) => void;
	onAdopt: (entry: DiscoveredPort) => void;
	/** Optimistically remove a discovered entry after a successful kill/ignore. */
	onDismissDiscovered: (entry: DiscoveredPort) => void;
	onSettings: () => void;
}

// Grow-to-content bounds. Floor is the tauri.conf default so the small-list
// case matches today and the Add/Settings dialogs (overlays inside this window)
// never get clipped. The cap is the webview screen's usable height, capped for
// taste. ponytail: single-monitor only — the tray can live on another display;
// move the cap into Rust (tray monitor work-area) if that ever bites.
const POPOVER_MIN = 520;
const POPOVER_MAX = Math.min(760, Math.round(window.screen.availHeight - 16));

/**
 * Drive the native window height from the popover's content. Observes the shell
 * (whose own min/max-height clamps it to [MIN, MAX]) and reports that height to
 * the `resize_popover` command, which resizes the window and re-pins it under
 * the tray. rAF-coalesced and deduped by rounded pixel: a burst of layout ticks
 * (typing in search, expanding a row) collapses to one resize, and the
 * observe→resize→relayout cycle settles at a fixpoint instead of oscillating.
 */
function useAutoHeight(ref: React.RefObject<HTMLElement | null>): void {
	useLayoutEffect(() => {
		const el = ref.current;
		if (!el) return;
		el.style.setProperty('--popover-max', `${POPOVER_MAX}px`);
		let last = 0;
		let frame = 0;
		const report = (): void => {
			frame = 0;
			const height = Math.round(el.offsetHeight);
			if (height === last) return;
			last = height;
			// First paint may briefly show the config's 520 before this lands —
			// harmless for the common small-list case, a one-time grow otherwise.
			void invoke('resize_popover', { height });
		};
		const observer = new ResizeObserver(() => {
			if (frame === 0) frame = requestAnimationFrame(report);
		});
		observer.observe(el);
		return () => {
			observer.disconnect();
			if (frame !== 0) cancelAnimationFrame(frame);
		};
	}, [ref]);
}

/** The full popover shell: brand bar, search toolbar, scrolling list, footer. */
export function Popup({
	items,
	statuses,
	lastErrors,
	metrics,
	discovered,
	radarDevOnly,
	onChange,
	onAdd,
	onEdit,
	onAdopt,
	onDismissDiscovered,
	onSettings,
}: PopupProps): React.JSX.Element {
	const shellRef = useRef<HTMLDivElement>(null);
	useAutoHeight(shellRef);
	const [query, setQuery] = useState('');
	// Which service row is expanded — single-open accordion across all rows.
	const [expandedId, setExpandedId] = useState<string | null>(null);
	const [searchOpen, setSearchOpen] = useState(false);
	const searchRef = useRef<HTMLInputElement>(null);
	// Focus the field once it's painted (rAF avoids racing the conditional render).
	useEffect(() => {
		if (searchOpen) requestAnimationFrame(() => searchRef.current?.focus());
	}, [searchOpen]);
	const closeSearch = (): void => {
		setSearchOpen(false);
		setQuery('');
	};
	// Drag-to-reorder state: which drag list a drag started in ('fav', 'other',
	// or 'grp:<name>'), its origin index, and the hovered target.
	const [drag, setDrag] = useState<{ group: string; from: number } | null>(null);
	const [overIdx, setOverIdx] = useState<number | null>(null);
	const statusOf = (i: ManagedItem): Status => statuses.get(i.id) ?? 'stopped';

	const filtered = items.filter((i) => matchesSearch(i, query));
	const { favorites, others } = splitFavorites(filtered);
	// Group clusters render first within each section; ungrouped items follow.
	// A group spanning both sections clusters in each independently.
	const favParts = groupItems(favorites);
	const { groups, ungrouped } = groupItems(others);
	// Radar entries on unmanaged ports are adoptable listeners; entries tagged
	// with a managed item are port collisions, badged on that item's row.
	// `radarDevOnly` hides listeners with no recognized dev stack (DBs, caches,
	// system services); it also drops non-adoptable infra (Docker port proxies,
	// forced to stack "docker"). Port collisions stay badged regardless.
	const unmanaged = discovered.filter(
		(d) => d.managedItemId == null && (!radarDevOnly || (d.stack != null && d.adoptable)),
	);
	const conflicts = new Map(
		discovered.filter((d) => d.managedItemId != null).map((d) => [d.managedItemId as string, d]),
	);
	// Reordering only makes sense on the full, unfiltered list.
	const canReorder = query === '';

	const handleDrop = (key: string, to: number) => {
		if (drag && drag.group === key && drag.from !== to) {
			// The independent drag lists, in the order their members are persisted.
			// Built here (drop is a rare event) rather than every render.
			const dragLists = new Map<string, ManagedItem[]>([
				...favParts.groups.map((g) => [`fav-grp:${g.name}`, g.items] as const),
				['fav', favParts.ungrouped],
				...groups.map((g) => [`grp:${g.name}`, g.items] as const),
				['other', ungrouped],
			]);
			// Move within one list, then persist the full flattened order
			// (favorites, then each group cluster, then ungrouped).
			const flat = [...dragLists.keys()].flatMap((k) => {
				const list = dragLists.get(k) ?? [];
				return k === key ? moveInList(list, drag.from, to) : list;
			});
			void reorder(flat.map((i) => i.id)).then(onChange);
		}
		setDrag(null);
		setOverIdx(null);
	};

	/** Drag props for a row at `localIndex` within its drag list `group` (empty when reorder is off). */
	const dragProps = (group: string, localIndex: number) =>
		canReorder
			? {
					reorder: true,
					// Insertion line: below the target when moving down, above when moving up.
					dropLine:
						drag?.group === group && overIdx === localIndex && drag.from !== localIndex
							? drag.from < localIndex
								? ('bottom' as const)
								: ('top' as const)
							: null,
					onDragStart: (e: React.DragEvent) => {
						setDrag({ group, from: localIndex });
						e.dataTransfer.effectAllowed = 'move';
						e.dataTransfer.setData('text/plain', '');
					},
					onDragOver: (e: React.DragEvent) => {
						if (drag?.group === group) {
							e.preventDefault();
							setOverIdx(localIndex);
						}
					},
					onDrop: (e: React.DragEvent) => {
						e.preventDefault();
						handleDrop(group, localIndex);
					},
					onDragEnd: () => {
						setDrag(null);
						setOverIdx(null);
					},
				}
			: {};

	const handleStopAll = async () => {
		if (confirm('Stop all running services?')) {
			await stopAll();
			onChange();
		}
	};

	/** Alert the reasons of any rejected results (settled actions stay silent otherwise). */
	const surfaceFailures = (results: PromiseSettledResult<unknown>[]) => {
		const failed = results.filter((r): r is PromiseRejectedResult => r.status === 'rejected');
		if (failed.length > 0) alert(failed.map((f) => String(f.reason)).join('\n'));
	};

	/**
	 * Start every stopped/errored member. Docker members get one shared daemon
	 * check (prompt-then-start, matching single-row start); declining it skips
	 * them rather than failing the whole group. Failures are surfaced, not
	 * swallowed by allSettled.
	 */
	const startGroup = async (members: ManagedItem[]) => {
		let targets = members.filter((m) => statusOf(m) === 'stopped' || statusOf(m) === 'error');
		if (targets.some((m) => m.kind === 'docker') && !(await ensureDockerDaemon())) {
			targets = targets.filter((m) => m.kind !== 'docker');
		}
		surfaceFailures(await Promise.allSettled(targets.map((m) => startItem(m.id))));
		onChange();
	};

	/** Stop every running/starting member; failures are surfaced, then refresh. */
	const stopGroup = async (members: ManagedItem[]) => {
		const targets = members.filter((m) => statusOf(m) === 'running' || statusOf(m) === 'starting');
		surfaceFailures(await Promise.allSettled(targets.map((m) => stopItem(m.id))));
		onChange();
	};

	/** Render a section's group clusters followed by its ungrouped rows. */
	const renderClusters = (
		parts: { groups: { name: string; items: ManagedItem[] }[]; ungrouped: ManagedItem[] },
		keyPrefix: string,
		ungroupedKey: string,
		baseIndex: number,
	) => {
		// Running offsets keep row indices contiguous across clusters so the
		// entrance stagger cascades top-to-bottom instead of restarting per group.
		const offsets: number[] = [];
		let next = baseIndex;
		for (const g of parts.groups) {
			offsets.push(next);
			next += g.items.length;
		}
		return (
			<>
				{parts.groups.map((g, gi) => (
					<GroupRow
						key={g.name}
						name={g.name}
						count={g.items.length}
						status={aggregateGroupStatus(g.items.map(statusOf))}
						metrics={aggregateGroupMetrics(
							g.items.map((m) => metrics.get(m.id)).filter((m): m is ItemMetrics => m != null),
						)}
						onStart={() => void startGroup(g.items)}
						onStop={() => void stopGroup(g.items)}
					>
						{g.items.map((item, i) => renderRow(item, offsets[gi] + i, `${keyPrefix}${g.name}`, i))}
					</GroupRow>
				))}
				{parts.ungrouped.map((item, i) => renderRow(item, next + i, ungroupedKey, i))}
			</>
		);
	};

	const renderRow = (
		item: ManagedItem,
		index: number,
		group: string,
		localIndex: number,
	) => (
		<ServiceRow
			key={item.id}
			item={item}
			status={statusOf(item)}
			lastError={lastErrors.get(item.id)}
			metrics={metrics.get(item.id)}
			portConflict={statusOf(item) === 'stopped' ? conflicts.get(item.id) : undefined}
			index={index}
			open={expandedId === item.id}
			onOpenChange={(next) => setExpandedId(next ? item.id : null)}
			onChange={onChange}
			onEdit={onEdit}
			{...dragProps(group, localIndex)}
		/>
	);

	// ponytail: bg-background is fully opaque (no desktop bleed); add a /NN suffix to bring some vibrancy back.
	return (
		<div
			ref={shellRef}
			style={{ minHeight: POPOVER_MIN, maxHeight: 'var(--popover-max, 760px)' }}
			className="flex flex-col overflow-hidden rounded-xl border border-border/60 bg-background text-[13px] shadow-[inset_0_1px_0_0_rgba(255,255,255,0.10)]"
		>
			{/* Brand bar */}
			<header className="flex items-center gap-2 px-3.5 pt-3 pb-2">
				{searchOpen ? (
					<div className="relative flex min-w-0 flex-1 items-center">
						<Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
						<Input
							ref={searchRef}
							value={query}
							onChange={(e) => setQuery(e.target.value)}
							onKeyDown={(e) => e.key === 'Escape' && closeSearch()}
							placeholder="Search services…"
							className="h-8 rounded-md bg-muted/60 pl-8 text-[13px] shadow-none"
							aria-label="Search services"
						/>
					</div>
				) : (
					<>
						<BuoyMark className="size-6 shrink-0" />
						<div className="flex min-w-0 flex-1 items-baseline gap-1.5">
							<h1 className="truncate font-heading text-[14px] font-semibold tracking-tight">
								{__APP_NAME__}
							</h1>
							<span className="shrink-0 rounded-full bg-muted px-1.5 py-px font-mono text-[10px] font-medium text-muted-foreground tabular-nums">
								v{__APP_VERSION__}
							</span>
						</div>
					</>
				)}
				{searchOpen ? (
					<Button
						variant="ghost"
						size="icon-sm"
						onClick={closeSearch}
						aria-label="Close search"
						className="text-muted-foreground hover:text-foreground"
					>
						<X />
					</Button>
				) : (
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon-sm"
								onClick={() => setSearchOpen(true)}
								aria-label="Search services"
								className="text-muted-foreground hover:text-foreground"
							>
								<Search />
							</Button>
						</TooltipTrigger>
						<TooltipContent>Search</TooltipContent>
					</Tooltip>
				)}
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon-sm"
							onClick={handleStopAll}
							aria-label="Stop all services"
							className="text-muted-foreground hover:text-destructive"
						>
							<CirclePower />
						</Button>
					</TooltipTrigger>
					<TooltipContent>Stop all</TooltipContent>
				</Tooltip>
			</header>

			{/* List body */}
			<div className="scroll-area min-h-0 flex-1 px-2 pb-1">
				{filtered.length === 0 ? (
					<p className="px-2 py-8 text-center text-xs text-muted-foreground">
						{items.length === 0 ? 'No services yet. Add one below.' : 'No matches.'}
					</p>
				) : (
					<>
						{favorites.length > 0 && (
							<>
								<SectionLabel>Favorites</SectionLabel>
								{query
									? favorites.map((item, i) => renderRow(item, i, 'fav', i))
									: renderClusters(favParts, 'fav-grp:', 'fav', 0)}
							</>
						)}

						{others.length > 0 &&
							(query ? (
								others.map((item, i) => renderRow(item, favorites.length + i, 'other', i))
							) : (
								<Collapsible defaultOpen className="mt-0.5">
									<CollapsibleTrigger className="group/more flex w-full items-center gap-1 rounded-md px-2 py-1.5 font-heading text-[10px] font-semibold tracking-wider text-muted-foreground uppercase transition-colors hover:text-foreground">
										<ChevronRight className="size-3 transition-transform group-data-[state=open]/more:rotate-90" />
										More ({others.length})
									</CollapsibleTrigger>
									<CollapsibleContent>
										{renderClusters({ groups, ungrouped }, 'grp:', 'other', favorites.length)}
									</CollapsibleContent>
								</Collapsible>
							))}
					</>
				)}

				{/* Unmanaged listeners found by the port radar (not searched/reordered). */}
				{query === '' && unmanaged.length > 0 && (
					<Collapsible className="mt-0.5">
						<CollapsibleTrigger className="group/det flex w-full items-center gap-1 rounded-md px-2 py-1.5 font-heading text-[10px] font-semibold tracking-wider text-muted-foreground uppercase transition-colors hover:text-foreground">
							<ChevronRight className="size-3 transition-transform group-data-[state=open]/det:rotate-90" />
							Detected ({unmanaged.length})
						</CollapsibleTrigger>
						<CollapsibleContent>
							{unmanaged.map((entry) => (
								<DetectedRow
									key={`${entry.port}:${entry.pid}`}
									entry={entry}
									onAdopt={onAdopt}
									onChange={onChange}
									onDismiss={onDismissDiscovered}
								/>
							))}
						</CollapsibleContent>
					</Collapsible>
				)}
			</div>

			{/* Footer */}
			<footer className="flex items-center gap-1.5 border-t border-border/60 px-3 py-2">
				<Button variant="ghost" size="sm" onClick={onAdd} className="text-muted-foreground hover:text-foreground">
					<Plus />
					Add
				</Button>
				<Button variant="ghost" size="sm" onClick={onSettings} className="text-muted-foreground hover:text-foreground">
					<Settings />
					Settings
				</Button>
			</footer>
		</div>
	);
}

function SectionLabel({ children }: { children: React.ReactNode }): React.JSX.Element {
	return (
		<div className="px-2 pt-2 pb-1 font-heading text-[10px] font-semibold tracking-wider text-muted-foreground uppercase">
			{children}
		</div>
	);
}

/**
 * A group rendered like a service row: a tall status pill (signalling a
 * cluster), name, member count, aggregate metrics (Σ CPU · Σ memory · max
 * uptime), hover-revealed start-all / stop-all, and a chevron that expands the
 * member rows. Collapsed by default.
 */
function GroupRow({
	name,
	count,
	status,
	metrics,
	onStart,
	onStop,
	children,
}: {
	name: string;
	count: number;
	status: GroupStatus;
	metrics: { cpuPercent: number; memoryBytes: number; uptimeSec: number | null } | null;
	onStart: () => void;
	onStop: () => void;
	children: React.ReactNode;
}): React.JSX.Element {
	const [open, setOpen] = useState(false);
	// Faded green when only some members run; full accent otherwise.
	const accent = status === 'partial' ? 'bg-emerald-500/40' : STATUS_ACCENT[status];
	return (
		<Collapsible open={open} onOpenChange={setOpen}>
			<div className="group relative flex items-center gap-2 rounded-lg pr-1.5 pl-6 transition-colors hover:bg-foreground/[0.04] data-[state=open]:bg-foreground/[0.04]">
				<CollapsibleTrigger className="flex min-w-0 flex-1 items-center gap-2 py-1.5 text-left outline-none">
					{/* Tall pill (~2:1) reads as a cluster of services, not a single dot;
					    centered in an 8px slot so its column matches a normal row's dot. */}
					<span className="flex w-2 shrink-0 items-center justify-center">
						<span
							className={cn(
								'h-3 w-1.5 rounded-full',
								accent,
								status === 'starting' && 'animate-pulse',
							)}
						/>
					</span>
					{/* 14px slot so the chevron column lines up with the service-icon column. */}
					<span className="flex size-3.5 shrink-0 items-center justify-center">
						<ChevronRight
							className={cn(
								'size-3 text-muted-foreground transition-transform',
								open && 'rotate-90',
							)}
						/>
					</span>
					<span className="flex min-w-0 flex-col">
						<span className="truncate font-heading text-[13px] font-semibold leading-tight">
							{name}
						</span>
						<span className="flex items-center gap-1.5 font-mono text-[11px] leading-tight text-muted-foreground">
							<span>{count} services</span>
							{metrics && <MetricsText metrics={metrics} />}
						</span>
					</span>
				</CollapsibleTrigger>
				<div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100 group-data-[state=open]:opacity-100">
					<IconAction label={`Start all in ${name}`} onClick={onStart}>
						<Play />
					</IconAction>
					<IconAction label={`Stop all in ${name}`} onClick={onStop}>
						<Square />
					</IconAction>
				</div>
			</div>
			<CollapsibleContent>
				<div className="ml-2.5 border-l border-border/60 pl-1">{children}</div>
			</CollapsibleContent>
		</Collapsible>
	);
}
