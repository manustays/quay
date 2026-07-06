import type { DiscoveredAgent } from '../model';
import { cn } from '@/lib/utils';
import { RowIcon } from './StackIcon';
import { MetricsText } from './RowBits';

interface AgentRowProps {
	entry: DiscoveredAgent;
}

/**
 * A read-only row for a terminal agent session found by the agent radar.
 * Leading dot shows the activity state: solid pulsing green = active (recent
 * session-log write or busy CPU), hollow muted = idle. Dimmed like the
 * Detected rows since these sessions are observed, not managed.
 */
export function AgentRow({ entry }: AgentRowProps): React.JSX.Element {
	const active = entry.state === 'active';
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
		</div>
	);
}
