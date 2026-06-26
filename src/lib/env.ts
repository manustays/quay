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
