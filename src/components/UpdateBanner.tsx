import { useState } from 'react';
import { ArrowUpCircle, ChevronRight, Loader2, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from '@/components/ui/collapsible';
import { cn } from '@/lib/utils';
import { installUpdate, openReleases } from '../ipc';
import type { UpdateInfo } from '../model';

interface UpdateBannerProps {
	info: UpdateInfo;
	/** Dismiss the banner for this version (per session). */
	onDismiss: () => void;
}

/**
 * In-app "update available" banner. Shows the new version, an expandable changelog
 * (only when the release carried notes), a "Changelog" link to the full GitHub
 * releases page, an Install button that downloads + restarts via
 * {@link installUpdate}, and a dismiss control. Release notes are rendered as
 * plain text — never markdown/HTML — so a malformed or hostile changelog can't inject
 * markup.
 */
export function UpdateBanner({ info, onDismiss }: UpdateBannerProps): React.JSX.Element {
	const [installing, setInstalling] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const hasNotes = info.notes.trim().length > 0;

	async function handleInstall(): Promise<void> {
		setInstalling(true);
		setError(null);
		try {
			// Resolves only if the remote is no longer newer; on a real install the
			// app restarts and this never returns.
			await installUpdate();
			onDismiss();
		} catch (e) {
			setError(String(e));
			setInstalling(false);
		}
	}

	return (
		<div className="mx-2 mt-1 mb-1 rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-[12px]">
			<div className="flex items-center gap-2">
				<ArrowUpCircle className="size-4 shrink-0 text-primary" />
				<span className="min-w-0 flex-1 truncate">
					<span className="font-semibold">Quay v{info.version}</span>{' '}
					<span className="text-muted-foreground">available</span>
				</span>
				<Button
					size="sm"
					className="h-6 px-2 text-[11px]"
					onClick={() => void handleInstall()}
					disabled={installing}
				>
					{installing ? <Loader2 className="size-3 animate-spin" /> : null}
					{installing ? 'Installing…' : 'Install & Restart'}
				</Button>
				<Button
					variant="ghost"
					size="icon-sm"
					onClick={onDismiss}
					disabled={installing}
					aria-label="Dismiss update"
					className="text-muted-foreground hover:text-foreground"
				>
					<X />
				</Button>
			</div>

			{hasNotes && (
				<Collapsible className="mt-1">
					<CollapsibleTrigger className="group flex items-center gap-1 text-[11px] text-muted-foreground hover:text-foreground">
						<ChevronRight className="size-3 transition-transform group-data-[state=open]:rotate-90" />
						What's new
					</CollapsibleTrigger>
					<CollapsibleContent>
						<p className="mt-1 max-h-40 overflow-y-auto whitespace-pre-wrap pr-1 text-[11px] leading-snug text-muted-foreground">
							{info.notes}
						</p>
					</CollapsibleContent>
				</Collapsible>
			)}

			<button
				type="button"
				onClick={() => void openReleases()}
				className="mt-1 text-[11px] text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
			>
				Changelog
			</button>

			{error && (
				<p className={cn('mt-1 text-[11px] text-destructive')}>{error}</p>
			)}
		</div>
	);
}
