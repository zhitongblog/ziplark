#!/usr/bin/env bash
# Build, Developer-ID-sign, NOTARIZE and staple the Ziplark macOS desktop app,
# then attach the universal .dmg to a GitHub release. Run on a Mac that has the
# "Developer ID Application" cert in its login keychain.
#
# Notarization uses a stored notarytool keychain profile (no password in env).
# Create one once (interactive — app-specific password from
# https://account.apple.com/account/manage → Sign-In and Security):
#
#   xcrun notarytool store-credentials ZiplarkNotary \
#     --apple-id lixd220@gmail.com --team-id 6NQM3XP5RF
#
# then run with NOTARY_PROFILE=ZiplarkNotary. Defaults to the shared "UntermNotary"
# profile already present on the maintainer's machine (same Apple ID + team).
#
# Usage:  scripts/release-macos.sh [tag]      # tag defaults to v<tauri.conf version>
set -euo pipefail
cd "$(dirname "$0")/.."

SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-Developer ID Application: xiangdong li (6NQM3XP5RF)}"
NOTARY_PROFILE="${NOTARY_PROFILE:-UntermNotary}"
VERSION="$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed -E 's/.*"version": *"([^"]+)".*/\1/')"
TAG="${1:-v$VERSION}"
TAURI="${CARGO_HOME:-$HOME/.cargo}/bin/cargo-tauri"
command -v cargo-tauri >/dev/null 2>&1 && TAURI=cargo-tauri

echo "▶ Ziplark macOS release — tag=$TAG  identity=$SIGNING_IDENTITY  profile=$NOTARY_PROFILE"

[[ -x "$TAURI" || "$TAURI" = cargo-tauri ]] || { echo "✗ tauri-cli missing: cargo install tauri-cli --version ^2 --locked" >&2; exit 1; }
if ! xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" >/dev/null 2>&1; then
  echo "✗ Notary profile '$NOTARY_PROFILE' not in keychain. Create it (see header)." >&2
  exit 1
fi

# Tauri signs the .app + .dmg with the Developer ID cert (hardened runtime).
export APPLE_SIGNING_IDENTITY="$SIGNING_IDENTITY"
echo "▶ Building + signing universal .dmg…"
"$TAURI" build --target universal-apple-darwin --bundles app,dmg

DMG="$(ls -t target/universal-apple-darwin/release/bundle/dmg/*.dmg 2>/dev/null | head -1 || true)"
[[ -f "$DMG" ]] || { echo "✗ no .dmg produced" >&2; exit 1; }
echo "▶ Built: $DMG"

echo "▶ Submitting to Apple notary service (profile $NOTARY_PROFILE)…"
xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
echo "▶ Stapling ticket…"
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"
spctl --assess --type install --verbose "$DMG" || true

OUT="/tmp/Ziplark-${TAG}-macos-universal.dmg"
cp "$DMG" "$OUT"
echo "▶ Uploading $OUT to release ${TAG} ..."
gh release upload "$TAG" "$OUT" --clobber

echo "✓ Done — notarized $(basename "$OUT") attached to release $TAG"
