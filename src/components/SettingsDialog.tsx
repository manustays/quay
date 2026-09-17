import { useEffect, useState } from 'react';
import { enable, disable, isEnabled } from '@tauri-apps/plugin-autostart';
import { Button } from '@/components/ui/button';
import {
	Dialog,
	DialogContent,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from '@/components/ui/select';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { CircleHelp } from 'lucide-react';
import { RowIcon } from '@/components/StackIcon';
import type { AgentKind, HookStatus, Settings } from '../model';
import {
	getSettings,
	updateSettings,
	getTerminals,
	getHookStatuses,
	installAgentHooks,
	uninstallAgentHooks,
} from '../ipc';

const AGENT_LABELS: Record<AgentKind, string> = {
	claude: 'Claude Code',
	codex: 'Codex',
	opencode: 'OpenCode',
	pi: 'Pi',
};

/**
 * A `?` beside a setting's label, explaining what the setting costs. Worth the
 * pixel: the intervals differ in whether they burn CPU with the popover closed,
 * which is invisible from the number alone.
 */
function InfoHint({ text }: { text: string }): React.JSX.Element {
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					aria-label={text}
					// Explain-only: focusable for keyboard/VoiceOver, but never submits
					// or steals the dialog's default action.
					className="text-muted-foreground/70 hover:text-foreground focus-visible:text-foreground"
				>
					<CircleHelp className="size-3.5" />
				</button>
			</TooltipTrigger>
			<TooltipContent className="max-w-[220px]">{text}</TooltipContent>
		</Tooltip>
	);
}

interface SettingsDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onSaved: () => void;
}

