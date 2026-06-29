#!/usr/bin/env bash
# Build, sign, NOTARIZE and staple the Packr macOS desktop app, then attach the
# universal .dmg to a GitHub release. Run this on a Mac with the "Developer ID
# Application" certificate in the login keychain.
#
# Required (one of the two credential sets) — generate once at appleid.apple.com
# or App Store Connect; only the maintainer can supply these:
#
#   App-specific password:
#     export APPLE_ID="you@example.com"
#     export APPLE_PASSWORD="abcd-efgh-ijkl-mnop"   # app-specific password
#     export APPLE_TEAM_ID="6NQM3XP5RF"
#
#   …or App Store Connect API key:
#     export APPLE_API_ISSUER="<issuer-uuid>"
#     export APPLE_API_KEY="<key-id>"
#     export APPLE_API_KEY_PATH="/path/to/AuthKey_XXXX.p8"
#
# Usage:  scripts/release-macos.sh [tag]      # tag defaults to v<version from tauri.conf.json>
set -euo pipefail
cd "$(dirname "$0")/.."

SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-Developer ID Application: xiangdong li (6NQM3XP5RF)}"
VERSION="$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed -E 's/.*"version": *"([^"]+)".*/\1/')"
TAG="${1:-v$VERSION}"

echo "▶ Packr macOS release  —  tag=$TAG  version=$VERSION"
echo "▶ Signing identity: $SIGNING_IDENTITY"

# Credential sanity check — notarization needs ONE of the two sets.
if [[ -z "${APPLE_API_KEY:-}" && -z "${APPLE_PASSWORD:-}" ]]; then
  echo "✗ No notarization credentials in env." >&2
  echo "  Set APPLE_ID + APPLE_PASSWORD + APPLE_TEAM_ID (app-specific password)," >&2
  echo "  or APPLE_API_ISSUER + APPLE_API_KEY + APPLE_API_KEY_PATH (API key)." >&2
  echo "  See the header of this script." >&2
  exit 1
fi

command -v cargo-tauri >/dev/null 2>&1 || { echo "✗ tauri-cli missing: cargo install tauri-cli --version ^2 --locked" >&2; exit 1; }

# Tauri signs the .app with APPLE_SIGNING_IDENTITY, then (because the APPLE_*
# notarization vars are present) submits to notarytool and staples the ticket —
# all during `tauri build`.
export APPLE_SIGNING_IDENTITY="$SIGNING_IDENTITY"

echo "▶ Building universal .dmg (this also signs + notarizes + staples)…"
cargo tauri build --target universal-apple-darwin --bundles app,dmg

DMG="$(ls -t src-tauri/target/universal-apple-darwin/release/bundle/dmg/*.dmg | head -1)"
[[ -f "$DMG" ]] || { echo "✗ no .dmg produced" >&2; exit 1; }

OUT="Packr-${TAG}-macos-universal.dmg"
cp "$DMG" "/tmp/$OUT"
echo "▶ Built: /tmp/$OUT"

echo "▶ Verifying notarization staple…"
xcrun stapler validate "/tmp/$OUT"
spctl -a -t open --context context:primary-signature -v "/tmp/$OUT" 2>&1 || true

echo "▶ Uploading to GitHub release $TAG…"
gh release upload "$TAG" "/tmp/$OUT" --clobber

echo "✓ Done — notarized $OUT attached to release $TAG"
