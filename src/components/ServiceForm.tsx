import { useEffect, useState } from 'react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
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
import { Textarea } from '@/components/ui/textarea';
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from '@/components/ui/select';
import type { ItemKind, ManagedItem, RunMode } from '../model';
import {
	addItem,
	detectFolder,
	listBrewFormulae,
	listDockerImages,
	setSuppressHide,
	updateItem,
} from '../ipc';
import { envToText, parseEnv } from '@/lib/env';
import { ensureDockerDaemon } from '@/lib/docker';

interface ServiceFormProps {
	open: boolean;
	/** Item to edit, or null to add a new one. */
	item: ManagedItem | null;
	/** Existing group names, offered as autocomplete for the Group field. */
	groups: string[];
	onOpenChange: (open: boolean) => void;
	onSaved: () => void;
}

/** A blank item for the add-new flow. */
function blank(): ManagedItem {
	return {
		id: '',
		name: '',
		kind: 'project',
		dir: null,
		startCmd: null,
		stopCmd: null,
		port: null,
		runMode: 'background',
		brewFormula: null,
		dockerImage: null,
		containerName: null,
		stack: null,
		group: null,
		order: 0,
		favorite: false,
		env: {},
		healthPath: null,
		autoStart: false,
	};
}

/** Add/edit dialog. Ports the original form.ts flow into a controlled React form. */
export function ServiceForm({ open, item, groups, onOpenChange, onSaved }: ServiceFormProps): React.JSX.Element {
	const [data, setData] = useState<ManagedItem>(blank);
	const [envText, setEnvText] = useState('');
	const [portText, setPortText] = useState('');
	const [formulae, setFormulae] = useState<string[]>([]);
	const [images, setImages] = useState<string[]>([]);
	// An item with an empty id is an add-mode draft (e.g. adopt-from-radar
	// prefill) — it doesn't exist in the config yet, so save must addItem.
	const isEdit = item !== null && item.id !== '';

	// Reset all fields whenever the dialog opens (for a fresh item or an edit).
	useEffect(() => {
		if (!open) return;
		const d = item ? { ...item } : blank();
		setData(d);
		setEnvText(envToText(d.env));
		setPortText(d.port != null ? String(d.port) : '');
	}, [open, item]);

	// Load brew formulae once per open, only when relevant (kind === 'brew').
	useEffect(() => {
		if (open && data.kind === 'brew' && formulae.length === 0) {
			void listBrewFormulae().then(setFormulae).catch(() => {});
		}
	}, [open, data.kind, formulae.length]);

	// Load installed docker images once per open, only when kind === 'docker'.
	// Returns empty when the daemon is down — the form shows a "Start Docker" hint.
	useEffect(() => {
		if (open && data.kind === 'docker' && images.length === 0) {
			void listDockerImages().then(setImages).catch(() => {});
		}
	}, [open, data.kind, images.length]);

	const set = (patch: Partial<ManagedItem>) => setData((d) => ({ ...d, ...patch }));

	/** Autofill name / container / start command from a picked image (only when empty). */
	const pickImage = (img: string) =>
		setData((d) => {
			const base = (img.split('/').pop() ?? img).split(':')[0];
			const cn = d.containerName || base;
			return {
				...d,
				dockerImage: img,
				name: d.name || base,
				containerName: cn,
				// Port mapping (-p) can't be inferred from the image — left for the user.
				startCmd: d.startCmd || `docker run --name ${cn} -d ${img}`,
			};
		});

	/** Bring Docker up (prompt-then-start), then reload the image list. */
	const startDockerAndReload = async () => {
		if (await ensureDockerDaemon()) {
			await listDockerImages().then(setImages).catch(() => {});
		}
	};

	const pickFolder = async () => {
		// Suppress hide-on-blur so the popover doesn't vanish behind the OS dialog.
		await setSuppressHide(true);
		let picked: string | string[] | null;
		try {
			picked = await openDialog({ directory: true });
		} finally {
			await setSuppressHide(false);
		}
		if (typeof picked !== 'string') return;
		const det = await detectFolder(picked);
		setData((d) => ({
			...d,
			dir: picked as string,
			name: d.name || det.name, // preserve a user-supplied name
			kind: det.kind,
			startCmd: det.startCmd,
			port: det.port,
			stack: det.stack,
		}));
		setPortText(det.port != null ? String(det.port) : '');
	};

	const save = async () => {
		// Docker items are joined to their container by name across status, stop,
		// and metrics — it must be present.
		if (data.kind === 'docker' && !data.containerName?.trim()) {
			alert('Container name is required for a Docker service.');
			return;
		}
		const result: ManagedItem = {
			...data,
			port: portText ? Number(portText) : null,
			env: parseEnv(envText),
			name: data.name,
			dir: data.dir || null,
			startCmd: data.startCmd || null,
			stopCmd: data.stopCmd || null,
			brewFormula: data.brewFormula || null,
			dockerImage: data.dockerImage || null,
			containerName: data.containerName?.trim() || null,
			healthPath: data.healthPath || null,
			group: data.group?.trim() || null,
		};
		// Adding a Docker service while the daemon is down: prompt to start it so the
		// service is ready to run. Cancelling still saves the config.
		if (!isEdit && data.kind === 'docker') {
			await ensureDockerDaemon();
		}
		try {
			if (isEdit) await updateItem(result);
			else await addItem(result);
			onOpenChange(false);
			onSaved();
		} catch (e) {
			alert(String(e));
		}
	};

	const isBrew = data.kind === 'brew';
	const isDocker = data.kind === 'docker';

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-h-[88vh] gap-0 overflow-y-auto sm:max-w-[340px]">
				<DialogHeader>
					<DialogTitle>{isEdit ? 'Edit service' : 'Add service'}</DialogTitle>
				</DialogHeader>

				<div className="grid gap-3 py-3">
					<Field label="Name">
						<Input value={data.name} onChange={(e) => set({ name: e.target.value })} />
					</Field>

					<Field label="Kind">
						<Select value={data.kind} onValueChange={(v) => set({ kind: v as ItemKind })}>
							<SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
							<SelectContent>
								<SelectItem value="project">Project</SelectItem>
								<SelectItem value="brew">Homebrew service</SelectItem>
								<SelectItem value="docker">Docker container</SelectItem>
								<SelectItem value="cli">CLI</SelectItem>
							</SelectContent>
						</Select>
					</Field>

					{!isBrew && !isDocker && (
						<Field label="Folder">
							<div className="flex gap-1.5">
								<Input value={data.dir ?? ''} readOnly placeholder="No folder" className="flex-1" />
								<Button type="button" variant="outline" size="sm" onClick={pickFolder}>
									Pick…
								</Button>
							</div>
						</Field>
					)}

					{isDocker && (
						<>
							<Field label="Image">
								<Input
									value={data.dockerImage ?? ''}
									onChange={(e) => pickImage(e.target.value)}
									list="docker-image-list"
									placeholder="repo:tag"
								/>
								<datalist id="docker-image-list">
									{images.map((img) => <option key={img} value={img} />)}
								</datalist>
								{images.length === 0 && (
									<button
										type="button"
										onClick={startDockerAndReload}
										className="mt-1 text-left text-[11px] text-muted-foreground underline-offset-2 hover:underline"
									>
										Docker isn't running — start it to list installed images
									</button>
								)}
							</Field>

							<Field label="Container name">
								<Input
									value={data.containerName ?? ''}
									onChange={(e) => set({ containerName: e.target.value })}
									placeholder="e.g. mongodb"
								/>
							</Field>
						</>
					)}

					<Field label="Start command">
						<Input value={data.startCmd ?? ''} onChange={(e) => set({ startCmd: e.target.value })} />
					</Field>

					<Field label="Stop command">
						<Input value={data.stopCmd ?? ''} onChange={(e) => set({ stopCmd: e.target.value })} />
					</Field>

					<Field label="Port">
						<Input type="number" value={portText} onChange={(e) => setPortText(e.target.value)} />
					</Field>

					{!isDocker && (
						<Field label="Run mode">
							<Select value={data.runMode} onValueChange={(v) => set({ runMode: v as RunMode })}>
								<SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
								<SelectContent>
									<SelectItem value="background">Background</SelectItem>
									<SelectItem value="terminal">Terminal</SelectItem>
								</SelectContent>
							</Select>
						</Field>
					)}

					{isBrew && (
						<Field label="Brew formula">
							<Input
								value={data.brewFormula ?? ''}
								onChange={(e) => set({ brewFormula: e.target.value })}
								list="brew-formula-list"
							/>
							<datalist id="brew-formula-list">
								{formulae.map((f) => <option key={f} value={f} />)}
							</datalist>
						</Field>
					)}

					<Field label="Env (KEY=VALUE per line)">
						<Textarea value={envText} onChange={(e) => setEnvText(e.target.value)} rows={3} className="font-mono text-xs" />
					</Field>

					<Field label="Health path">
						<Input value={data.healthPath ?? ''} onChange={(e) => set({ healthPath: e.target.value })} placeholder="/health" />
					</Field>

					<Field label="Group (optional)">
						<Input
							value={data.group ?? ''}
							onChange={(e) => set({ group: e.target.value })}
							list="group-name-list"
							placeholder="e.g. myapp"
						/>
						<datalist id="group-name-list">
							{groups.map((g) => <option key={g} value={g} />)}
						</datalist>
					</Field>

					<ToggleRow label="Favorite" checked={data.favorite} onChange={(v) => set({ favorite: v })} />
					<ToggleRow label="Auto-start on launch" checked={data.autoStart} onChange={(v) => set({ autoStart: v })} />
				</div>

				<DialogFooter>
					<Button variant="ghost" size="sm" onClick={() => onOpenChange(false)}>Cancel</Button>
					<Button size="sm" onClick={save}>Save</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

function Field({ label, children }: { label: string; children: React.ReactNode }): React.JSX.Element {
	return (
		<div className="grid gap-1.5">
			<Label className="text-xs text-muted-foreground">{label}</Label>
			{children}
		</div>
	);
}

function ToggleRow({
	label,
	checked,
	onChange,
}: {
	label: string;
	checked: boolean;
	onChange: (v: boolean) => void;
}): React.JSX.Element {
	return (
		<label className="flex items-center justify-between gap-2 text-[13px]">
			<span>{label}</span>
			<Switch checked={checked} onCheckedChange={onChange} />
		</label>
	);
}
