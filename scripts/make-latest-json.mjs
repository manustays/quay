#!/usr/bin/env node
/**
 * Generate the Tauri updater manifest (`latest.json`) from the freshly-built
 * universal macOS bundle, then leave it at the repo root for
 * @semantic-release/github to publish as a release asset.
 *
 * Invoked by @semantic-release/exec's prepareCmd AFTER scripts/release-build.sh,
 * so the version in package.json is already the release version and the bundler
 * has emitted the `.app.tar.gz` + `.app.tar.gz.sig` (the latter only when
 * TAURI_SIGNING_PRIVATE_KEY is set — see .github/workflows/release.yml).
 *
 * Fails hard (non-zero exit) when the tarball or its signature is missing or
 * empty: better to abort the release than publish a `latest.json` that points at
 * an unsigned/absent asset and breaks auto-update for every installed client.
 *
 * Asset URLs use the deterministic tag form `releases/download/v<version>/<file>`
 * (not `releases/latest/download/...`) so the manifest always resolves to the
 * exact release it was built for, even mid-publish. Pure Node — no glob/path deps.
 */
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/**
 * Extract the newest release section from a keep-a-changelog / semantic-release
 * CHANGELOG. Release headings are level 1-2 followed by a version (`# [0.15.0]…`
 * or `## [0.13.1]…`); the `### Features` / `### Bug Fixes` subsections stay part
 * of the body. Returns the body between the first release heading and the next,
 * trimmed — empty string if no release heading is found.
 */
function latestChangelogSection(md) {
	const lines = md.split("\n");
	const isRelease = (l) => /^#{1,2} \[?\d+\.\d+\.\d+/.test(l);
	const start = lines.findIndex(isRelease);
	if (start === -1) return "";
	const rest = lines.slice(start + 1);
	const end = rest.findIndex(isRelease);
	return (end === -1 ? rest : rest.slice(0, end)).join("\n").trim();
}

// Self-test runs before any bundle/signature side effects so it works on a
// machine with no built artifacts: `node scripts/make-latest-json.mjs --selftest`.
if (process.argv[2] === "--selftest") {
	const { strict: assert } = await import("node:assert");
	const fixture = [
		"# [0.15.0](http://x/compare/v0.14.0...v0.15.0) (2026-07-06)",
		"",
		"### Features",
		"",
		"* new thing ([abc](http://x))",
		"",
		"# [0.14.0](http://x/compare/v0.13.1...v0.14.0) (2026-07-05)",
		"",
		"### Bug Fixes",
		"",
		"* old thing ([def](http://x))",
	].join("\n");
	const got = latestChangelogSection(fixture);
	assert.equal(got, "### Features\n\n* new thing ([abc](http://x))");
	assert.equal(latestChangelogSection("no headings here"), "");
	console.log("make-latest-json: selftest OK");
	process.exit(0);
}

const { version } = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));

const bundleDir = join(
	root,
	"src-tauri/target/universal-apple-darwin/release/bundle/macos",
);

let entries;
try {
	entries = readdirSync(bundleDir);
} catch {
	console.error(`make-latest-json: bundle dir not found: ${bundleDir}`);
	process.exit(1);
}

const tarball = entries.find((f) => f.endsWith(".app.tar.gz"));
const sigFile = entries.find((f) => f.endsWith(".app.tar.gz.sig"));

if (!tarball) {
	console.error("make-latest-json: no .app.tar.gz found — is bundle.createUpdaterArtifacts enabled?");
	process.exit(1);
}
if (!sigFile) {
	console.error("make-latest-json: no .app.tar.gz.sig found — is TAURI_SIGNING_PRIVATE_KEY set in CI?");
	process.exit(1);
}

const signature = readFileSync(join(bundleDir, sigFile), "utf8").trim();
if (!signature) {
	console.error("make-latest-json: signature file is empty — refusing to publish a broken manifest.");
	process.exit(1);
}

// Universal binary: both macOS architectures download the same artifact and
// verify against the same signature.
const url = `https://github.com/manustays/quay/releases/download/v${version}/${encodeURIComponent(tarball)}`;
const platform = { signature, url };

// Release notes for the in-app "What's new" banner. @semantic-release/changelog
// writes CHANGELOG.md earlier in the prepare lifecycle, so its top section is the
// version we're building. Cosmetic — fall back to "" rather than aborting.
let notes = "";
try {
	notes = latestChangelogSection(readFileSync(join(root, "CHANGELOG.md"), "utf8"));
} catch {
	console.warn("make-latest-json: CHANGELOG.md unreadable — shipping empty notes.");
}

const manifest = {
	version,
	notes,
	pub_date: new Date().toISOString(),
	platforms: {
		"darwin-aarch64": platform,
		"darwin-x86_64": platform,
	},
};

const out = join(root, "latest.json");
writeFileSync(out, JSON.stringify(manifest, null, 2) + "\n");
console.log(`make-latest-json: wrote ${out} (v${version}, asset ${tarball})`);
