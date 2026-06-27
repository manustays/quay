# Multi-terminal detection & launch — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Detect installed macOS terminals (Terminal, iTerm, Ghostty, Kitty, WezTerm, Alacritty) and let the user pick any installed one to run services / open folders in.

**Architecture:** A single static registry in `terminal.rs` is the source of truth for both detection and launch. Detection checks each terminal's `.app` bundle (and a `command -v` CLI fallback). Launch dispatches by strategy: AppleScript for Terminal.app/iTerm (unchanged), or `open -na "<App>" --args … /bin/zsh -ilc "<line>; exec /bin/zsh -il"` for the GPU terminals. A new `get_terminals` IPC command feeds the settings dropdown.

**Tech Stack:** Rust (Tauri v2 backend), React + TypeScript + shadcn/ui (`Select`).

## Global Constraints

- Build: `cargo build --manifest-path src-tauri/Cargo.toml` (cargo on default PATH; no prefix).
- Rust tests: `cargo test --manifest-path src-tauri/Cargo.toml`.
- Frontend typecheck: `npx tsc --noEmit`. Frontend tests: `npm test`.
- Indentation: **tabs** (matches existing Rust + TSX files).
- Commit identity: plain `git commit` (inherits repo's noreply email); conventional-commit messages; NO `Co-Authored-By` trailer.
- macOS only. Existing `Settings::terminal_app` stays a `String` (display name); no schema/migration change.
- Reuse existing `build_command_line(dir, env, cmd)` in `terminal.rs` — do not reimplement cd/env logic.

---

### Task 1: Terminal registry + launch dispatch (Rust)

Replace the two-branch `match` in `run_in_terminal` with a registry-driven dispatch, and add pure, unit-tested argv builders for the CLI terminals. Detection (`installed_terminals`) comes in Task 2; this task only adds the registry + launch.

**Files:**
- Modify: `src-tauri/src/terminal.rs`

**Interfaces:**
- Consumes: `build_command_line(dir, env, cmd) -> String` (existing, unchanged).
- Produces (used by Task 2 & dispatch):
  - `struct TermSpec { name: &'static str, app_bundle: &'static str, cli_bin: Option<&'static str>, strategy: Strategy, always: bool }`
  - `enum Strategy { AppleDoScript, AppleWriteText, Cli { args_before_shell: &'static [&'static str] } }`
  - `fn registry() -> &'static [TermSpec]`
  - `fn spec(name: &str) -> Option<&'static TermSpec>`
  - `fn keepalive_shell(line: &str) -> [String; 3]`
  - `fn cli_open_argv(open_name: &str, args_before_shell: &[&str], line: &str) -> Vec<String>`
  - `fn cli_bin_argv(args_before_shell: &[&str], line: &str) -> Vec<String>`
  - `run_in_terminal(app_name, dir, env, cmd)` and `open_folder` keep their existing signatures.

- [ ] **Step 1: Write failing tests**

Add to the `tests` module at the bottom of `src-tauri/src/terminal.rs` (keep the existing `builds_command_with_env_exports` test):

```rust
	#[test]
	fn keepalive_shell_runs_line_then_keeps_open() {
		let s = keepalive_shell("cd '/tmp' && npm run dev");
		assert_eq!(s[0], "/bin/zsh");
		assert_eq!(s[1], "-ilc");
		assert_eq!(s[2], "cd '/tmp' && npm run dev; exec /bin/zsh -il");
	}

	#[test]
	fn cli_open_argv_wraps_open_na_with_prefix() {
		let v = cli_open_argv("WezTerm", &["start", "--"], "echo hi");
		assert_eq!(
			v,
			vec![
				"-na", "WezTerm", "--args", "start", "--",
				"/bin/zsh", "-ilc", "echo hi; exec /bin/zsh -il",
			]
		);
	}

	#[test]
	fn cli_bin_argv_omits_open_wrapper() {
		let v = cli_bin_argv(&["-e"], "echo hi");
		assert_eq!(v, vec!["-e", "/bin/zsh", "-ilc", "echo hi; exec /bin/zsh -il"]);
	}

	#[test]
	fn registry_lookup_is_case_sensitive_by_name() {
		assert!(spec("Ghostty").is_some());
		assert!(spec("nope").is_none());
		assert_eq!(spec("Terminal").unwrap().app_bundle, "Terminal.app");
	}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml terminal::`
Expected: FAIL — `cannot find function keepalive_shell` / `cli_open_argv` / `cli_bin_argv` / `spec`.

- [ ] **Step 3: Add the registry, strategy, and pure builders**

At the top of `src-tauri/src/terminal.rs` (after the existing `use` lines), add:

```rust
/// How a terminal is launched.
enum Strategy {
	/// AppleScript `do script` (Terminal.app).
	AppleDoScript,
	/// AppleScript `write text` to a new iTerm session.
	AppleWriteText,
	/// GUI app run via `open -na "<App>" --args <args_before_shell…> <shell>`,
	/// or the bare CLI binary + the same prefix when only the CLI is on PATH.
	Cli { args_before_shell: &'static [&'static str] },
}

/// A supported terminal: the display `name` (also what `Settings::terminal_app`
/// stores), its macOS `.app` bundle file name, an optional CLI binary name for
/// the `command -v` detection fallback, the launch `strategy`, and `always`
/// (true only for Terminal.app, which ships with macOS).
struct TermSpec {
	name: &'static str,
	app_bundle: &'static str,
	cli_bin: Option<&'static str>,
	strategy: Strategy,
	always: bool,
}

/// All terminals Quay knows how to detect and launch. Single source of truth.
fn registry() -> &'static [TermSpec] {
	&[
		TermSpec { name: "Terminal", app_bundle: "Terminal.app", cli_bin: None,
			strategy: Strategy::AppleDoScript, always: true },
		TermSpec { name: "iTerm", app_bundle: "iTerm.app", cli_bin: None,
			strategy: Strategy::AppleWriteText, always: false },
		TermSpec { name: "Ghostty", app_bundle: "Ghostty.app", cli_bin: Some("ghostty"),
			strategy: Strategy::Cli { args_before_shell: &["-e"] }, always: false },
		TermSpec { name: "Kitty", app_bundle: "kitty.app", cli_bin: Some("kitty"),
			strategy: Strategy::Cli { args_before_shell: &[] }, always: false },
		TermSpec { name: "WezTerm", app_bundle: "WezTerm.app", cli_bin: Some("wezterm"),
			strategy: Strategy::Cli { args_before_shell: &["start", "--"] }, always: false },
		TermSpec { name: "Alacritty", app_bundle: "Alacritty.app", cli_bin: Some("alacritty"),
			strategy: Strategy::Cli { args_before_shell: &["-e"] }, always: false },
	]
}

/// Look up a terminal spec by its display name.
fn spec(name: &str) -> Option<&'static TermSpec> {
	registry().iter().find(|t| t.name == name)
}

/// The interactive login-shell command that runs `line` and then re-execs an
/// interactive shell so the window stays open afterwards (parity with
/// Terminal.app's `do script`). `-ilc` sources `~/.zshrc`, so PATH resolves.
fn keepalive_shell(line: &str) -> [String; 3] {
	[
		"/bin/zsh".into(),
		"-ilc".into(),
		format!("{line}; exec /bin/zsh -il"),
	]
}

/// Argv for launching a GUI terminal via `open -na "<open_name>" --args …`.
fn cli_open_argv(open_name: &str, args_before_shell: &[&str], line: &str) -> Vec<String> {
	let mut v = vec!["-na".to_string(), open_name.to_string(), "--args".to_string()];
	v.extend(args_before_shell.iter().map(|s| s.to_string()));
	v.extend(keepalive_shell(line));
	v
}

/// Argv for launching a terminal by its bare CLI binary (PATH fallback): the
/// per-terminal prefix followed by the keep-alive shell.
fn cli_bin_argv(args_before_shell: &[&str], line: &str) -> Vec<String> {
	let mut v: Vec<String> = args_before_shell.iter().map(|s| s.to_string()).collect();
	v.extend(keepalive_shell(line));
	v
}
```

- [ ] **Step 4: Add detection helpers + CLI launch, rewrite `run_in_terminal`**

Add these helpers (used by both launch and Task 2's `installed_terminals`):

```rust
/// True if `<bundle>` exists in `/Applications` or `~/Applications`.
fn app_bundle_installed(bundle: &str) -> bool {
	let home = std::env::var("HOME").unwrap_or_default();
	std::path::Path::new("/Applications").join(bundle).exists()
		|| std::path::Path::new(&home).join("Applications").join(bundle).exists()
}

/// Absolute path of `bin` per the login shell, or `None` if not found. Mirrors
/// the `command -v` probe used by `brew`/`docker` resolution.
fn resolve_cli(bin: &str) -> Option<String> {
	let out = Command::new("/bin/zsh")
		.args(["-lc", &format!("command -v {bin}")])
		.output()
		.ok()?;
	if !out.status.success() {
		return None;
	}
	let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
	(!p.is_empty()).then_some(p)
}
```

Replace the current body of `run_in_terminal` (the `let script = match app_name { … }` block and the `osascript` spawn, lines ~36-56) with strategy dispatch. The function keeps its signature:

```rust
	let line = build_command_line(dir, env, cmd);
	let spec = spec(app_name)
		.ok_or_else(|| AppError::Message(format!("unknown terminal: {app_name}")))?;
	match &spec.strategy {
		Strategy::AppleDoScript | Strategy::AppleWriteText => {
			let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
			// `matches!` borrows — don't move the non-Copy strategy out of `&spec`.
			let script = if matches!(spec.strategy, Strategy::AppleWriteText) {
				format!("tell application \"iTerm\"\n create window with default profile\n tell current session of current window to write text \"{escaped}\"\nend tell")
			} else {
				format!("tell application \"Terminal\"\n activate\n do script \"{escaped}\"\nend tell")
			};
			run_osascript(&script)
		}
		Strategy::Cli { args_before_shell } => {
			if app_bundle_installed(spec.app_bundle) {
				let open_name = spec.app_bundle.strip_suffix(".app").unwrap_or(spec.app_bundle);
				run_command("open", &cli_open_argv(open_name, args_before_shell, &line))
			} else if let Some(bin) = spec.cli_bin.and_then(resolve_cli) {
				run_command(&bin, &cli_bin_argv(args_before_shell, &line))
			} else {
				Err(AppError::Message(format!("{} is not installed", spec.name)))
			}
		}
	}
```

Add two small spawn helpers (keep the existing stderr-on-failure behavior):

```rust
/// Run `osascript -e <script>`, returning stderr text on failure.
fn run_osascript(script: &str) -> Result<(), AppError> {
	run_command("osascript", &["-e".to_string(), script.to_string()])
}

/// Spawn `program` with `args`, mapping a non-zero exit to its stderr.
fn run_command(program: &str, args: &[String]) -> Result<(), AppError> {
	let out = Command::new(program)
		.args(args)
		.output()
		.map_err(|e| AppError::Message(format!("{program} failed: {e}")))?;
	if out.status.success() {
		Ok(())
	} else {
		Err(AppError::Message(String::from_utf8_lossy(&out.stderr).trim().to_string()))
	}
}
```

Update the `run_in_terminal` doc comment (lines ~25-28) to mention registry dispatch instead of "Terminal.app / iTerm" only. Leave `open_folder` and `build_command_line` as-is.

- [ ] **Step 5: Run tests + build, verify pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml terminal::`
Expected: PASS (5 tests).
Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: `Finished` with no errors (warnings for not-yet-used `installed_terminals` come in Task 2; `app_bundle_installed`/`always` may warn as unused until Task 2 — acceptable mid-plan, resolved in Task 2).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/terminal.rs
git commit -m "feat: registry-driven terminal launch with CLI terminal support"
```

---

### Task 2: Detection command (`get_terminals`)

Expose the installed terminals to the frontend.

**Files:**
- Modify: `src-tauri/src/terminal.rs` (add `installed_terminals`)
- Modify: `src-tauri/src/commands.rs` (add `get_terminals` command)
- Modify: `src-tauri/src/lib.rs` (register in `invoke_handler`)

**Interfaces:**
- Consumes: `registry()`, `app_bundle_installed`, `resolve_cli` from Task 1.
- Produces: `pub fn installed_terminals() -> Vec<String>`; Tauri command `get_terminals() -> Result<Vec<String>, AppError>`.

- [ ] **Step 1: Add `installed_terminals` to `terminal.rs`**

```rust
/// Display names of every supported terminal that is installed: Terminal.app
/// (always), any whose `.app` bundle is present, or whose CLI binary is on the
/// login-shell PATH (covers brew-installed CLIs with no `.app`).
pub fn installed_terminals() -> Vec<String> {
	registry()
		.iter()
		.filter(|t| {
			t.always
				|| app_bundle_installed(t.app_bundle)
				|| t.cli_bin.map(|b| resolve_cli(b).is_some()).unwrap_or(false)
		})
		.map(|t| t.name.to_string())
		.collect()
}
```

- [ ] **Step 2: Add the `get_terminals` command in `commands.rs`**

Add near `open_terminal` (after `tail_log`, around line 341):

```rust
/// List installed terminal apps for the settings picker. Terminal.app is always
/// present; others appear when their `.app` bundle or CLI binary is found.
#[tauri::command]
pub fn get_terminals() -> Vec<String> {
	terminal::installed_terminals()
}
```

(`terminal` is already imported in `commands.rs` — it's used by `open_terminal`/`start_item`. If the build reports it unresolved, add `use crate::terminal;` with the other `use` lines.)

- [ ] **Step 3: Register the command in `lib.rs`**

In the `tauri::generate_handler![…]` list (around line 64), add `commands::get_terminals,` after `commands::open_terminal,`:

```rust
			commands::open_terminal,
			commands::get_terminals,
			commands::tail_log,
```

- [ ] **Step 4: Build, verify pass**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: `Finished`, no errors, no unused-fn warnings for `installed_terminals`/`app_bundle_installed`.
Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: all tests PASS (same count as Task 1 + existing suite).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/terminal.rs src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: get_terminals command to detect installed terminals"
```

---

### Task 3: Settings dropdown wiring (frontend)

Populate the Terminal-app dropdown from `get_terminals` instead of the two hardcoded entries.

**Files:**
- Modify: `src/ipc.ts` (add `getTerminals`)
- Modify: `src/components/SettingsDialog.tsx` (fetch + render dynamically)

**Interfaces:**
- Consumes: Tauri command `get_terminals` (Task 2).
- Produces: `getTerminals(): Promise<string[]>`.

- [ ] **Step 1: Add `getTerminals` to `ipc.ts`**

After the `getSettings` / `updateSettings` lines (around line 20):

```ts
/** List terminal apps detected as installed, for the settings picker. */
export const getTerminals = () => invoke<string[]>('get_terminals');
```

- [ ] **Step 2: Fetch the list in `SettingsDialog.tsx`**

Add to the imports from `../ipc` (line 22):

```ts
import { getSettings, updateSettings, getTerminals } from '../ipc';
```

Add state + load alongside the existing `settings` state/effect (after line 32 and within the existing `open` effect at lines 34-36):

```tsx
	const [terminals, setTerminals] = useState<string[]>([]);

	useEffect(() => {
		if (open) {
			void getSettings().then(setSettings);
			void getTerminals().then(setTerminals);
		}
	}, [open]);
```

(Replace the existing `useEffect(() => { if (open) void getSettings().then(setSettings); }, [open]);` with the block above.)

- [ ] **Step 3: Render options dynamically**

Replace the hardcoded `<SelectContent>` (lines 66-69) with a mapped list that also keeps the currently-saved value visible even if it is no longer installed:

```tsx
								<SelectContent>
									{Array.from(new Set([...terminals, settings.terminalApp]))
										.filter(Boolean)
										.map((name) => (
											<SelectItem key={name} value={name}>{name}</SelectItem>
										))}
								</SelectContent>
```

- [ ] **Step 4: Typecheck + tests**

Run: `npx tsc --noEmit`
Expected: exit 0, no errors.
Run: `npm test`
Expected: existing tests PASS (3 passed).

- [ ] **Step 5: Commit**

```bash
git add src/ipc.ts src/components/SettingsDialog.tsx
git commit -m "feat: populate terminal picker from detected terminals"
```

---

## Manual verification (real device — by user)

1. `npm run tauri dev`. Open Settings → "Terminal app": only installed terminals appear (e.g. Terminal, plus any of Ghostty/Kitty/WezTerm/Alacritty/iTerm you have).
2. Pick each installed terminal; for a Terminal-run-mode service, Start it → that terminal opens, `cd`s into the dir, runs the command, **and stays open** after it exits.
3. "Open terminal" row action opens the selected terminal in the item's folder.
4. CLI-arg sanity (esp. Ghostty `-e`): if a terminal opens but the command doesn't run, adjust that spec's `args_before_shell` in `registry()` — this is the one part not verifiable in CI.

## Notes / caveats

- Ghostty/Kitty/WezTerm/Alacritty argv contracts are encoded once in `registry()`; fixing a wrong one is a single-line edit there.
- No config migration: `terminal_app` remains a free-form display-name string; existing configs keep working.
