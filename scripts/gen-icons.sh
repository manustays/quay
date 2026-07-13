#!/usr/bin/env bash
# Regenerate the tray PNGs from their SVG sources (the source of truth).
# The committed PNGs are what `include_image!` embeds at compile time; build.rs
# fails a debug build if an SVG is newer than its PNG, pointing here.
#
# Requires librsvg (`brew install librsvg`).
set -euo pipefail
cd "$(dirname "$0")/.."

for name in tray tray-error tray-starting tray-waiting; do
	rsvg-convert -w 44 -h 44 "src-tauri/icons/src/$name.svg" -o "src-tauri/icons/$name.png"
	echo "generated src-tauri/icons/$name.png"
done
