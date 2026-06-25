import { open } from '@tauri-apps/plugin-dialog';
import { addItem, updateItem, detectFolder, listBrewFormulae } from './ipc';
import type { ManagedItem, ItemKind, RunMode } from './model';

/** Return a blank ManagedItem for the add-new flow. */
function blank(): ManagedItem {
	return {
		id: '',
		name: '',
		kind: 'project',
		dir: null,
		startCmd: null,
		stopCmd: null,
		port: null,
		runMode: 'background',
		brewFormula: null,
		order: 0,
		favorite: false,
		env: {},
		healthPath: null,
		autoStart: false,
	};
}

/**
 * Parse "KEY=VALUE" lines into an env record.
 * Lines without an `=` are silently skipped.
 *
 * @param text - Multi-line string where each line is `KEY=VALUE`.
 * @returns Record mapping env var names to values.
 */
export function parseEnv(text: string): Record<string, string> {
	const env: Record<string, string> = {};
	for (const line of text.split('\n')) {
		const i = line.indexOf('=');
		if (i > 0) {
			env[line.slice(0, i).trim()] = line.slice(i + 1).trim();
		}
	}
	return env;
}

/**
 * Serialise an env record back to "KEY=VALUE" lines for display in a textarea.
 *
 * @param env - Record mapping env var names to values.
 * @returns Multi-line string.
 */
export function envToText(env: Record<string, string>): string {
	return Object.entries(env).map(([k, v]) => `${k}=${v}`).join('\n');
}

/**
 * Open the add/edit modal overlay.
 *
 * When `item` is `null` the form opens in "add" mode and calls `addItem` on
 * Save; otherwise it opens in "edit" mode and calls `updateItem`. Picking a
 * folder calls `detectFolder` and prefills name/kind/startCmd/port from the
 * returned `DetectResult`.
 *
 * @param item   - Existing item to edit, or `null` to add a new one.
 * @param onDone - Callback invoked after a successful save or on cancel.
 */
