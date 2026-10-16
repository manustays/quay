import { describe, expect, it } from 'vitest';
import { envToText, parseEnv } from './env';

describe('env helpers', () => {
	it('preserves existing values when a service is opened and saved', () => {
		const env = {
			GREETING: ' hello ',
			SEPARATOR: ' \t ',
			EMPTY: '',
			TOKEN: 'a=b==',
		};
		expect(parseEnv(envToText(env))).toEqual(env);
	});

	it.each(['\n', '\r\n'])('preserves value whitespace with %j line endings', (newline) => {
		expect(parseEnv(`GREETING= hello ${newline}SEPARATOR=\t ${newline}EMPTY=${newline}`))
			.toEqual({ GREETING: ' hello ', SEPARATOR: '\t ', EMPTY: '' });
	});

	it('trims names and splits values at only the first equals sign', () => {
		expect(parseEnv('  TOKEN \t=a=b==')).toEqual({ TOKEN: 'a=b==' });
	});

	it('ignores blank lines and lines without a name or equals sign', () => {
		expect(parseEnv('\nnot an assignment\n=value\nKEY=value\n')).toEqual({ KEY: 'value' });
	});

	it('uses the last value for a repeated name', () => {
		expect(parseEnv('KEY=first\nKEY=last')).toEqual({ KEY: 'last' });
	});
});
