import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { ManagedItem, Settings, ItemStatus, DetectResult } from './model';

export const getItems = () => invoke<ManagedItem[]>('get_items');
export const addItem = (item: ManagedItem) => invoke<ManagedItem>('add_item', { item });
export const updateItem = (item: ManagedItem) => invoke<void>('update_item', { item });
export const deleteItem = (id: string) => invoke<void>('delete_item', { id });
export const reorder = (ids: string[]) => invoke<void>('reorder', { ids });
export const toggleFavorite = (id: string) => invoke<void>('toggle_favorite', { id });
export const startItem = (id: string) => invoke<void>('start_item', { id });
export const stopItem = (id: string) => invoke<void>('stop_item', { id });
export const stopAll = () => invoke<void>('stop_all');
export const openBrowser = (id: string) => invoke<void>('open_browser', { id });
export const openTerminal = (id: string) => invoke<void>('open_terminal', { id });
export const tailLog = (id: string, lines: number) => invoke<string>('tail_log', { id, lines });
export const detectFolder = (path: string) => invoke<DetectResult>('detect_folder_cmd', { path });
export const getStatuses = () => invoke<ItemStatus[]>('get_statuses');
export const getSettings = () => invoke<Settings>('get_settings');
export const updateSettings = (settings: Settings) => invoke<void>('update_settings', { settings });

/**
 * List formula names known to `brew services`.
 * Returns an empty array when Homebrew is unavailable.
 */
export const listBrewFormulae = () => invoke<string[]>('list_brew_formulae');

/**
 * Subscribe to backend status-changed events.
 * The callback receives an {@link ItemStatus} payload each time a service
 * transitions state. Returns a Promise that resolves to an unlisten function —
 * call it to stop receiving events (e.g. on component unmount).
 */
export function onStatusChanged(cb: (s: ItemStatus) => void): Promise<UnlistenFn> {
	return listen<ItemStatus>('status_changed', (e) => cb(e.payload));
}

/**
 * Suppress (or re-enable) hide-on-blur in the Rust backend.
 * Call with `true` before opening a native dialog and `false` in a `finally`
 * block after it closes, so the popover stays visible during the pick flow.
 */
export const setSuppressHide = (value: boolean) => invoke<void>('set_suppress_hide', { value });
