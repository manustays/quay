import { useState } from 'react';
import { ChevronRight, EyeOff, FolderOpen, Square } from 'lucide-react';
import { aggregateGroupMetrics, type AgentFolder, type DiscoveredAgent } from '../model';
import { ignoreAgent, killAgent, revealPath } from '../ipc';
import { cn } from '@/lib/utils';
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from '@/components/ui/collapsible';
import { RowIcon } from './StackIcon';
import { IconAction, MetricsText } from './RowBits';

interface AgentRowProps {
	entry: DiscoveredAgent;
	/** Remove this entry from the list now — the next radar scan is up to 5 s away. */
	onDismiss: (entry: DiscoveredAgent) => void;
}

/** The active/idle dot shared by session rows and folder rows. */
function StateDot({ active }: { active: boolean }): React.JSX.Element {
	return (
		<span
			title={active ? 'recent activity' : 'idle'}
			className={cn(
				'size-2 shrink-0 rounded-full',
				active ? 'animate-pulse bg-emerald-500' : 'border border-muted-foreground/60',
			)}
		/>
	);
}

/**
 * A read-only row for a terminal agent session found by the agent radar.
 * Leading dot shows the activity state: solid pulsing green = active (recent
 * session-log write or busy CPU), hollow muted = idle. Hovering the name
 * shows the session's best-effort label. Dimmed like the Detected rows since
 * these sessions are observed, not managed. Hover reveals Reveal / Kill /
 * Ignore.
 */
export function AgentRow({ entry, onDismiss }: AgentRowProps): React.JSX.Element {
	/**
	 * Run an ipc action, surface errors, and only dismiss the row on success —
	 * a failed kill must leave the row visible with an alert, never a silent
	 * success. (Reveal succeeding shouldn't dismiss; see below.)
	 */
	const act = (fn: () => Promise<unknown>, dismiss = true) => async (e: React.MouseEvent) => {
		e.stopPropagation();
		try {
			await fn();
			if (dismiss) onDismiss(entry);
		} catch (err) {
			alert(String(err));
		}
	};

	return (
		<div className="group relative flex items-center gap-2 rounded-lg py-1.5 pr-1.5 pl-6 opacity-75 transition-colors hover:bg-foreground/[0.04] hover:opacity-100">
			<StateDot active={entry.state === 'active'} />
			<RowIcon stack={entry.agent} />
			<span className="flex min-w-0 flex-1 flex-col">
				<span
					className="truncate font-heading text-[13px] font-semibold leading-tight"
					title={entry.sessionName ?? entry.cwd}
				>
					{entry.name}
				</span>
				<span className="flex items-center gap-1.5 truncate font-mono text-[11px] leading-tight text-muted-foreground">
					<MetricsText
						metrics={{
							cpuPercent: entry.cpuPercent,
							memoryBytes: entry.memoryBytes,
							uptimeSec: entry.uptimeSec,
						}}
						className="shrink-0"
					/>
					<span className="truncate" title={entry.cwd}>{entry.cwd}</span>
				</span>
			</span>

			<div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
				<IconAction label="Reveal in Finder" onClick={act(() => revealPath(entry.cwd), false)}>
					<FolderOpen />
				</IconAction>
				<IconAction
					label="Kill session (⌥ = force)"
					onClick={(e) =>
						void act(() => killAgent(entry.pid, entry.agent, entry.cwd, e.altKey))(e)
					}
				>
					<Square />
				</IconAction>
				<IconAction
					label="Ignore this agent here (hides all its sessions in this folder)"
					onClick={act(() => ignoreAgent(entry.agent, entry.cwd))}
				>
					<EyeOff />
				</IconAction>
			</div>
		</div>
	);
}

/** How many stacked agent badges a folder row shows before collapsing to +N. */
const MAX_BADGES = 4;

/**
 * A project folder clubbing 2+ agent sessions in the same cwd. Collapsed it
 * reads as the project (stack icon, manifest name, aggregate metrics) with a
 * stack of overlapping agent badges on the right — one per session, dimmed
 * when idle, session name in the tooltip. Expanded it lists the member
 * session rows.
 */
export function AgentFolderRow({
	folder,
	onDismiss,
}: {
	folder: AgentFolder;
	onDismiss: (entry: DiscoveredAgent) => void;
}): React.JSX.Element {
	const [open, setOpen] = useState(false);
	const anyActive = folder.agents.some((a) => a.state === 'active');
	const metrics = aggregateGroupMetrics(
		folder.agents.map((a) => ({
			id: String(a.pid),
			cpuPercent: a.cpuPercent,
			memoryBytes: a.memoryBytes,
			uptimeSec: a.uptimeSec,
		})),
	);
	return (
		<Collapsible open={open} onOpenChange={setOpen}>
			<div className="group relative flex items-center gap-2 rounded-lg py-1.5 pr-1.5 pl-6 opacity-75 transition-colors hover:bg-foreground/[0.04] hover:opacity-100 data-[state=open]:bg-foreground/[0.04] data-[state=open]:opacity-100">
				<CollapsibleTrigger className="flex min-w-0 flex-1 items-center gap-2 text-left outline-none">
					<StateDot active={anyActive} />
					{/* 14px slot keeps the chevron aligned with the icon column. */}
					<span className="flex size-3.5 shrink-0 items-center justify-center">
						<ChevronRight
							className={cn(
								'size-3 text-muted-foreground transition-transform',
								open && 'rotate-90',
							)}
						/>
					</span>
					<RowIcon stack={folder.stack} />
					<span className="flex min-w-0 flex-col">
						<span
							className="truncate font-heading text-[13px] font-semibold leading-tight"
							title={folder.cwd}
						>
							{folder.name}
						</span>
						<span className="flex items-center gap-1.5 font-mono text-[11px] leading-tight text-muted-foreground">
							<span>{folder.agents.length} sessions</span>
							{metrics && <MetricsText metrics={{ ...metrics, uptimeSec: null }} />}
						</span>
					</span>
				</CollapsibleTrigger>
				{/* Stacked agent badges: which agents are in here, and their state. */}
				<div className="flex shrink-0 items-center -space-x-1.5">
					{folder.agents.slice(0, MAX_BADGES).map((a) => (
						<span
							key={a.pid}
							title={a.sessionName ?? `${a.agent} · pid ${a.pid}`}
							className={cn(
								'flex size-5 items-center justify-center rounded-full bg-muted ring-1 ring-background',
								a.state === 'idle' && 'opacity-50',
							)}
						>
							<RowIcon stack={a.agent} />
						</span>
					))}
					{folder.agents.length > MAX_BADGES && (
						<span className="flex size-5 items-center justify-center rounded-full bg-muted ring-1 ring-background font-mono text-[9px] text-muted-foreground">
							+{folder.agents.length - MAX_BADGES}
						</span>
					)}
				</div>
			</div>
			<CollapsibleContent>
				<div className="ml-2.5 border-l border-border/60 pl-1">
					{folder.agents.map((a) => (
						<AgentRow key={a.pid} entry={a} onDismiss={onDismiss} />
					))}
				</div>
			</CollapsibleContent>
		</Collapsible>
	);
}
