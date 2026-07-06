import { EyeOff, FolderOpen, Square } from 'lucide-react';
import type { DiscoveredAgent } from '../model';
import { ignoreAgent, killAgent, revealPath } from '../ipc';
import { cn } from '@/lib/utils';
import { RowIcon } from './StackIcon';
import { IconAction, MetricsText } from './RowBits';

interface AgentRowProps {
	entry: DiscoveredAgent;
	/** Remove this entry from the list now — the next radar scan is up to 5 s away. */
	onDismiss: (entry: DiscoveredAgent) => void;
}

/**
 * A read-only row for a terminal agent session found by the agent radar.
 * Leading dot shows the activity state: solid pulsing green = active (recent
 * session-log write or busy CPU), hollow muted = idle. Dimmed like the
 * Detected rows since these sessions are observed, not managed. Hover reveals
 * Reveal / Kill / Ignore.
 */
export function AgentRow({ entry, onDismiss }: AgentRowProps): React.JSX.Element {
	const active = entry.state === 'active';

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
			<span
				title={active ? 'recent activity' : 'idle'}
				className={cn(
					'size-2 shrink-0 rounded-full',
					active
						? 'animate-pulse bg-emerald-500'
						: 'border border-muted-foreground/60',
				)}
			/>
			<RowIcon stack={entry.agent} />
			<span className="flex min-w-0 flex-1 flex-col">
				<span className="truncate font-heading text-[13px] font-semibold leading-tight">
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