/** Settings dialog. Ports settings.ts; preserves unedited fields (e.g. `browser`) via spread. */
export function SettingsDialog({ open, onOpenChange, onSaved }: SettingsDialogProps): React.JSX.Element {
	const [settings, setSettings] = useState<Settings | null>(null);
	const [terminals, setTerminals] = useState<string[]>([]);
	const [hooks, setHooks] = useState<HookStatus[]>([]);
	// Per-agent hook-install error, shown inline on the row instead of a modal.
	const [hookErrors, setHookErrors] = useState<Partial<Record<AgentKind, string>>>({});
	const [hookBusy, setHookBusy] = useState<AgentKind | null>(null);

	useEffect(() => {
		if (open) {
			void getSettings().then(setSettings);
			void getTerminals().then(setTerminals);
			void getHookStatuses().then(setHooks).catch(() => setHooks([]));
		}
	}, [open]);

	/** Install/remove one agent's hooks immediately, then refresh statuses. */
	const toggleHook = (h: HookStatus) => async () => {
		setHookBusy(h.agent);
		setHookErrors((e) => ({ ...e, [h.agent]: undefined }));
		try {
			await (h.installed ? uninstallAgentHooks(h.agent) : installAgentHooks(h.agent));
			setHooks(await getHookStatuses());
		} catch (err) {
			setHookErrors((e) => ({ ...e, [h.agent]: String(err) }));
		} finally {
			setHookBusy(null);
		}
	};

	const set = (patch: Partial<Settings>) =>
		setSettings((s) => (s ? { ...s, ...patch } : s));

	const save = async () => {
		if (!settings) return;
		try {
			await updateSettings(settings);
			// Only touch the launchd LaunchAgent when the state actually changes —
			// re-registering it re-fires macOS's "can run in the background" notice.
			const currentlyEnabled = await isEnabled();
			if (settings.launchAtLogin && !currentlyEnabled) await enable();
			else if (!settings.launchAtLogin && currentlyEnabled) await disable();
			onOpenChange(false);
			onSaved();
		} catch (e) {
			alert(String(e));
		}
	};

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-h-[88vh] overflow-y-auto sm:max-w-[320px]">
				<DialogHeader>
					<DialogTitle>Settings</DialogTitle>
				</DialogHeader>

				{settings && (
					<div className="grid gap-3 py-2">
						<div className="grid gap-1.5">
							<Label className="text-xs text-muted-foreground">Terminal app</Label>
							<Select value={settings.terminalApp} onValueChange={(v) => set({ terminalApp: v })}>
								<SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
								<SelectContent>
									{Array.from(new Set([...terminals, settings.terminalApp]))
										.filter(Boolean)
										.map((name) => (
											<SelectItem key={name} value={name}>{name}</SelectItem>
										))}
								</SelectContent>
							</Select>
						</div>

						<div className="grid gap-1.5">
							<Label className="gap-1.5 text-xs text-muted-foreground">
								Poll interval (sec)
								<InfoHint text="How often each service's status is re-checked. Runs all the time, even with this window closed — the one interval that affects battery. Raise it to use less CPU." />
							</Label>
							<Input
								type="number"
								min={1}
								value={settings.pollIntervalSec}
								onChange={(e) => set({ pollIntervalSec: Number(e.target.value) || 3 })}
							/>
						</div>

						<div className="grid gap-1.5">
							<Label className="gap-1.5 text-xs text-muted-foreground">
								Metrics interval (sec)
								<InfoHint text="How often CPU and memory are sampled per service. Only while this window is open — closed, it costs nothing." />
							</Label>
							<Input
								type="number"
								min={1}
								value={settings.metricsIntervalSec}
								onChange={(e) => set({ metricsIntervalSec: Number(e.target.value) || 10 })}
							/>
						</div>

						<div className="grid gap-1.5">
							<Label className="gap-1.5 text-xs text-muted-foreground">
								Agent interval (sec)
								<InfoHint text="How often AI coding agent sessions are re-scanned. Only while this window is open — the menubar waiting badge updates on the poll interval instead." />
							</Label>
							<Input
								type="number"
								min={1}
								disabled={!settings.trackAgents}
								value={settings.agentIntervalSec}
								onChange={(e) => set({ agentIntervalSec: Number(e.target.value) || 5 })}
							/>
						</div>

						<label className="flex items-center justify-between gap-2 text-[13px]">
							<span>Launch at login</span>
							<Switch
								checked={settings.launchAtLogin}
								onCheckedChange={(v) => set({ launchAtLogin: v })}
							/>
						</label>

						<label className="flex items-center justify-between gap-2 text-[13px]">
							<span>Detected: dev stacks only</span>
							<Switch
								checked={settings.radarDevOnly}
								onCheckedChange={(v) => set({ radarDevOnly: v })}
							/>
						</label>

						<label className="flex items-center justify-between gap-2 text-[13px]">
							<span>Track AI coding agents</span>
							<Switch
								checked={settings.trackAgents}
								onCheckedChange={(v) => set({ trackAgents: v })}
							/>
						</label>

						{/* Both of these only mean anything while the radar is running, so they
						    follow the toggle above rather than sitting there inert. */}
						<label
							className="flex items-center justify-between gap-2 pl-4 text-[13px]"
							data-disabled={!settings.trackAgents}
						>
							<span className={settings.trackAgents ? undefined : 'text-muted-foreground'}>
								Show waiting count in menubar
							</span>
							<Switch
								disabled={!settings.trackAgents}
								checked={settings.waitingTitleBadge && settings.trackAgents}
								onCheckedChange={(v) => set({ waitingTitleBadge: v })}
							/>
						</label>

						{hooks.length > 0 && (
							<div className={`grid gap-1.5 ${settings.trackAgents ? '' : 'opacity-50'}`}>
								<Label className="gap-1.5 text-xs text-muted-foreground">
									Agent radar hooks (working / waiting / idle)
									{!settings.trackAgents && ' — tracking off'}
								</Label>
								{hooks.map((h) => (
									<div key={h.agent} className="flex flex-col gap-0.5">
										<div className="flex items-center justify-between gap-2 text-[13px]">
											<span className="flex min-w-0 items-center gap-1.5">
												<RowIcon stack={h.agent} />
												<span className="truncate">{AGENT_LABELS[h.agent]}</span>
											</span>
											<span className="flex shrink-0 items-center gap-1.5">
												<span className="text-[11px] text-muted-foreground">
													{h.installed ? 'Installed' : 'Not installed'}
												</span>
												<Button
													variant="ghost"
													size="sm"
													disabled={hookBusy === h.agent || !settings.trackAgents}
													onClick={toggleHook(h)}
												>
													{h.installed ? 'Remove' : 'Install'}
												</Button>
											</span>
										</div>
										{hookErrors[h.agent] && (
											<span className="pl-6 text-[11px] text-destructive">{hookErrors[h.agent]}</span>
										)}
									</div>
								))}
							</div>
						)}

						{settings.ignoredPorts.length > 0 && (
							<div className="grid gap-1.5">
								<Label className="text-xs text-muted-foreground">Ignored ports (click to unhide)</Label>
								<div className="flex flex-wrap gap-1">
									{settings.ignoredPorts.map((port) => (
										<button
											key={port}
											type="button"
											onClick={() =>
												set({ ignoredPorts: settings.ignoredPorts.filter((p) => p !== port) })
											}
											className="rounded-full bg-muted px-2 py-0.5 font-mono text-[11px] text-muted-foreground hover:bg-destructive/15 hover:text-destructive"
											title={`Stop ignoring port ${port}`}
										>
											:{port} ×
										</button>
									))}
								</div>
							</div>
						)}

						{settings.ignoredAgents.length > 0 && (
							<div className="grid gap-1.5">
								<Label className="text-xs text-muted-foreground">Ignored agents (click to unhide)</Label>
								<div className="flex flex-wrap gap-1">
									{settings.ignoredAgents.map((ia) => (
										<button
											key={`${ia.agent}:${ia.cwd}`}
											type="button"
											onClick={() =>
												set({
													ignoredAgents: settings.ignoredAgents.filter(
														(x) => !(x.agent === ia.agent && x.cwd === ia.cwd),
													),
												})
											}
											className="rounded-full bg-muted px-2 py-0.5 font-mono text-[11px] text-muted-foreground hover:bg-destructive/15 hover:text-destructive"
											title={`Stop ignoring ${ia.agent} sessions in ${ia.cwd}`}
										>
											{ia.agent} · {ia.cwd.split('/').pop() || ia.cwd} ×
										</button>
									))}
								</div>
							</div>
						)}
					</div>
				)}

				<DialogFooter>
					<Button variant="ghost" size="sm" onClick={() => onOpenChange(false)}>Cancel</Button>
					<Button size="sm" onClick={save} disabled={!settings}>Save</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
