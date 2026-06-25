import { getItems, onStatusChanged } from './ipc';
import { renderList } from './list';
import { openForm } from './form';
import { openSettings } from './settings';
import type { ManagedItem, Status } from './model';

const app = document.querySelector<HTMLDivElement>('#app')!;
const statuses = new Map<string, Status>();
/** Last error message per item id, populated from `status_changed` events. */
const lastErrors = new Map<string, string>();
let items: ManagedItem[] = [];

/**
 * Reload all items from the backend and re-render the list.
 * Called on startup and after any mutation (add, delete, start, stop, etc.).
 */
async function refresh(): Promise<void> {
	items = await getItems();
	render();
}

/** Re-render the list with the current `items` and `statuses` snapshots. */
function render(): void {
	renderList(app, items, statuses, lastErrors, {
		onChange: refresh,
		onAdd: () => openForm(null, refresh),
		onSettings: () => openSettings(refresh),
	});
}

// Subscribe to live status events from the backend — update the map and re-render
// without hitting the backend (statuses arrive as push events, not polled).
onStatusChanged((s) => {
	statuses.set(s.id, s.status);
	if (s.lastError != null) {
		lastErrors.set(s.id, s.lastError);
	} else if (s.status !== 'error') {
		lastErrors.delete(s.id);
	}
	render();
});

refresh();
