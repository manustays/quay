import { Button } from '@/components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';
import { formatBytes, formatUptime } from '../model';

/** A small ghost icon button with a tooltip — the shared row/group/detected action. */
export function IconAction({
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

/** The shared `cpu% · mem[ · uptime]` metrics line for service and group rows. */
export function MetricsText({
	metrics,
	className,
}: {
	/** `cpuPercent` is omitted for agent rows: sampling it costs a second process
	 *  refresh and a 200 ms stall, which is not worth a number on a hidden popover. */
	metrics: { cpuPercent?: number; memoryBytes: number; uptimeSec: number | null };
	className?: string;
}): React.JSX.Element {
	return (
		<span className={cn('tabular-nums', className)}>
			{metrics.cpuPercent != null && `${metrics.cpuPercent.toFixed(0)}% · `}
			{formatBytes(metrics.memoryBytes)}
			{metrics.uptimeSec != null && ` · ${formatUptime(metrics.uptimeSec)}`}
		</span>
	);
}
