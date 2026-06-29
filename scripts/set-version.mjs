#!/usr/bin/env node
/**
 * Write a single version number into every file that carries one, so the app
 * version stays in sync from one source of truth (the version semantic-release
 * computes). Invoked by @semantic-release/exec's prepareCmd:
 *
 *   node scripts/set-version.mjs <version>
 *
 * Updates:
 *   - package.json                "version"
 *   - src-tauri/tauri.conf.json   "version"  (drives the macOS bundle version)
 *   - src-tauri/Cargo.toml        version in the [package] table only
 *
 * Cargo.lock's root entry is refreshed by the subsequent `tauri build` (cargo
 * rewrites the lockfile when the manifest version changes) before the files are
 * committed. Pure Node — no BSD/GNU sed differences, safe on macOS and Linux.
 */
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+/.test(version)) {
	console.error(`set-version: invalid or missing version argument: ${version ?? "(none)"}`);
	process.exit(1);
}

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Update a top-level "version" key in a JSON file, preserving tab indentation. */
const setJsonVersion = (relPath) => {
	const path = join(root, relPath);
	const json = JSON.parse(readFileSync(path, "utf8"));
	json.version = version;
	writeFileSync(path, JSON.stringify(json, null, "\t") + "\n");
	console.log(`set-version: ${relPath} -> ${version}`);
};

/** Replace `version = "..."` inside the [package] table of a Cargo.toml only. */
const setCargoVersion = (relPath) => {
	const path = join(root, relPath);
	const lines = readFileSync(path, "utf8").split("\n");
	let inPackage = false;
	let done = false;
	const out = lines.map((line) => {
		const section = line.match(/^\s*\[([^\]]+)\]\s*$/);
		if (section) {
			inPackage = section[1] === "package";
			return line;
		}
		if (inPackage && !done && /^\s*version\s*=/.test(line)) {
			done = true;
			return line.replace(/version\s*=\s*"[^"]*"/, `version = "${version}"`);
		}
		return line;
	});
	if (!done) {
		console.error(`set-version: no [package] version found in ${relPath}`);
		process.exit(1);
	}
	writeFileSync(path, out.join("\n"));
	console.log(`set-version: ${relPath} [package] -> ${version}`);
};

setJsonVersion("package.json");
setJsonVersion("src-tauri/tauri.conf.json");
setCargoVersion("src-tauri/Cargo.toml");
