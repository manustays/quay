import { useEffect, useState } from 'react';
import { enable, disable } from '@tauri-apps/plugin-autostart';
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
import type { Settings } from '../model';
import { getSettings, updateSettings, getTerminals } from '../ipc';

interface SettingsDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onSaved: () => void;
}

/** Settings dialog. Ports settings.ts; preserves unedited fields (e.g. `browser`) via spread. */
export function SettingsDialog({ open, onOpenChange, onSaved }: SettingsDialogProps): React.JSX.Element {
	const [settings, setSettings] = useState<Settings | null>(null);
	const [terminals, setTerminals] = useState<string[]>([]);

	useEffect(() => {
		if (open) {
			void getSettings().then(setSettings);
			void getTerminals().then(setTerminals);
		}
	}, [open]);

	const set = (patch: Partial<Settings>) =>
		setSettings((s) => (s ? { ...s, ...patch } : s));

	const save = async () => {
		if (!settings) return;
		try {
			await updateSettings(settings);
			settings.launchAtLogin ? await enable() : await disable();
			onOpenChange(false);
			onSaved();
		} catch (e) {
			alert(String(e));
		}
	};

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-[320px]">
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
							<Label className="text-xs text-muted-foreground">Poll interval (sec)</Label>
							<Input
								type="number"
								min={1}
								value={settings.pollIntervalSec}
								onChange={(e) => set({ pollIntervalSec: Number(e.target.value) || 3 })}
							/>
						</div>

						<div className="grid gap-1.5">
							<Label className="text-xs text-muted-foreground">Metrics interval (sec)</Label>
							<Input
								type="number"
								min={1}
								value={settings.metricsIntervalSec}
								onChange={(e) => set({ metricsIntervalSec: Number(e.target.value) || 10 })}
							/>
						</div>

						<label className="flex items-center justify-between gap-2 text-[13px]">
							<span>Launch at login</span>
							<Switch
								checked={settings.launchAtLogin}
								onCheckedChange={(v) => set({ launchAtLogin: v })}
							/>
						</label>
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
