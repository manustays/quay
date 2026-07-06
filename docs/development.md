# Development

How to set up, run, test, and navigate the codebase.

## Prerequisites

- **Rust** (stable) via [rustup](https://rustup.rs) — `cargo` + `rustc` on your `PATH`.
- **Node.js 18+** and **npm**.
- **Xcode Command Line Tools** — `xcode-select --install`.

Verify `cargo --version` works in your shell. If `cargo` isn't found even after installing Rust, ensure your toolchain's `bin` directory is on `PATH` (standard rustup installs add `~/.cargo/bin`) — see [Troubleshooting](troubleshooting.md#cargo-not-found).

## Run in dev mode

```bash
npm install
npm run tauri dev
```

The first build compiles all Rust crates (a couple of minutes); subsequent runs are fast. Vite serves the frontend on `http://localhost:1420` with hot-reload; Rust changes trigger a rebuild via the Tauri watcher.

Open the webview devtools (right-click inside the popover → Inspect, in dev builds) to debug the frontend.

## Tests & checks

```bash
# Rust unit tests
cargo test --manifest-path src-tauri/Cargo.toml

# Frontend unit tests (vitest)
npm test

# TypeScript type-check (no emit)
npx tsc --noEmit

# Rust build (surfaces warnings)
cargo build --manifest-path src-tauri/Cargo.toml
```

Run all four green before opening a PR. The unit tests cover the pure logic — folder detection, the `brew services list` parser, the status-decision function, config round-trips, the process supervisor, and the frontend helpers.

## Project layout

```
.
├── index.html                # webview entry
├── package.json              # frontend deps + scripts
├── vite.config.ts
├── tsconfig.json
├── src/                      # frontend (vanilla TypeScript)
│   ├── main.ts               # bootstrap: load items, render, subscribe to status events
│   ├── ipc.ts                # typed wrappers over Tauri invoke/listen
│   ├── model.ts              # TS mirrors of the Rust types + pure helpers (tested)
│   ├── list.ts               # the two-tier list, search, stop-all
│   ├── row.ts                # one item row + expand panel + actions
│   ├── form.ts               # add/edit modal + folder pick + detect prefill
│   ├── settings.ts           # settings modal + launch-at-login
│   ├── model.test.ts         # vitest tests for model.ts helpers
│   └── styles.css
├── src-tauri/                # Rust core
│   ├── Cargo.toml
│   ├── tauri.conf.json       # app + bundle config
│   ├── capabilities/default.json  # Tauri permission grants
│   ├── icons/
│   └── src/
│       ├── lib.rs            # Tauri builder: plugins, tray, popover, commands, run loop
│       ├── main.rs           # thin entry calling lib::run()
│       ├── model.rs          # ManagedItem, Settings, AppConfig, Status, AppError
│       ├── store.rs          # load/save config.json (atomic; corrupt-file recovery)
│       ├── detect.rs         # inspect a folder → suggested name/cmd/port/kind
│       ├── brew.rs           # brew services wrappers + list parser
│       ├── supervisor.rs     # spawn/stop background children (own process group)
│       ├── health.rs         # status-decision fn, port/HTTP probes, poll loop
│       ├── terminal.rs       # drive Terminal.app / iTerm via osascript
│       ├── state.rs          # AppState (shared mutable state behind Mutexes)
│       └── commands.rs       # all #[tauri::command] handlers + init_state
└── docs/                     # this documentation
```

Each Rust module has one responsibility. Pure logic lives in `detect`, `brew`, `health::decide_status`, and `store` and is unit-tested directly.

## How the pieces talk

- **Frontend → Rust:** Tauri **commands** (`invoke('start_item', { id })`), wrapped with types in `src/ipc.ts`.
- **Rust → Frontend:** Tauri **events** — the backend emits `status_changed` with an `ItemStatus` payload; `main.ts` listens and re-renders.

> **Keep the IPC contract in sync.** A Rust command's name and parameters, and the serde field names (camelCase) on payload structs, must match the strings/interfaces in `src/ipc.ts` and `src/model.ts`. The TypeScript compiler can't catch a mismatch here — it shows up only at runtime.

The full list of commands (in `src-tauri/src/commands.rs`, registered in `lib.rs`):
`get_items`, `add_item`, `update_item`, `delete_item`, `reorder`, `toggle_favorite`, `detect_folder_cmd`, `get_settings`, `update_settings`, `start_item`, `stop_item`, `stop_all`, `open_browser`, `open_terminal`, `tail_log`, `list_brew_formulae`, `set_suppress_hide`.

## Conventions

- **Tabs** for indentation (Rust and TS).
- **Doc comments:** `///` on public Rust items, JSDoc on exported TS functions.
- **No `any`** in TypeScript; type DOM queries and IPC payloads.
- **Conventional commits**; `feature/` `bugfix/` `chore/` branch prefixes.
- Frontend uses `textContent` (never `innerHTML` with user/process data).

## Dependencies

**Rust** (`src-tauri/Cargo.toml`): `tauri` (+`macos-private-api`), `tauri-plugin-positioner` (+`tray-icon`), `tauri-plugin-dialog`, `tauri-plugin-autostart`, `serde`/`serde_json`, `dirs`, `uuid` (v4), `libc`, `ureq`.

**Frontend** (`package.json`): `@tauri-apps/api`, `@tauri-apps/plugin-positioner`, `@tauri-apps/plugin-dialog`, `@tauri-apps/plugin-autostart`; dev: `@tauri-apps/cli`, `typescript`, `vite`, `vitest`.

## Further reading

- [Architecture](architecture.md) — the design in more depth.
