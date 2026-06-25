import { getSettings, updateSettings } from './ipc';
import { enable, disable } from '@tauri-apps/plugin-autostart';
import type { Settings } from './model';

/**
 * Open the settings modal overlay.
 * Loads current settings via IPC, renders a form for terminalApp,
 * pollIntervalSec, and launchAtLogin, and on save persists the
 * new values and toggles the macOS login item via the autostart plugin.
 *
 * @param onDone - Callback invoked after settings are saved and the modal is closed.
 */
export async function openSettings(onDone: () => void): Promise<void> {
	const s: Settings = await getSettings();

	const overlay = document.createElement('div');
	overlay.className = 'overlay';

	// Static HTML template — no interpolated user data; values set below via .value / .checked
	overlay.innerHTML = `
		<div class="modal">
			<label>Terminal app
				<select id="s-term">
					<option value="Terminal">Terminal</option>
					<option value="iTerm">iTerm</option>
				</select>
			</label>
			<label>Poll interval (sec)
				<input id="s-poll" type="number" min="1">
			</label>
			<label>
				<input type="checkbox" id="s-login"> Launch at login
			</label>
			<div class="modal-actions">
				<button id="s-cancel">Cancel</button>
				<button id="s-save">Save</button>
			</div>
		</div>`;

	document.body.appendChild(overlay);

	/** Typed helper to query within the overlay. */
	const $ = <T extends HTMLElement>(id: string): T =>
		overlay.querySelector<T>(id)!;

	// Populate form fields with current values — never via innerHTML interpolation
	$<HTMLSelectElement>('#s-term').value = s.terminalApp;
	$<HTMLInputElement>('#s-poll').value = String(s.pollIntervalSec);
	$<HTMLInputElement>('#s-login').checked = s.launchAtLogin;

	$<HTMLButtonElement>('#s-cancel').onclick = () => overlay.remove();

	$<HTMLButtonElement>('#s-save').onclick = async () => {
		const next: Settings = {
			...s,
			terminalApp: $<HTMLSelectElement>('#s-term').value,
			pollIntervalSec: Number($<HTMLInputElement>('#s-poll').value) || 3,
			launchAtLogin: $<HTMLInputElement>('#s-login').checked,
		};

		try {
			await updateSettings(next);
			next.launchAtLogin ? await enable() : await disable();
			overlay.remove();
			onDone();
		} catch (e) {
			alert(String(e));
		}
	};
}
