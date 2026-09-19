import { describe, it, expect } from 'vitest';
import {
	aggregateGroupMetrics,
	aggregateGroupStatus,
	browserUrlFor,
	groupAgentsByCwd,
	groupItems,
	matchesSearch,
	moveInList,
	splitFavorites,
	statusDot,
	type DiscoveredAgent,
	type ManagedItem,
} from './model';

const base: ManagedItem = {
	id: '1', name: 'myapp', kind: 'project', dir: '/x', startCmd: 'npm run dev',
	stopCmd: null, port: 5173, runMode: 'background', brewFormula: null,
	dockerImage: null, containerName: null, stack: null, group: null, order: 0,
	favorite: false, env: {}, healthPath: null, browserUrl: null, autoStart: false,
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
	it('browserUrlFor defaults to localhost, substitutes {port} in a template', () => {
		expect(browserUrlFor(base)).toBe('http://localhost:5173');
		expect(browserUrlFor({ ...base, browserUrl: '  ' })).toBe('http://localhost:5173');
		expect(browserUrlFor({ ...base, browserUrl: ' http://127.0.0.1:{port}/index.html ' }))
			.toBe('http://127.0.0.1:5173/index.html');
		expect(browserUrlFor({ ...base, port: null, browserUrl: 'http://my.app.localhost' }))
			.toBe('http://my.app.localhost');
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
		/** Build an ItemMetrics fixture. */
		const m = (id: string, cpu: number, mem: number, up: number | null) =>
			({ id, cpuPercent: cpu, memoryBytes: mem, uptimeSec: up });
		expect(aggregateGroupMetrics([])).toBeNull();
		expect(aggregateGroupMetrics([m('a', 10, 100, 5), m('b', 2.5, 50, 60)]))
			.toEqual({ cpuPercent: 12.5, memoryBytes: 150, uptimeSec: 60 });
		expect(aggregateGroupMetrics([m('a', 1, 1, null)])?.uptimeSec).toBeNull();
	});
	it('groupAgentsByCwd clubs ≥2 sharing a cwd, keeps first-pid order', () => {
		/** Build a DiscoveredAgent fixture. */
		const agent = (pid: number, cwd: string): DiscoveredAgent => ({
			pid, agent: 'claude', name: cwd.split('/').pop() ?? cwd, cwd, stack: null,
			sessionName: null, uptimeSec: 0, memoryBytes: 0, state: 'idle',
			tty: 'ttys002', jumpSupported: false,
		});
		const [a, b, c, d] = [agent(1, '/x/app'), agent(2, '/y/solo'), agent(3, '/x/app'), agent(4, '/z/lone')];
		const out = groupAgentsByCwd([a, b, c, d]);
		// Folder for /x/app (pids 1+3) first, then flat singles in pid order.
		expect(out).toHaveLength(3);
		expect('agents' in out[0] && out[0].agents.map(m => m.pid)).toEqual([1, 3]);
		expect('agents' in out[1]).toBe(false);
		expect((out[1] as DiscoveredAgent).pid).toBe(2);
		expect((out[2] as DiscoveredAgent).pid).toBe(4);
		expect(groupAgentsByCwd([])).toEqual([]);
	});
	it('aggregateGroupStatus precedence', () => {
		expect(aggregateGroupStatus(['running', 'error'])).toBe('error');
		expect(aggregateGroupStatus(['running', 'starting'])).toBe('starting');
		expect(aggregateGroupStatus(['running', 'running'])).toBe('running');
		expect(aggregateGroupStatus(['running', 'stopped'])).toBe('partial');
		expect(aggregateGroupStatus(['stopped', 'stopped'])).toBe('stopped');
		expect(aggregateGroupStatus([])).toBe('stopped');
	});
});
