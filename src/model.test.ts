import { describe, it, expect } from 'vitest';
import { matchesSearch, splitFavorites, statusDot, type ManagedItem } from './model';

const base: ManagedItem = {
	id: '1', name: 'myapp', kind: 'project', dir: '/x', startCmd: 'npm run dev',
	stopCmd: null, port: 5173, runMode: 'background', brewFormula: null,
	dockerImage: null, containerName: null, order: 0,
	favorite: false, env: {}, healthPath: null, autoStart: false,
};

describe('model helpers', () => {
	it('matchesSearch on name, kind, port', () => {
		expect(matchesSearch(base, 'myap')).toBe(true);
		expect(matchesSearch(base, 'project')).toBe(true);
		expect(matchesSearch(base, '5173')).toBe(true);
		expect(matchesSearch(base, 'zzz')).toBe(false);
	});
	it('splitFavorites separates and preserves order', () => {
		const a = { ...base, id: 'a', favorite: true, order: 1 };
		const b = { ...base, id: 'b', favorite: false, order: 0 };
		const { favorites, others } = splitFavorites([a, b]);
		expect(favorites.map(i => i.id)).toEqual(['a']);
		expect(others.map(i => i.id)).toEqual(['b']);
	});
	it('statusDot maps each status', () => {
		expect(statusDot('running')).toContain('running');
		expect(statusDot('error')).toContain('error');
	});
});
