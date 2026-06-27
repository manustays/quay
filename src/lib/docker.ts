import { dockerDaemonRunning, setSuppressHide, startDockerDaemon } from '../ipc';

/**
 * Ensure the Docker daemon is up before a Docker action.
 *
 * Returns immediately if the daemon already answers. Otherwise prompts the user
 * (prompt-then-auto-start), and on confirm launches Docker Desktop and waits.
 * Resolves `true` only when the daemon is ready to use.
 *
 * The native `confirm` can blur the popover, so hide-on-blur is suppressed around
 * it (mirrors the folder-pick flow in ServiceForm).
 */
export async function ensureDockerDaemon(): Promise<boolean> {
	if (await dockerDaemonRunning()) return true;

	await setSuppressHide(true);
	let proceed: boolean;
	try {
		proceed = window.confirm("Docker Desktop isn't running. Start it now?");
	} finally {
		await setSuppressHide(false);
	}
	if (!proceed) return false;

	try {
		const up = await startDockerDaemon();
		if (!up) {
			alert('Docker did not start in time. Try again once Docker Desktop is ready.');
		}
		return up;
	} catch (e) {
		alert(String(e));
		return false;
	}
}
