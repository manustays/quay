# Development

How to set up, run, test, and navigate the codebase.

## Prerequisites

- **Rust** (stable) via [rustup](https://rustup.rs) — `cargo` + `rustc` on your `PATH`.
- **Node.js 22.14+ (22.x)** and **npm**, matching the Node 22 release workflow.
- **Xcode Command Line Tools** — `xcode-select --install`.

Verify `cargo --version` works in your shell. If `cargo` isn't found even after installing Rust, ensure your toolchain's `bin` directory is on `PATH` (standard rustup installs add `~/.cargo/bin`) — see [Troubleshooting](troubleshooting.md#cargo-not-found).

## Run in dev mode

```bash
npm ci
npm run tauri dev
```

The first build downloads and compiles the Rust dependencies and may take several minutes or longer. Subsequent runs reuse the build cache. The Tauri `beforeDevCommand` builds and stages the `quay-hook` helper before starting Vite. Vite serves the frontend on `http://localhost:1420` with hot-reload; Rust changes trigger a rebuild via the Tauri watcher.

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

Run all four green before opening a PR. To include the separate `quay-hook` workspace member, also run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --workspace
```

For a production frontend build, run `npm run build`. The unit tests cover the pure logic — folder detection, the `brew services list` parser, the status-decision function, config round-trips, the process supervisor, and the frontend helpers.

## Project layout

Selected entry points and modules (not an exhaustive file list):

```
.
├── index.html                # webview entry
├── package.json              # frontend deps + scripts
├── vite.config.ts
├── tsconfig.json
├── src/                      # React + TypeScript frontend
│   ├── main.tsx              # mount App and load fonts/styles
│   ├── App.tsx               # live state, event subscriptions, dialogs
│   ├── ipc.ts                # typed wrappers over Tauri invoke/listen
│   ├── model.ts              # TS mirrors of Rust types + pure helpers
│   ├── model.test.ts         # Vitest tests for model helpers
│   ├── components/
│   │   ├── Popup.tsx         # search, groups, favorites, radar, drag-to-reorder
│   │   ├── ServiceRow.tsx    # service actions, metrics, log tail
│   │   ├── ServiceForm.tsx   # add/edit dialog + folder detection
│   │   ├── SettingsDialog.tsx # settings, autostart, agent hooks
│   │   ├── DetectedRow.tsx   # unmanaged port actions
│   │   ├── AgentRow.tsx      # coding-agent session actions
│   │   ├── UpdateBanner.tsx  # update notification and installation
│   │   └── ui/              # shared Radix/shadcn UI components
│   ├── lib/                 # environment parsing, Docker prompts, styling helpers
│   └── index.css            # Tailwind styles and theme
├── src-tauri/                # Rust core
│   ├── crates/quay-hook/     # agent hook helper and process identity library
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
- **Rust → Frontend:** Tauri **events** — the backend emits `status_changed` with an `ItemStatus` payload; `App.tsx` subscribes through `src/ipc.ts` and updates React state. Metrics, discovered ports, and agent sessions also arrive as events.

> **Keep the IPC contract in sync.** A Rust command's name and parameters, and the serde field names (camelCase) on payload structs, must match the strings/interfaces in `src/ipc.ts` and `src/model.ts`. The TypeScript compiler can't catch a mismatch here — it shows up only at runtime.

Use `src/ipc.ts` for the frontend command/event wrappers and `src-tauri/src/lib.rs` for the registered command list. Handlers live in `src-tauri/src/commands.rs`; these source files are the reference when adding or changing an IPC call.

## Conventions

- **Tabs** for indentation (Rust and TS).
- **Doc comments:** `///` on public Rust items, JSDoc on exported TS functions.
- **No `any`** in TypeScript; type DOM queries and IPC payloads.
- **Conventional commits**; `feature/` `bugfix/` `chore/` branch prefixes.
- Render user/process strings as React text children; do not interpolate them into HTML or use `dangerouslySetInnerHTML`.

## Dependencies

**Rust** (`src-tauri/Cargo.toml`): Tauri and its plugins provide the desktop integration; `serde`/`serde_json` handle IPC and persistence; `sysinfo` supplies process metrics; `ureq` handles HTTP; macOS integration uses the `objc2` crates. The local `quay-hook` workspace member provides shared process-identity helpers.

**Frontend** (`package.json`): React and React DOM render the UI; Radix/shadcn components and Tailwind provide its UI primitives and styling; `@tauri-apps/*` packages provide IPC and plugin access. Vite builds the frontend, TypeScript checks types, and Vitest runs unit tests. Consult the manifests and lockfiles for the complete dependency lists and versions.

## Further reading

- [Architecture](architecture.md) — the design in more depth.