export function openForm(item: ManagedItem | null, onDone: () => void): void {
	const data: ManagedItem = item ? { ...item } : blank();

	const overlay = document.createElement('div');
	overlay.className = 'overlay';
	overlay.innerHTML = `
		<div class="modal">
			<label>Name <input id="f-name"></label>
			<label>Kind
				<select id="f-kind">
					<option value="project">project</option>
					<option value="brew">brew</option>
					<option value="agent">agent</option>
				</select>
			</label>
			<label id="f-dir-row">Folder <input id="f-dir" readonly><button id="f-pick">Pick…</button></label>
			<label>Start cmd <input id="f-cmd"></label>
			<label>Stop cmd <input id="f-stop"></label>
			<label>Port <input id="f-port" type="number"></label>
			<label>Run mode
				<select id="f-mode">
					<option value="background">background</option>
					<option value="terminal">terminal</option>
				</select>
			</label>
			<label>Brew formula <input id="f-formula" list="f-formula-list"><datalist id="f-formula-list"></datalist></label>
			<label>Env (KEY=VALUE per line) <textarea id="f-env"></textarea></label>
			<label>Health path <input id="f-health" placeholder="/health"></label>
			<label><input type="checkbox" id="f-fav"> Favorite</label>
			<label><input type="checkbox" id="f-auto"> Auto-start on launch</label>
			<div class="modal-actions">
				<button id="f-cancel">Cancel</button>
				<button id="f-save">Save</button>
			</div>
		</div>`;
	document.body.appendChild(overlay);

	/** Typed querySelector helper scoped to the overlay. */
	const $el = <T extends HTMLElement>(selector: string): T =>
		overlay.querySelector<T>(selector)!;

	/** Populate all form fields from a ManagedItem. */
	const fill = (d: ManagedItem): void => {
		$el<HTMLInputElement>('#f-name').value = d.name;
		$el<HTMLSelectElement>('#f-kind').value = d.kind;
		$el<HTMLInputElement>('#f-dir').value = d.dir ?? '';
		$el<HTMLInputElement>('#f-cmd').value = d.startCmd ?? '';
		$el<HTMLInputElement>('#f-stop').value = d.stopCmd ?? '';
		$el<HTMLInputElement>('#f-port').value = d.port != null ? String(d.port) : '';
		$el<HTMLSelectElement>('#f-mode').value = d.runMode;
		$el<HTMLInputElement>('#f-formula').value = d.brewFormula ?? '';
		$el<HTMLTextAreaElement>('#f-env').value = envToText(d.env);
		$el<HTMLInputElement>('#f-health').value = d.healthPath ?? '';
		$el<HTMLInputElement>('#f-fav').checked = d.favorite;
		$el<HTMLInputElement>('#f-auto').checked = d.autoStart;
	};

	fill(data);

	/**
	 * Update visibility and datalist whenever the kind selection changes.
	 * Brew items need no folder; populate the formula datalist from brew services.
	 *
	 * @param kind - The newly selected item kind.
	 */
	const applyKindUI = async (kind: string): Promise<void> => {
		const dirRow = $el<HTMLLabelElement>('#f-dir-row');
		const datalist = $el<HTMLDataListElement>('#f-formula-list');
		if (kind === 'brew') {
			dirRow.style.display = 'none';
			const formulae = await listBrewFormulae();
			datalist.innerHTML = formulae
				.map((f) => `<option value="${f}"></option>`)
				.join('');
		} else {
			dirRow.style.display = '';
			datalist.innerHTML = '';
		}
	};

	// Trigger datalist population and dir-row visibility on kind change.
	$el<HTMLSelectElement>('#f-kind').onchange = async () => {
		await applyKindUI($el<HTMLSelectElement>('#f-kind').value);
	};

	// Apply initial state based on the kind already loaded into the form.
	void applyKindUI(data.kind);

	// Pick folder → detectFolder → prefill name/kind/startCmd/port.
	$el<HTMLButtonElement>('#f-pick').onclick = async () => {
		const picked = await open({ directory: true });
		if (typeof picked !== 'string') return;
		const det = await detectFolder(picked);
		// Preserve a user-supplied name; otherwise use detected name.
		const currentName = $el<HTMLInputElement>('#f-name').value;
		fill({
			...data,
			dir: picked,
			name: currentName || det.name,
			kind: det.kind,
			startCmd: det.startCmd,
			port: det.port,
		});
		// Keep data.dir in sync so subsequent fills don't clobber the choice.
		data.dir = picked;
	};

	$el<HTMLButtonElement>('#f-cancel').onclick = () => {
		overlay.remove();
		onDone();
	};

	$el<HTMLButtonElement>('#f-save').onclick = async () => {
		const portRaw = $el<HTMLInputElement>('#f-port').value;
		const result: ManagedItem = {
			...data,
			name: $el<HTMLInputElement>('#f-name').value,
			kind: $el<HTMLSelectElement>('#f-kind').value as ItemKind,
			dir: $el<HTMLInputElement>('#f-dir').value || null,
			startCmd: $el<HTMLInputElement>('#f-cmd').value || null,
			stopCmd: $el<HTMLInputElement>('#f-stop').value || null,
			port: portRaw ? Number(portRaw) : null,
			runMode: $el<HTMLSelectElement>('#f-mode').value as RunMode,
			brewFormula: $el<HTMLInputElement>('#f-formula').value || null,
			env: parseEnv($el<HTMLTextAreaElement>('#f-env').value),
			healthPath: $el<HTMLInputElement>('#f-health').value || null,
			favorite: $el<HTMLInputElement>('#f-fav').checked,
			autoStart: $el<HTMLInputElement>('#f-auto').checked,
		};
		try {
			if (item) {
				await updateItem(result);
			} else {
				await addItem(result);
			}
			overlay.remove();
			onDone();
		} catch (e) {
			alert(String(e));
		}
	};
}
