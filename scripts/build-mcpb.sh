#!/usr/bin/env bash
#
# Build `ziplark-mcp-<version>.mcpb` — the MCP Bundle, which is how the official
# MCP registry (registry.modelcontextprotocol.io) accepts a server that is a
# compiled binary rather than an npm or PyPI package. Listing there is what
# feeds Glama, PulseMCP and the other directories.
#
# A bundle is a zip of `manifest.json` plus the files it names. This one carries
# all three platforms, because the registry takes a single artifact per server:
#
#   server/ziplark-mcp        macOS universal (arm64 + x86_64)
#   server/ziplark-mcp.exe    Windows x64
#   server/ziplark-mcp-linux  Linux x64
#
# The macOS binary keeps the plain name on purpose: a host that follows the
# spec's "apps will automatically append .exe on Windows" note lands on the
# Windows file even if it ignores `platform_overrides`.
#
# Usage:
#   scripts/build-mcpb.sh <dir-with-release-archives> [output-dir]
#
# where the input directory holds the per-target archives a release produces:
#   ziplark-v<version>-aarch64-apple-darwin.tar.gz
#   ziplark-v<version>-x86_64-apple-darwin.tar.gz
#   ziplark-v<version>-x86_64-pc-windows-msvc.zip
#   ziplark-v<version>-x86_64-unknown-linux-gnu.tar.gz
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
in_dir="${1:?usage: build-mcpb.sh <dir-with-release-archives> [output-dir]}"
out_dir="${2:-$repo/target/mcpb}"
in_dir="$(cd "$in_dir" && pwd)"

version="$(sed -nE 's/^version = "([^"]+)".*/\1/p' "$repo/Cargo.toml" | head -1)"
[[ -n "$version" ]] || { echo "✗ could not read version from Cargo.toml" >&2; exit 1; }
echo "▶ Ziplark MCP bundle — version $version"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
stage="$work/bundle"
mkdir -p "$stage/server"

extract_from_tar() {  # <archive> <member-basename> <destination>
  local archive="$1" member="$2" dest="$3" path
  path="$(tar tzf "$archive" | grep -m1 "/$member\$")" ||
    { echo "✗ $member not found in $(basename "$archive")" >&2; exit 1; }
  tar xzf "$archive" -C "$work" "$path"
  mv "$work/$path" "$dest"
}

# macOS: one universal binary from the two per-arch builds, so a single file
# serves both Apple Silicon and Intel.
extract_from_tar "$in_dir/ziplark-v$version-aarch64-apple-darwin.tar.gz" ziplark-mcp "$work/mcp-arm64"
extract_from_tar "$in_dir/ziplark-v$version-x86_64-apple-darwin.tar.gz" ziplark-mcp "$work/mcp-x64"
lipo -create "$work/mcp-arm64" "$work/mcp-x64" -output "$stage/server/ziplark-mcp"

extract_from_tar "$in_dir/ziplark-v$version-x86_64-unknown-linux-gnu.tar.gz" ziplark-mcp "$stage/server/ziplark-mcp-linux"

unzip -q -j "$in_dir/ziplark-v$version-x86_64-pc-windows-msvc.zip" "*/ziplark-mcp.exe" -d "$stage/server"

chmod +x "$stage/server/ziplark-mcp" "$stage/server/ziplark-mcp-linux"

# The manifest carries a placeholder version in the repo; the release's version
# is what ships.
python3 - "$repo/mcpb/manifest.json" "$stage/manifest.json" "$version" <<'PY'
import json, sys
src, dst, version = sys.argv[1], sys.argv[2], sys.argv[3]
manifest = json.load(open(src))
manifest["version"] = version
json.dump(manifest, open(dst, "w"), indent=2, ensure_ascii=False)
PY

cp "$repo/assets/icon.png" "$stage/icon.png" 2>/dev/null || cp "$repo/web/icon.png" "$stage/icon.png"
cp "$repo/LICENSE" "$repo/THIRD_PARTY_LICENSES.md" "$stage/"

mkdir -p "$out_dir"
out="$out_dir/ziplark-mcp-$version.mcpb"
rm -f "$out"
# `mcpb pack` validates the manifest before zipping, which is the check that
# matters — the schema has no stable public URL to point an editor at.
npx --yes @anthropic-ai/mcpb@latest pack "$stage" "$out" >/dev/null

# The registry entry has to carry the bundle's SHA-256, so publish it beside
# the bundle rather than making everyone download 10 MB to compute it.
shasum -a 256 "$out" | sed "s|$out_dir/||" > "$out.sha256"

echo "▶ Built $out"
cat "$out.sha256"
unzip -l "$out" | sed -n '1,12p'
