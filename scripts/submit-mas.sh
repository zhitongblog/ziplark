#!/usr/bin/env bash
# Validate and upload a Ziplark MAS .pkg to App Store Connect using the ASC API key
# (~/.appstoreconnect/private_keys/AuthKey_H85Q4NJPVD.p8).
#
# Usage: ./scripts/submit-mas.sh [dist-mas/Ziplark_0.5.0_1.pkg]
set -euo pipefail
cd "$(dirname "$0")/.."

ASC_KEY_ID="${ASC_KEY_ID:-H85Q4NJPVD}"
ASC_ISSUER="${ASC_ISSUER:-21dd1b35-fb04-42f1-8ec0-d847838fa7b6}"

PKG="${1:-$(ls -t dist-mas/*.pkg 2>/dev/null | head -1 || true)}"
[[ -n "$PKG" && -f "$PKG" ]] || { echo "✗ No .pkg found; run ./scripts/build-mas.sh first" >&2; exit 1; }

echo "▶ Validating $PKG with App Store Connect…"
xcrun altool --validate-app -f "$PKG" -t osx \
  --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER"

echo "▶ Uploading $PKG to App Store Connect…"
xcrun altool --upload-app -f "$PKG" -t osx \
  --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER"

echo "✓ Uploaded $PKG — check App Store Connect -> TestFlight / macOS Builds."
