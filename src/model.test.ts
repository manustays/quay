import { describe, it, expect } from 'vitest';
import { matchesSearch, moveInList, splitFavorites, statusDot, type ManagedItem } from './model';

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
	it('moveInList moves an item down and up', () => {
		expect(moveInList(['a', 'b', 'c'], 0, 2)).toEqual(['b', 'c', 'a']);
		expect(moveInList(['a', 'b', 'c'], 2, 0)).toEqual(['c', 'a', 'b']);
		expect(moveInList(['a', 'b', 'c'], 1, 1)).toEqual(['a', 'b', 'c']);
	});
	it('statusDot maps each status', () => {
		expect(statusDot('running')).toContain('running');
		expect(statusDot('error')).toContain('error');
	});
});
