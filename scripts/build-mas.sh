#!/usr/bin/env bash
#
# Build and sign Ziplark for the Mac App Store (.pkg).
#
#   • Builds Universal macOS binary (arm64 + x86_64)
#   • Embeds src-tauri/Ziplark.provisionprofile + PrivacyInfo.xcprivacy
#   • Sets ITSAppUsesNonExemptEncryption=false & LSApplicationCategoryType
#   • Strips quarantine xattrs (prevents Apple validator error 91109)
#   • Signs .app with "Apple Distribution" + entitlements.mas.plist
#     (App Sandbox + WKWebView JIT/network; NO hardened runtime, which MAS rejects)
#   • Builds signed .pkg with "3rd Party Mac Developer Installer"
#
# Usage:
#   MAS_VERSION=0.5.0 MAS_BUILD_NUMBER=1 ./scripts/build-mas.sh
set -euo pipefail
cd "$(dirname "$0")/.."

MAS_SIGNING_IDENTITY="${MAS_SIGNING_IDENTITY:-Apple Distribution: xiangdong li (6NQM3XP5RF)}"
MAS_INSTALLER_IDENTITY="${MAS_INSTALLER_IDENTITY:-3rd Party Mac Developer Installer: xiangdong li (6NQM3XP5RF)}"
MAS_PROVISIONING_PROFILE="${MAS_PROVISIONING_PROFILE:-src-tauri/Ziplark.provisionprofile}"
DEFAULT_VER="$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed -E 's/.*"version": *"([^"]+)".*/\1/')"
MAS_VERSION="${MAS_VERSION:-$DEFAULT_VER}"
MAS_BUILD_NUMBER="${MAS_BUILD_NUMBER:-1}"
ENTITLEMENTS="src-tauri/entitlements.mas.plist"
TAURI="${CARGO_HOME:-$HOME/.cargo}/bin/cargo-tauri"
command -v cargo-tauri >/dev/null 2>&1 && TAURI=cargo-tauri

[[ -f "$MAS_PROVISIONING_PROFILE" ]] || {
  echo "▶ Provisioning profile missing; running node scripts/asc/setup-appstore.mjs…"
  node scripts/asc/setup-appstore.mjs
}
[[ -f "$ENTITLEMENTS" ]] || { echo "✗ Missing $ENTITLEMENTS" >&2; exit 1; }

echo "▶ Ziplark MAS build — v${MAS_VERSION} (build ${MAS_BUILD_NUMBER})"

# Build universal .app without Developer ID hardened-runtime signing so we can
# sign cleanly with the MAS distribution certificate and sandbox entitlements.
unset APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID
"$TAURI" build --target universal-apple-darwin --bundles app

APP="target/universal-apple-darwin/release/bundle/macos/Ziplark.app"
[[ -d "$APP" ]] || { echo "✗ .app not found at $APP" >&2; exit 1; }

PLIST="$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $MAS_VERSION" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $MAS_BUILD_NUMBER" "$PLIST"
/usr/libexec/PlistBuddy -c "Add :ITSAppUsesNonExemptEncryption bool false" "$PLIST" 2>/dev/null ||
  /usr/libexec/PlistBuddy -c "Set :ITSAppUsesNonExemptEncryption false" "$PLIST"
/usr/libexec/PlistBuddy -c "Add :LSApplicationCategoryType string public.app-category.utilities" "$PLIST" 2>/dev/null ||
  /usr/libexec/PlistBuddy -c "Set :LSApplicationCategoryType public.app-category.utilities" "$PLIST"

echo "▶ Embedding provisioning profile + PrivacyInfo.xcprivacy"
cp "$MAS_PROVISIONING_PROFILE" "$APP/Contents/embedded.provisionprofile"
cp "src-tauri/PrivacyInfo.xcprivacy" "$APP/Contents/Resources/PrivacyInfo.xcprivacy"

echo "▶ Stripping xattrs & ad-hoc signatures"
xattr -cr "$APP"
find "$APP" -type f \( -perm -u+x -o -name "*.dylib" -o -name "*.framework" \) -print0 |
  while IFS= read -r -d '' f; do codesign --remove-signature "$f" 2>/dev/null || true; done
codesign --remove-signature "$APP" 2>/dev/null || true

# Ensure only the main GUI executable sits in Contents/MacOS (no bare CLI helpers).
find "$APP/Contents/MacOS" -type f ! -name ziplark-gui -delete

echo "▶ Signing $APP with '$MAS_SIGNING_IDENTITY' (App Sandbox, no hardened runtime)"
codesign --force --sign "$MAS_SIGNING_IDENTITY" \
  --entitlements "$ENTITLEMENTS" \
  --identifier app.ziplark.desktop \
  "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"

echo "▶ Building signed installer package (.pkg)"
mkdir -p dist-mas
PKG="dist-mas/Ziplark_${MAS_VERSION}_${MAS_BUILD_NUMBER}.pkg"
rm -f "$PKG"
productbuild --component "$APP" /Applications --sign "$MAS_INSTALLER_IDENTITY" "$PKG"
pkgutil --check-signature "$PKG"

echo "✓ MAS package ready: $PKG"
