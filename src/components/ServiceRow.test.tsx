// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ServiceRow } from './ServiceRow';
import { TooltipProvider } from './ui/tooltip';
import type { ManagedItem } from '../model';

const item: ManagedItem = {
	id: '1', name: 'myapp', kind: 'project', dir: '/x', startCmd: 'npm run dev',
	stopCmd: null, port: 5173, runMode: 'background', brewFormula: null,
	dockerImage: null, containerName: null, stack: null, group: null, order: 0,
	favorite: false, env: {}, healthPath: null, browserUrl: 'http://app.local:{port}', autoStart: false,
};

let container: HTMLDivElement;
let root: Root;
let copy: HTMLElement;
const writeText = vi.fn<(text: string) => Promise<void>>();

beforeEach(async () => {
	vi.useFakeTimers();
	vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
	vi.stubGlobal('navigator', { clipboard: { writeText } });
	vi.spyOn(window, 'alert').mockImplementation(() => {});
	writeText.mockReset();
	container = document.createElement('div');
	document.body.append(container);
	root = createRoot(container);
	await act(async () => root.render(
		<TooltipProvider>
			<ServiceRow item={item} status="stopped" lastError={undefined} metrics={undefined}
				index={0} open={false} onOpenChange={vi.fn()} onChange={vi.fn()} onEdit={vi.fn()} />
		</TooltipProvider>,
	));
	copy = container.querySelector<HTMLElement>('[role="button"]')!;
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.useRealTimers();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

it('waits for clipboard success before showing feedback and ignores duplicate clicks', async () => {
	let resolve!: () => void;
	writeText.mockReturnValue(new Promise<void>(done => { resolve = done; }));
	await act(async () => { copy.click(); copy.click(); });
	expect(writeText).toHaveBeenCalledExactlyOnceWith('http://app.local:5173');
	expect(copy.textContent).toBe(':5173');
	await act(async () => resolve());
	expect(copy.textContent).toBe('copied!');
	await act(async () => vi.advanceTimersByTime(1000));
	expect(copy.textContent).toBe(':5173');
});

it('reports a rejected write without success feedback and allows keyboard retry', async () => {
	writeText.mockRejectedValueOnce(new Error('Permission denied')).mockResolvedValueOnce(undefined);
	await act(async () => copy.click());
	expect(copy.textContent).toBe(':5173');
	expect(window.alert).toHaveBeenCalledWith(expect.stringContaining('Permission denied'));
	await act(async () => {
		copy.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
	});
	expect(writeText).toHaveBeenCalledTimes(2);
	expect(copy.textContent).toBe('copied!');
});

it('clears earlier success feedback when the next copy fails', async () => {
	writeText.mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error('Unavailable'));
	await act(async () => copy.click());
	expect(copy.textContent).toBe('copied!');
	await act(async () => copy.click());
	expect(copy.textContent).toBe(':5173');
	expect(window.alert).toHaveBeenCalledOnce();
});

it('keeps feedback visible for a full second after each successful copy', async () => {
	writeText.mockResolvedValue(undefined);
	await act(async () => copy.click());
	await act(async () => vi.advanceTimersByTime(500));
	await act(async () => copy.click());
	await act(async () => vi.advanceTimersByTime(500));
	expect(copy.textContent).toBe('copied!');
	await act(async () => vi.advanceTimersByTime(500));
	expect(copy.textContent).toBe(':5173');
});

it('handles an unavailable clipboard API without showing success', async () => {
	vi.stubGlobal('navigator', {});
	await act(async () => copy.click());
	expect(copy.textContent).toBe(':5173');
	expect(window.alert).toHaveBeenCalledWith(expect.stringContaining('Could not copy URL:'));
});
