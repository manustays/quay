import { useState } from 'react';
import { ChevronRight, CirclePower, Play, Plus, Search, Settings, Square } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from '@/components/ui/collapsible';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { aggregateGroupStatus, groupItems, matchesSearch, moveInList, splitFavorites, type DiscoveredPort, type ItemMetrics, type ManagedItem, type Status } from '../model';
import { cn } from '@/lib/utils';
import { reorder, startItem, stopAll, stopItem } from '../ipc';
import { BuoyMark } from './BuoyMark';
import { DetectedRow } from './DetectedRow';
import { ServiceRow, STATUS_ACCENT } from './ServiceRow';

interface PopupProps {
	items: ManagedItem[];
	statuses: Map<string, Status>;
	lastErrors: Map<string, string>;
	metrics: Map<string, ItemMetrics>;
	discovered: DiscoveredPort[];
	onChange: () => void;
	onAdd: () => void;
	onEdit: (item: ManagedItem) => void;
	onAdopt: (entry: DiscoveredPort) => void;
	onSettings: () => void;
}

/** The full popover shell: brand bar, search toolbar, scrolling list, footer. */
export function Popup({
	items,
	statuses,
	lastErrors,
	metrics,
	discovered,
	onChange,
	onAdd,
	onEdit,
	onAdopt,
	onSettings,
}: PopupProps): React.JSX.Element {
	const [query, setQuery] = useState('');
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
	const unmanaged = discovered.filter((d) => d.managedItemId == null);
	const conflicts = new Map(
		discovered.filter((d) => d.managedItemId != null).map((d) => [d.managedItemId as string, d]),
	);
	// Reordering only makes sense on the full, unfiltered list.
	const canReorder = query === '';

	/** The independent drag lists, in the order their members are persisted. */
	const dragLists = new Map<string, ManagedItem[]>([
		...favParts.groups.map((g) => [`fav-grp:${g.name}`, g.items] as const),
		['fav', favParts.ungrouped],
		...groups.map((g) => [`grp:${g.name}`, g.items] as const),
		['other', ungrouped],
	]);

	const handleDrop = (key: string, to: number) => {
		if (drag && drag.group === key && drag.from !== to) {
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

	/** Start every stopped/errored member; refresh when all have settled. */
	const startGroup = (members: ManagedItem[]) =>
		void Promise.allSettled(
			members.filter((m) => statusOf(m) === 'stopped' || statusOf(m) === 'error')
				.map((m) => startItem(m.id)),
		).then(onChange);

	/** Stop every running/starting member; refresh when all have settled. */
	const stopGroup = (members: ManagedItem[]) =>
		void Promise.allSettled(
			members.filter((m) => statusOf(m) === 'running' || statusOf(m) === 'starting')
				.map((m) => stopItem(m.id)),
		).then(onChange);

	/** Render a section's group clusters followed by its ungrouped rows. */
	const renderClusters = (
		parts: { groups: { name: string; items: ManagedItem[] }[]; ungrouped: ManagedItem[] },
		keyPrefix: string,
		ungroupedKey: string,
		baseIndex: number,
	) => (
		<>
			{parts.groups.map((g) => (
				<div key={g.name}>
					<GroupHeader
						name={g.name}
						status={aggregateGroupStatus(g.items.map(statusOf))}
						onStart={() => startGroup(g.items)}
						onStop={() => stopGroup(g.items)}
					/>
					{g.items.map((item, i) => renderRow(item, baseIndex + i, `${keyPrefix}${g.name}`, i))}
				</div>
			))}
			{parts.ungrouped.map((item, i) => renderRow(item, baseIndex + i, ungroupedKey, i))}
		</>
	);

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
			onChange={onChange}
			onEdit={onEdit}
			{...dragProps(group, localIndex)}
		/>
	);

	return (
		<div className="flex h-screen flex-col overflow-hidden rounded-xl border border-border/60 bg-background/55 text-[13px] backdrop-saturate-150">
			{/* Brand bar */}
			<header className="flex items-center gap-2 px-3.5 pt-3 pb-2">
				<BuoyMark className="size-6 shrink-0" />
				<div className="flex min-w-0 flex-1 items-baseline gap-1.5">
					<h1 className="truncate font-heading text-[14px] font-semibold tracking-tight">
						{__APP_NAME__}
					</h1>
					<span className="shrink-0 rounded-full bg-muted px-1.5 py-px font-mono text-[10px] font-medium text-muted-foreground tabular-nums">
						v{__APP_VERSION__}
					</span>
				</div>
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

			{/* Search toolbar */}
			<div className="px-3.5 pb-2">
				<div className="relative">
					<Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
					<Input
						value={query}
						onChange={(e) => setQuery(e.target.value)}
						placeholder="Search services…"
						className="h-8 rounded-lg bg-muted/60 pl-8 text-[13px] shadow-none"
						aria-label="Search services"
					/>
				</div>
			</div>

			{/* List body */}
			<div className="scroll-area flex-1 px-2 pb-1">
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
 * Sub-header for a group cluster: aggregate status dot, group name, and
 * hover-revealed start-all / stop-all actions.
 */
function GroupHeader({
	name,
	status,
	onStart,
	onStop,
}: {
	name: string;
	status: Status;
	onStart: () => void;
	onStop: () => void;
}): React.JSX.Element {
	return (
		<div className="group/hdr flex items-center gap-1.5 rounded-md px-2 pt-1.5 pb-0.5">
			<span className={cn('size-1.5 shrink-0 rounded-full', STATUS_ACCENT[status])} />
			<span className="flex-1 truncate font-heading text-[10px] font-semibold tracking-wider text-muted-foreground uppercase">
				{name}
			</span>
			<div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover/hdr:opacity-100 focus-within:opacity-100">
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon-xs"
							onClick={onStart}
							aria-label={`Start all in ${name}`}
							className="text-muted-foreground hover:text-foreground"
						>
							<Play />
						</Button>
					</TooltipTrigger>
					<TooltipContent>Start all</TooltipContent>
				</Tooltip>
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon-xs"
							onClick={onStop}
							aria-label={`Stop all in ${name}`}
							className="text-muted-foreground hover:text-destructive"
						>
							<Square />
						</Button>
					</TooltipTrigger>
					<TooltipContent>Stop all</TooltipContent>
				</Tooltip>
			</div>
		</div>
	);
}
