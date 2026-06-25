import { statusDot, type ManagedItem, type Status } from './model';
import { startItem, stopItem, openBrowser, openTerminal, toggleFavorite, deleteItem, tailLog } from './ipc';

/**
 * Render a single item row element.
 * Shows a status dot, name, port/kind label, and action buttons.
 * Clicking the row body expands a panel with log tail, favorite toggle, and delete.
 *
 * All user-supplied strings (name, log output) are set via `textContent` to
 * prevent XSS — no `innerHTML` is used for untrusted data.
 *
 * @param item - The managed service item to render.
 * @param status - Current live status of the item.
 * @param onChange - Callback to trigger a full list re-render after any mutation.
 * @returns A `div.row` HTMLElement ready to insert into the DOM.
 */
export function renderRow(item: ManagedItem, status: Status, onChange: () => void): HTMLElement {
	const row = document.createElement('div');
	row.className = 'row';

	const dotStr = statusDot(status).split(' ');
	const dotGlyph = dotStr[0];
	const dotCls = dotStr[1];
	const label = item.kind === 'brew' ? 'brew' : (item.port != null ? `:${item.port}` : item.runMode);
	const running = status === 'running' || status === 'starting';

	// Build row body using safe DOM methods — no innerHTML for untrusted values.
	const dotEl = document.createElement('span');
	dotEl.className = `dot ${dotCls}`;
	dotEl.textContent = dotGlyph;

	const nameEl = document.createElement('span');
	nameEl.className = 'name';
	nameEl.textContent = item.name;

	const metaEl = document.createElement('span');
	metaEl.className = 'meta';
	metaEl.textContent = label;

	const actionsEl = document.createElement('span');
	actionsEl.className = 'actions';

	row.append(dotEl, nameEl, metaEl, actionsEl);

	/** Helper to create and append an action button. */
	const btn = (text: string, title: string, fn: () => Promise<unknown>): void => {
		const b = document.createElement('button');
		b.textContent = text;
		b.title = title;
		b.onclick = async (e: MouseEvent) => {
			e.stopPropagation();
			try { await fn(); } catch (err) { alert(String(err)); }
			onChange();
		};
		actionsEl.appendChild(b);
	};

	btn(running ? '■' : '▶', running ? 'Stop' : 'Start', () => running ? stopItem(item.id) : startItem(item.id));
	if (item.port != null) btn('↗', 'Open in browser', () => openBrowser(item.id));
	if (item.dir) btn('>_', 'Open terminal', () => openTerminal(item.id));

	// Expand panel on row-body click.
	row.onclick = async () => {
		const existing = row.querySelector('.expand');
		if (existing) { existing.remove(); return; }

		const panel = document.createElement('div');
		panel.className = 'expand';

		const log = await tailLog(item.id, 20).catch(() => '');

		// Use textContent for log output — it may contain arbitrary process output.
		const pre = document.createElement('pre');
		pre.className = 'log';
		pre.textContent = log || '(no log)';
		panel.appendChild(pre);

		const fav = document.createElement('button');
		fav.textContent = item.favorite ? '★ Unfavorite' : '☆ Favorite';
		fav.onclick = async (e: MouseEvent) => {
			e.stopPropagation();
			await toggleFavorite(item.id);
			onChange();
		};

		const del = document.createElement('button');
		del.textContent = 'Delete';
		del.onclick = async (e: MouseEvent) => {
			e.stopPropagation();
			if (confirm(`Delete ${item.name}?`)) {
				await deleteItem(item.id);
				onChange();
			}
		};

		panel.append(fav, del);
		row.appendChild(panel);
	};

	return row;
}
