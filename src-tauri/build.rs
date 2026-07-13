fn main() {
	// The quay-hook helper is bundled as a resource (see tauri.conf.json) and
	// tauri-build fails compilation if the file is absent. `npm run hook:build`
	// (run by beforeDev/BuildCommand) fills it with the real binary for bundles;
	// here we ensure at least an empty placeholder exists so a bare `cargo build`
	// / `cargo test` still compiles on a fresh checkout (binaries/ is gitignored).
	let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
	let hook = std::path::Path::new(&manifest).join("binaries/quay-hook");
	if !hook.exists() {
		let _ = std::fs::create_dir_all(hook.parent().unwrap());
		let _ = std::fs::write(&hook, b"");
	}
	regenerate_stale_tray_icons(std::path::Path::new(&manifest));
	tauri_build::build()
}

/// Keep the tray PNGs in sync with their SVG sources. The SVGs are the source of
/// truth; the committed PNGs are what `include_image!` embeds at compile time. When an
/// SVG is edited without regenerating its PNG, regenerate it here via `rsvg-convert`
/// (the same tool `scripts/gen-icons.sh` uses) so a `tauri dev` loop just picks up the
/// new icon on the next rebuild — no manual step, no fatal build.
///
// ponytail: mtime-triggered + best-effort shell-out to rsvg-convert. No Cargo dep — it
// only runs for debug builds (local dev, where icons get edited) and only when an SVG
// is actually newer than its PNG. Release/bundle builds skip it entirely and ship the
// committed PNGs (so CI needs neither librsvg nor mtime fidelity after a `git clone`).
// If rsvg-convert is missing we warn rather than crash the dev server; the PNG entirely
// absent is the only hard error. `scripts/gen-icons.sh` is still the pre-commit regen.
fn regenerate_stale_tray_icons(manifest: &std::path::Path) {
	let debug = std::env::var("PROFILE").as_deref() == Ok("debug");
	for name in ["tray", "tray-error", "tray-starting", "tray-waiting"] {
		let svg = manifest.join(format!("icons/src/{name}.svg"));
		let png = manifest.join(format!("icons/{name}.png"));
		// Watch only the SVG: it is the source, and we regenerate the PNG from it.
		// (Listing the PNG too would re-trigger this build the moment we rewrite it.)
		println!("cargo:rerun-if-changed={}", svg.display());
		if !debug {
			continue;
		}
		let svg_m = std::fs::metadata(&svg).and_then(|m| m.modified());
		let png_m = std::fs::metadata(&png).and_then(|m| m.modified());
		let stale = match (&svg_m, &png_m) {
			(Ok(s), Ok(p)) => s > p, // SVG edited after the PNG was generated
			(Ok(_), Err(_)) => true, // PNG missing entirely
			_ => return,             // SVG missing: not this guard's concern
		};
		if !stale {
			continue;
		}
		let rendered = std::process::Command::new("rsvg-convert")
			.args(["-w", "44", "-h", "44"])
			.arg(&svg)
			.arg("-o")
			.arg(&png)
			.status()
			.map(|s| s.success())
			.unwrap_or(false);
		if rendered {
			println!("cargo:warning=regenerated icons/{name}.png from its SVG");
		} else if png_m.is_err() {
			panic!(
				"icons/{name}.png is missing and rsvg-convert failed to render it.\n\
				 Install librsvg (`brew install librsvg`) or run ./scripts/gen-icons.sh."
			);
		} else {
			println!(
				"cargo:warning=icons/{name}.png is stale and rsvg-convert is unavailable — \
				 run ./scripts/gen-icons.sh (needs `brew install librsvg`)"
			);
		}
	}
}
