import { EyeOff, Plus, Square } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import type { DiscoveredPort } from '../model';
import { ignorePort, killDiscovered } from '../ipc';
import { StackIcon } from './StackIcon';

interface DetectedRowProps {
	entry: DiscoveredPort;
	/** Open the add-service form prefilled from this listener. */
	onAdopt: (entry: DiscoveredPort) => void;
	onChange: () => void;
}

/**
 * A read-only row for an unmanaged listener found by the port radar. Dimmed
 * relative to registered services; hover reveals Adopt / Kill / Ignore.
 */
export function DetectedRow({ entry, onAdopt, onChange }: DetectedRowProps): React.JSX.Element {
	// Docker published ports belong to Docker Desktop's proxy, not a project —
	// manage those via a Docker-kind service instead of adopting the proxy.
	const adoptable = entry.stack !== 'docker';

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

	return (
		<div className="group relative flex items-center gap-2 rounded-lg py-1.5 pr-1.5 pl-3 opacity-75 transition-colors hover:bg-foreground/[0.04] hover:opacity-100">
			<span className="size-2 shrink-0 rounded-full border border-dashed border-muted-foreground/60" />
			<StackIcon stack={entry.stack} />
			<span className="flex min-w-0 flex-1 flex-col">
				<span className="truncate font-heading text-[13px] font-semibold leading-tight">
					{entry.name}
				</span>
				<span className="flex items-center gap-1.5 truncate font-mono text-[11px] leading-tight text-muted-foreground">
					<span>:{entry.port}</span>
					<span className="truncate" title={entry.command}>{entry.command}</span>
				</span>
			</span>

			<div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
				{adoptable && (
					<DetectedAction label="Adopt as service" onClick={(e) => { e.stopPropagation(); onAdopt(entry); }}>
						<Plus />
					</DetectedAction>
				)}
				<DetectedAction
					label="Kill process (⌥ = force)"
					onClick={(e) =>
						void act(() => killDiscovered(entry.pid, entry.port, e.altKey))(e)
					}
				>
					<Square />
				</DetectedAction>
				<DetectedAction label="Ignore this port" onClick={act(() => ignorePort(entry.port))}>
					<EyeOff />
				</DetectedAction>
			</div>
		</div>
	);
}

/** Small icon button used for detected-row actions, wrapped in a tooltip. */
function DetectedAction({
	label,
	onClick,
	children,
}: {
	label: string;
	onClick: (e: React.MouseEvent) => void;
	children: React.ReactNode;
}): React.JSX.Element {
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Button
					variant="ghost"
					size="icon-xs"
					onClick={onClick}
					aria-label={label}
					className="text-muted-foreground hover:text-foreground focus-visible:opacity-100"
				>
					{children}
				</Button>
			</TooltipTrigger>
			<TooltipContent>{label}</TooltipContent>
		</Tooltip>
	);
}
