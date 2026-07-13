# Branding — app & menubar icons

Quay's icons are derived from a single "status buoy" glyph: a beacon dot, a sign,
the buoy body, and water waves. There are two products from it:

- **Menubar (tray) icon** — a monochrome macOS *template* image that auto-inverts
  black/white with the menubar's light/dark theme.
- **App / dock / Finder / DMG icon** — a dark navy→black gradient rounded-square with
  the white buoy line-art and a **green status beacon** (the buoy light).

## Sources (committed, reproducible)
- `src-tauri/icons/src/status-buoy.svg` — the monochrome glyph (root `color="#000000"`
  so `currentColor` renders black deterministically).
- `src-tauri/icons/src/tray.svg` — menubar-tuned variant of the glyph (scaled ~0.9 and
  nudged down so it sits optically centered next to neighbouring tray icons rather than
  reading top-heavy). This is the source for `tray.png`.
- `src-tauri/icons/src/tray-error.svg` / `tray-starting.svg` — attention variants of
  tray.svg: mid-grey body (`#8E8E93`) with the beacon dot glowing red (`#ef4444`) or
  amber (`#f59e0b`) plus a soft halo. Sources for `tray-error.png` / `tray-starting.png`,
  which the app swaps in at runtime (see Wiring).
- `src-tauri/icons/src/tray-waiting.svg` — the **agent-waiting** attention variant: the
  buoy sits low and half-submerged in a wavy grey water mound (only its top A-frame
  section shows), and the beacon is an enlarged amber core wrapped in a stepped
  translucent halo so it reads as a bright glow. The deliberate submerged-buoy +
  bigger-glow silhouette separates it from plain amber `tray-starting` at a glance.
  Source for `tray-waiting.png`.
- `src-tauri/icons/src/app-icon.svg` — the 1024×1024 master (gradient squircle, white
  buoy, green glowing beacon). Big Sur grid: 824×824 body, `r=185`, 100px margin; the
  glyph is mapped via `translate(182 197) scale(30)`.

## Regenerate

Menubar template icon (→ `src-tauri/icons/tray.png`, 44×44 = 22pt @2x, transparent RGBA)
and the colored attention variants — regenerate all four with one command:
```
./scripts/gen-icons.sh
```
It just wraps `rsvg-convert -w 44 -h 44 src/<name>.svg -o <name>.png` for `tray`,
`tray-error`, `tray-starting`, `tray-waiting` (needs `brew install librsvg`).

**The SVGs are the source of truth; the PNGs are a committed build artifact.**
`src-tauri/build.rs` (`regenerate_stale_tray_icons`) auto-regenerates a PNG via
`rsvg-convert` whenever its SVG is newer — so a `tauri dev` loop just picks up the edited
icon on the next rebuild, no manual step. It runs only for **debug** builds (local dev,
where icons get edited) and only when an SVG actually changed; release/bundle builds skip
it and ship the committed PNGs (so CI needs neither librsvg nor post-`git clone` mtime
fidelity). If `rsvg-convert` is missing it warns rather than crashing the dev server; a
PNG entirely absent is the only hard error. Still run `./scripts/gen-icons.sh` before
committing so the committed PNGs (what release embeds) match the SVGs.

App bundle icons (overwrites `32x32`, `128x128`, `128x128@2x`, `icon.png/.icns/.ico`,
and the Windows `Square*Logo`/`StoreLogo` PNGs):
```
rsvg-convert -w 1024 -h 1024 src-tauri/icons/src/app-icon.svg -o /tmp/icon-source.png
npm run tauri -- icon /tmp/icon-source.png
# Quay is macOS-only — delete the iOS/Android/64x64 assets tauri-cli also emits:
rm -rf src-tauri/icons/android src-tauri/icons/ios src-tauri/icons/64x64.png
```

## Wiring
The tray uses the template image, set in `src-tauri/src/lib.rs` (TrayIconBuilder):
```rust
.icon(tauri::include_image!("icons/tray.png"))
.icon_as_template(true)
```
`include_image!` decodes the PNG to raw RGBA at compile time (no extra Cargo feature);
the path is relative to the crate root (`src-tauri/`). `bundle.icon` in
`tauri.conf.json` is unchanged (same file paths).

The icon is dynamic: `update_tray_icon` in `lib.rs` folds service health together with
the waiting-agent count and swaps the icon via `set_icon_with_as_template`. Precedence
of the single glyph: **any service `Error` (red) > any waiting agent (`tray-waiting`) >
any service `Starting` (amber) > nominal**. The colored attention variants
(`tray-error` / `tray-waiting` / `tray-starting`) render with template mode **off**
(template images are forced monochrome, so color only shows non-template); nominal
restores `tray.png` with template mode on.

The waiting count comes from `AppState.waiting_count`, refreshed by
`refresh_waiting_badge` off the always-on poll loop (`health::spawn_poll_loop`) reading
`agent-state/*.json` — so the menubar reflects waiting agents even while the popover
(and its heavier radar scan) is closed. When agents are waiting and the
`waitingTitleBadge` setting is on, the menubar title also shows the count (e.g. `●2`)
via `set_title`; the toggle gates only the title text, not the icon.

`update_tray_icon` runs on every real status change (`commands::set_status`), on item
deletion, on a settings change (`commands::update_settings`, so the title toggle applies
at once), whenever the waiting count changes, and once after the tray is built at startup.
