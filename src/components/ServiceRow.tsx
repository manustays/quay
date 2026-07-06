import { useState } from 'react';
import {
	ArrowUpRight,
	FolderOpen,
	GripVertical,
	Pencil,
	Play,
	Square,
	SquareTerminal,
	Star,
	TriangleAlert,
	X,
} from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from '@/components/ui/collapsible';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';
import { ensureDockerDaemon } from '@/lib/docker';
import { RowIcon } from './StackIcon';
import { IconAction, MetricsText } from './RowBits';
import type { DiscoveredPort, ItemMetrics, ManagedItem, Status } from '../model';
import {
	markStopped,
	openBrowser,
	openTerminal,
	revealInFinder,
	startItem,
	stopItem,
	tailLog,
	toggleFavorite,
} from '../ipc';

interface ServiceRowProps {
	item: ManagedItem;
	status: Status;
	lastError: string | undefined;
	metrics: ItemMetrics | undefined;
	/** Set when this stopped item's port is occupied by a foreign process. */
	portConflict?: DiscoveredPort;
	index: number;
	/** Expansion is owned by the parent so only one row is open at a time. */
	open: boolean;
	onOpenChange: (next: boolean) => void;
	onChange: () => void;
	onEdit: (item: ManagedItem) => void;
	/** When true, show a drag handle and wire the row as a drag source/target. */
	reorder?: boolean;
	/** Draw an insertion line above/below this row to show where the drop lands. */
	dropLine?: 'top' | 'bottom' | null;
	onDragStart?: React.DragEventHandler;
	onDragOver?: React.DragEventHandler;
	onDrop?: React.DragEventHandler;
	onDragEnd?: () => void;
}

/** Per-status color for the status dot. */
export const STATUS_ACCENT: Record<Status, string> = {
	running: 'bg-emerald-500',
	starting: 'bg-amber-500',
	stopped: 'bg-zinc-400/70 dark:bg-zinc-500',
	error: 'bg-red-500',
};

/** Short descriptor shown after the port (kept honest to the data we have). */
function descriptor(item: ManagedItem): string {
	if (item.kind === 'brew') return 'brew';
	if (item.kind === 'docker') return 'docker';
	if (item.kind === 'cli') return 'cli';
	if (item.kind === 'command') return 'command';
	return item.runMode;
}

/**
 * A single service row. Clicking the name area expands a panel (log tail +
 * edit/favorite/delete); action buttons live outside the collapsible trigger so
 * they never toggle expansion, and reveal on hover or keyboard focus.
 */
