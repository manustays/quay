import { matchesSearch, splitFavorites, type ManagedItem, type Status } from './model';
import { renderRow } from './row';
import { stopAll } from './ipc';

/** Options passed to {@link renderList} to wire up external actions. */
interface ListOpts {
	onChange: () => void;
	onAdd: () => void;
	onSettings: () => void;
}

/**
 * Render the full two-tier list into `container`.
 *
 * Builds a header (search box + Stop-all button), a scrollable body with a
 * FAVORITES section and a collapsible "More (n)" group, and a footer with
 * "+ Add" and "⚙ Settings" buttons.
 *
 * @param container - The root element to render into (cleared on each call).
 * @param items - Full list of managed items from the backend.
 * @param statuses - Live status map keyed by item id.
 * @param lastErrors - Last error message map keyed by item id.
 * @param opts - Callbacks for mutations, add-item, and open-settings.
 */
export function renderList(
	container: HTMLElement,
	items: ManagedItem[],
	statuses: Map<string, Status>,
	lastErrors: Map<string, string>,
	opts: ListOpts,
): void {
	container.replaceChildren();

	// --- Header ---
	const header = document.createElement('div');
	header.className = 'header';

	const search = document.createElement('input');
	search.placeholder = 'Search…';
	search.className = 'search';

	const stop = document.createElement('button');
	stop.textContent = '■ Stop all';
	stop.onclick = async () => {
		if (confirm('Stop all running services?')) {
			await stopAll();
			opts.onChange();
		}
	};

	header.append(search, stop);
	container.appendChild(header);

	// --- Body ---
	const body = document.createElement('div');
	body.className = 'body';
	container.appendChild(body);

	/** Resolve current status for an item, defaulting to 'stopped'. */
	const statusOf = (i: ManagedItem): Status => statuses.get(i.id) ?? 'stopped';

	/** Re-draw the body based on current search query. */
	const draw = (): void => {
		body.replaceChildren();
		const q = (search as HTMLInputElement).value;
		const filtered = items.filter(i => matchesSearch(i, q));
		const { favorites, others } = splitFavorites(filtered);

		if (favorites.length) {
			const sectionHeader = document.createElement('div');
			sectionHeader.className = 'section';
			sectionHeader.textContent = 'FAVORITES';
			body.appendChild(sectionHeader);
			favorites.forEach(i => body.appendChild(
				renderRow(i, statusOf(i), lastErrors.get(i.id), opts.onChange),
			));
		}

		if (others.length) {
			if (q) {
				// When searching, show all matches flat (no collapse).
				others.forEach(i => body.appendChild(
					renderRow(i, statusOf(i), lastErrors.get(i.id), opts.onChange),
				));
			} else {
				const more = document.createElement('details');
				const sum = document.createElement('summary');
				sum.textContent = `More (${others.length})`;
				more.appendChild(sum);
				others.forEach(i => more.appendChild(
					renderRow(i, statusOf(i), lastErrors.get(i.id), opts.onChange),
				));
				body.appendChild(more);
			}
		}
	};

	(search as HTMLInputElement).oninput = draw;
	draw();

	// --- Footer ---
	const footer = document.createElement('div');
	footer.className = 'footer';

	const add = document.createElement('button');
	add.textContent = '+ Add';
	add.onclick = opts.onAdd;

	const set = document.createElement('button');
	set.textContent = '⚙ Settings';
	set.onclick = opts.onSettings;

	footer.append(add, set);
	container.appendChild(footer);
}
