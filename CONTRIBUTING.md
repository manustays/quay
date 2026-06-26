# Contributing to Quay

Thanks for your interest in improving this project. This guide covers how to get set up, the conventions we follow, and how to get a change merged.

## Ground rules

- Be respectful and constructive.
- Open an **issue** before a large change so we can agree on the approach.
- Keep pull requests focused — one logical change per PR.
- This is a **macOS-only** app; changes must keep it building and running on macOS.

## Getting set up

You'll need:

- **Rust** (stable) via [rustup](https://rustup.rs) — `cargo` and `rustc` on your `PATH`.
- **Node.js 18+** and **npm**.
- **Xcode Command Line Tools** — `xcode-select --install`.

Then:

```bash
git clone https://github.com/manustays/menubar-cli-launcher.git
cd menubar-cli-launcher
npm install
npm run tauri dev      # launches the app in development mode
```

See [docs/development.md](docs/development.md) for the project layout and more detail.

## Branch naming

Use a prefix:

- `feature/<short-name>` — new functionality
- `bugfix/<short-name>` — fixes
- `chore/<short-name>` — tooling, docs, refactors

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add per-item restart action
fix: stop holding the running lock across child wait
docs: document the config.json schema
chore: bump tauri to 2.x
```

Keep the subject under ~50 characters; add a body when the "why" isn't obvious.

## Before you open a PR

Run the full check suite and make sure it's green:

```bash
# Rust unit tests
cargo test --manifest-path src-tauri/Cargo.toml

# Frontend unit tests
npm test

# TypeScript type-check
npx tsc --noEmit

# Rust build (catches warnings)
cargo build --manifest-path src-tauri/Cargo.toml
```

Guidelines:

- **Add or update tests** for behavior you change. Pure logic (parsing, status decisions, config round-trips) is unit-tested; favor table-driven tests there.
- **No `any` in TypeScript.** Type DOM queries and IPC payloads.
- **Tabs for indentation** in both Rust and TypeScript (project convention).
- **Doc comments** — `///` on public Rust items, JSDoc on exported TS functions.
- **Keep the IPC contract in sync** — a Rust command's name/args and serde field names (camelCase) must match the TypeScript wrappers in `src/ipc.ts` and interfaces in `src/model.ts`. A mismatch silently breaks a feature at runtime; the type checker won't catch it.

## Security-sensitive areas

- `src-tauri/src/terminal.rs` builds shell lines passed to `osascript`. Keep the single-quote and AppleScript escaping intact.
- The frontend must use `textContent` (not `innerHTML`) for any process- or user-supplied string.
- Tauri capabilities (`src-tauri/capabilities/default.json`) should grant only what's needed.

## Project documentation

When you add or change a core feature, update the relevant doc under `docs/` (and the [design spec](docs/specs/2026-06-26-menubar-service-manager-design.md) if the behavior changes).

## Reporting bugs

Open an issue with:

- macOS version and chip (Apple Silicon / Intel)
- What you did, what you expected, what happened
- Relevant log output — per-item logs live at
  `~/Library/Application Support/com.abhi.quay/logs/<id>.log`

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
