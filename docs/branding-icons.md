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
- `src-tauri/icons/src/app-icon.svg` — the 1024×1024 master (gradient squircle, white
  buoy, green glowing beacon). Big Sur grid: 824×824 body, `r=185`, 100px margin; the
  glyph is mapped via `translate(182 197) scale(30)`.

## Regenerate

Menubar template icon (→ `src-tauri/icons/tray.png`, 44×44 = 22pt @2x, transparent RGBA):
```
rsvg-convert -w 44 -h 44 src-tauri/icons/src/tray.svg -o src-tauri/icons/tray.png
```

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
