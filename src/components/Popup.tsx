import { useState } from 'react';
import { ChevronRight, CirclePower, Plus, Search, Settings } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from '@/components/ui/collapsible';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { matchesSearch, splitFavorites, type ItemMetrics, type ManagedItem, type Status } from '../model';
import { stopAll } from '../ipc';
import { BuoyMark } from './BuoyMark';
import { ServiceRow } from './ServiceRow';

interface PopupProps {
	items: ManagedItem[];
	statuses: Map<string, Status>;
	lastErrors: Map<string, string>;
	metrics: Map<string, ItemMetrics>;
	onChange: () => void;
	onAdd: () => void;
	onEdit: (item: ManagedItem) => void;
	onSettings: () => void;
}

/** The full popover shell: brand bar, search toolbar, scrolling list, footer. */
export function Popup({
	items,
	statuses,
	lastErrors,
	metrics,
	onChange,
	onAdd,
	onEdit,
	onSettings,
}: PopupProps): React.JSX.Element {
	const [query, setQuery] = useState('');
	const statusOf = (i: ManagedItem): Status => statuses.get(i.id) ?? 'stopped';

	const filtered = items.filter((i) => matchesSearch(i, query));
	const { favorites, others } = splitFavorites(filtered);

	const handleStopAll = async () => {
		if (confirm('Stop all running services?')) {
			await stopAll();
			onChange();
		}
	};

	const renderRow = (item: ManagedItem, index: number) => (
		<ServiceRow
			key={item.id}
			item={item}
			status={statusOf(item)}
			lastError={lastErrors.get(item.id)}
			metrics={metrics.get(item.id)}
			index={index}
			onChange={onChange}
			onEdit={onEdit}
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
								{favorites.map(renderRow)}
							</>
						)}

						{others.length > 0 &&
							(query ? (
								others.map((item, i) => renderRow(item, favorites.length + i))
							) : (
								<Collapsible defaultOpen className="mt-0.5">
									<CollapsibleTrigger className="group/more flex w-full items-center gap-1 rounded-md px-2 py-1.5 font-heading text-[10px] font-semibold tracking-wider text-muted-foreground uppercase transition-colors hover:text-foreground">
										<ChevronRight className="size-3 transition-transform group-data-[state=open]/more:rotate-90" />
										More ({others.length})
									</CollapsibleTrigger>
									<CollapsibleContent>
										{others.map((item, i) => renderRow(item, favorites.length + i))}
									</CollapsibleContent>
								</Collapsible>
							))}
					</>
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