export function ServiceRow({
	item,
	status,
	lastError,
	metrics,
	portConflict,
	index,
	open,
	onOpenChange,
	onChange,
	onEdit,
	reorder = false,
	dropLine = null,
	onDragStart,
	onDragOver,
	onDrop,
	onDragEnd,
}: ServiceRowProps): React.JSX.Element {
	const [log, setLog] = useState<string>('');
	const [copied, setCopied] = useState(false);
	// Gate `draggable` on the handle so only the grip starts a drag, not the whole row.
	const [grabbing, setGrabbing] = useState(false);
	const running = status === 'running' || status === 'starting';

	const handleOpenChange = async (next: boolean) => {
		onOpenChange(next);
		if (next) setLog(await tailLog(item.id, 20).catch(() => ''));
	};

	/** Copy the service URL and flash the "copied!" label. */
	const copyUrl = () => {
		void navigator.clipboard.writeText(`http://localhost:${item.port}`);
		setCopied(true);
		setTimeout(() => setCopied(false), 1000);
	};

	/** Run an ipc action, surface errors, then refresh. */
	const act = (fn: () => Promise<unknown>) => async (e: React.MouseEvent) => {
		e.stopPropagation();
		try {
			await fn();
		} catch (err) {
			alert(String(err));
		}
		onChange();
	};

	/**
	 * Toggle the service. Docker starts first ensure the daemon is up (prompt then
	 * launch Docker Desktop); the `DOCKER_DAEMON_DOWN` backend sentinel is a fallback
	 * if the daemon dropped between the check and the call.
	 */
	const startOrStop = async () => {
		if (running) return stopItem(item.id);
		if (item.kind === 'docker') {
			if (!(await ensureDockerDaemon())) return;
			try {
				return await startItem(item.id);
			} catch (err) {
				if (String(err).includes('DOCKER_DAEMON_DOWN') && (await ensureDockerDaemon())) {
					return startItem(item.id);
				}
				throw err;
			}
		}
		return startItem(item.id);
	};

	return (
		<div
			draggable={reorder && grabbing}
			onDragStart={onDragStart}
			onDragOver={onDragOver}
			onDrop={(e) => {
				onDrop?.(e);
				setGrabbing(false);
			}}
			onDragEnd={() => {
				onDragEnd?.();
				setGrabbing(false);
			}}
			className="relative rounded-lg"
		>
			{dropLine && (
				<span
					className={cn(
						'pointer-events-none absolute inset-x-1 z-10 h-0.5 rounded-full bg-primary',
						dropLine === 'top' ? '-top-px' : '-bottom-px',
					)}
				/>
			)}
			<Collapsible
				open={open}
				onOpenChange={handleOpenChange}
				className="row-in"
				style={{ animationDelay: `${Math.min(index, 8) * 28}ms` }}
			>
			<div className={cn(
				'group relative flex items-center gap-2 rounded-lg pr-1.5 transition-colors hover:bg-foreground/[0.04] data-[state=open]:bg-foreground/[0.04]',
				reorder ? 'pl-6' : 'pl-3',
			)}>
				{/* Drag handle — only the grip initiates a drag */}
				{reorder && (
					<button
						type="button"
						aria-label="Drag to reorder"
						onMouseDown={() => setGrabbing(true)}
						onMouseUp={() => setGrabbing(false)}
						className="absolute top-1/2 left-0.5 flex -translate-y-1/2 cursor-grab items-center text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 active:cursor-grabbing"
					>
						<GripVertical className="size-3.5" />
					</button>
				)}
				{/* Name + meta — the only click target that toggles expansion */}
				<CollapsibleTrigger
					className="flex min-w-0 flex-1 items-center gap-2 py-1.5 text-left outline-none"
					title={status === 'error' ? lastError : undefined}
				>
					<span
						className={cn(
							'size-2 shrink-0 rounded-full',
							STATUS_ACCENT[status],
							status === 'starting' && 'animate-pulse',
						)}
					/>
					<RowIcon stack={item.stack} kind={item.kind} runMode={item.runMode} />
					<span className="flex min-w-0 flex-col">
						<span className="truncate font-heading text-[13px] font-semibold leading-tight">
							{item.name}
						</span>
						<span className="flex items-center gap-1.5 font-mono leading-tight text-muted-foreground">
							{item.port != null && (
								// Not a <button>: this sits inside the CollapsibleTrigger button,
								// so it carries its own tabIndex + key handling to stay reachable
								// by keyboard despite the nesting.
								<span
									role="button"
									tabIndex={0}
									title={`Copy http://localhost:${item.port}`}
									onClick={(e) => {
										e.stopPropagation();
										copyUrl();
									}}
									onKeyDown={(e) => {
										if (e.key === 'Enter' || e.key === ' ') {
											e.preventDefault();
											e.stopPropagation();
											copyUrl();
										}
									}}
									className="font-mono text-[11px] hover:text-foreground"
								>
									{copied ? 'copied!' : `:${item.port}`}
								</span>
							)}
							{portConflict && (
								<Tooltip>
									<TooltipTrigger asChild>
										<TriangleAlert className="size-3 text-amber-500" />
									</TooltipTrigger>
									<TooltipContent>
										Port {portConflict.port} is in use by {portConflict.name} (pid {portConflict.pid})
									</TooltipContent>
								</Tooltip>
							)}
							<span className="text-[11px]">{descriptor(item)}</span>
							{running && metrics && (
								<MetricsText metrics={metrics} className="font-mono text-[11px]" />
							)}
						</span>
					</span>
				</CollapsibleTrigger>

				{/* Actions — hidden until row hover / keyboard focus */}
				<div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100 group-data-[state=open]:opacity-100">
					<IconAction
						label={running ? 'Stop' : 'Start'}
						onClick={act(startOrStop)}
					>
						{running ? <Square /> : <Play />}
					</IconAction>
					{status === 'error' && (
						<IconAction label="Mark stopped" onClick={act(() => markStopped(item.id))}>
							<X />
						</IconAction>
					)}
					{item.port != null && status === 'running' && (
						<IconAction label="Open in browser" onClick={act(() => openBrowser(item.id))}>
							<ArrowUpRight />
						</IconAction>
					)}
				</div>
			</div>

			<CollapsibleContent>
				<div className="mx-1 mt-1 mb-1.5 flex flex-col gap-2 rounded-lg bg-foreground/[0.04] p-2">
					<pre className="scroll-area max-h-32 overflow-auto rounded-md bg-background/60 p-2 font-mono text-[10px] leading-relaxed whitespace-pre-wrap text-muted-foreground">
						{log || '(no log)'}
					</pre>
					<div className="flex items-center gap-1.5">
						<Button variant="outline" size="xs" onClick={(e) => { e.stopPropagation(); onEdit(item); }}>
							<Pencil />
							Edit
						</Button>
						<Button variant="outline" size="xs" onClick={act(() => toggleFavorite(item.id))}>
							<Star className={cn('size-3', item.favorite && 'fill-amber-400 text-amber-400')} />
							{item.favorite ? 'Unfavorite' : 'Favorite'}
						</Button>
						{item.dir && (
							<Button variant="outline" size="xs" onClick={act(() => revealInFinder(item.id))}>
								<FolderOpen />
								Reveal
							</Button>
						)}
						{item.dir && (
							<Button variant="outline" size="xs" onClick={act(() => openTerminal(item.id))}>
								<SquareTerminal />
								Terminal
							</Button>
						)}
						{status === 'error' && (
							<Button variant="outline" size="xs" onClick={act(() => markStopped(item.id))}>
								<Square />
								Mark stopped
							</Button>
						)}
					</div>
				</div>
			</CollapsibleContent>
			</Collapsible>
		</div>
	);
}
