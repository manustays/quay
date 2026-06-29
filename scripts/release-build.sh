#!/usr/bin/env bash
# Build the universal macOS bundle for a release, signing only when a COMPLETE
# set of Apple credentials is present. Called from @semantic-release/exec's
# prepareCmd after scripts/set-version.mjs has written the version.
#
# Why the gate: Tauri's bundler attempts code-signing whenever APPLE_CERTIFICATE
# is set — even to an empty string — and then fails importing an empty cert. We
# therefore unset the whole Apple env unless BOTH the certificate and the signing
# identity are non-empty (a partial set is treated as "unsigned", never half-signed).
set -euo pipefail

if [ -z "${APPLE_CERTIFICATE:-}" ] || [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
	echo "release-build: Apple credentials incomplete — building UNSIGNED."
	unset APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY \
		APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID 2>/dev/null || true
else
	echo "release-build: Apple credentials present — building SIGNED + notarized."
fi

npm run tauri build -- --target universal-apple-darwin
