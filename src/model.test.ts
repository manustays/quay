import { describe, it, expect } from 'vitest';
import {
	aggregateGroupMetrics,
	aggregateGroupStatus,
	groupItems,
	matchesSearch,
	moveInList,
	splitFavorites,
	statusDot,
	type ManagedItem,
} from './model';

const base: ManagedItem = {
	id: '1', name: 'myapp', kind: 'project', dir: '/x', startCmd: 'npm run dev',
	stopCmd: null, port: 5173, runMode: 'background', brewFormula: null,
	dockerImage: null, containerName: null, stack: null, group: null, order: 0,
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
	it('groupItems clusters by first-member position, keeps ungrouped', () => {
		const a = { ...base, id: 'a', group: 'app', order: 0 };
		const b = { ...base, id: 'b', group: null, order: 1 };
		const c = { ...base, id: 'c', group: 'db', order: 2 };
		const d = { ...base, id: 'd', group: 'app', order: 3 };
		const { groups, ungrouped } = groupItems([a, b, c, d]);
		expect(groups.map(g => g.name)).toEqual(['app', 'db']);
		expect(groups[0].items.map(i => i.id)).toEqual(['a', 'd']);
		expect(ungrouped.map(i => i.id)).toEqual(['b']);
	});
	it('aggregateGroupMetrics sums cpu/mem, maxes uptime', () => {
		const m = (id: string, cpu: number, mem: number, up: number | null) =>
			({ id, cpuPercent: cpu, memoryBytes: mem, uptimeSec: up });
		expect(aggregateGroupMetrics([])).toBeNull();
		expect(aggregateGroupMetrics([m('a', 10, 100, 5), m('b', 2.5, 50, 60)]))
			.toEqual({ cpuPercent: 12.5, memoryBytes: 150, uptimeSec: 60 });
		expect(aggregateGroupMetrics([m('a', 1, 1, null)])?.uptimeSec).toBeNull();
	});
	it('aggregateGroupStatus precedence', () => {
		expect(aggregateGroupStatus(['running', 'error'])).toBe('error');
		expect(aggregateGroupStatus(['running', 'starting'])).toBe('starting');
		expect(aggregateGroupStatus(['running', 'running'])).toBe('running');
		expect(aggregateGroupStatus(['running', 'stopped'])).toBe('stopped');
		expect(aggregateGroupStatus([])).toBe('stopped');
	});
});
