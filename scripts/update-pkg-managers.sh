#!/usr/bin/env bash
# Automatically update zhitongblog/homebrew-tap (Formula + Cask) and
# zhitongblog/scoop-bucket from the published GitHub Release assets for <tag>.
#
# Usage: scripts/update-pkg-managers.sh [tag]
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed -E 's/.*"version": *"([^"]+)".*/\1/')"
TAG="${1:-v$VERSION}"
VER="${TAG#v}"
REPO="${GITHUB_REPOSITORY:-zhitongblog/ziplark}"

echo "▶ Updating Homebrew tap and Scoop bucket for $REPO @ $TAG (version $VER)…"

WORK="$(mktemp -d /tmp/ziplark-pkg-mgr.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

gh release download "$TAG" --repo "$REPO" --dir "$WORK" \
  --pattern "*.sha256" --pattern "Ziplark-${TAG}-macos-universal.dmg" || true

sha_from_file() {
  local f="$1"
  if [[ -f "$f" ]]; then
    awk '{print $1}' "$f" | tr '[:upper:]' '[:lower:]' | head -1
  fi
}

SHA_MAC_ARM="$(sha_from_file "$WORK/ziplark-${TAG}-aarch64-apple-darwin.tar.gz.sha256")"
SHA_MAC_X64="$(sha_from_file "$WORK/ziplark-${TAG}-x86_64-apple-darwin.tar.gz.sha256")"
SHA_LINUX_X64="$(sha_from_file "$WORK/ziplark-${TAG}-x86_64-unknown-linux-gnu.tar.gz.sha256")"
SHA_WIN_X64="$(sha_from_file "$WORK/ziplark-${TAG}-x86_64-pc-windows-msvc.zip.sha256")"

if [[ -f "$WORK/Ziplark-${TAG}-macos-universal.dmg" ]]; then
  SHA_DMG="$(shasum -a 256 "$WORK/Ziplark-${TAG}-macos-universal.dmg" | awk '{print $1}')"
else
  SHA_DMG=""
fi

echo "  aarch64-apple-darwin : ${SHA_MAC_ARM:-missing}"
echo "  x86_64-apple-darwin  : ${SHA_MAC_X64:-missing}"
echo "  x86_64-linux-gnu     : ${SHA_LINUX_X64:-missing}"
echo "  x86_64-windows-msvc  : ${SHA_WIN_X64:-missing}"
echo "  macos-universal.dmg  : ${SHA_DMG:-missing}"

if [[ -n "$SHA_MAC_ARM" && -n "$SHA_MAC_X64" && -n "$SHA_LINUX_X64" ]]; then
  echo "▶ Updating zhitongblog/homebrew-tap…"
  gh repo clone zhitongblog/homebrew-tap "$WORK/homebrew-tap" -- --depth 1
  cat > "$WORK/homebrew-tap/Formula/ziplark.rb" <<EOF
class Ziplark < Formula
  desc "Free, fast, cross-platform archive tool and MCP server (CLI + MCP)"
  homepage "https://ziplark.com"
  version "${VER}"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/zhitongblog/ziplark/releases/download/v#{version}/ziplark-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "${SHA_MAC_ARM}"
    else
      url "https://github.com/zhitongblog/ziplark/releases/download/v#{version}/ziplark-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "${SHA_MAC_X64}"
    end
  end

  on_linux do
    url "https://github.com/zhitongblog/ziplark/releases/download/v#{version}/ziplark-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "${SHA_LINUX_X64}"
  end

  def install
    bin.install "ziplark"
    bin.install "ziplark-mcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/ziplark --version")
  end
end
EOF

  if [[ -n "$SHA_DMG" ]]; then
    cat > "$WORK/homebrew-tap/Casks/ziplark.rb" <<EOF
cask "ziplark" do
  version "${VER}"
  sha256 "${SHA_DMG}"

  url "https://github.com/zhitongblog/ziplark/releases/download/v#{version}/Ziplark-v#{version}-macos-universal.dmg"
  name "Ziplark"
  desc "Free, fast, cross-platform archiver (ZIP, RAR, 7z, tar, zstd, lz4, ISO, PKG)"
  homepage "https://ziplark.com"

  livecheck do
    url :url
    strategy :github_latest
  end

  app "Ziplark.app"

  zap trash: [
    "~/Library/Application Support/app.ziplark.desktop",
    "~/Library/Caches/app.ziplark.desktop",
    "~/Library/Preferences/app.ziplark.desktop.plist",
  ]
end
EOF
  fi

  (
    cd "$WORK/homebrew-tap"
    if [[ -n "$(git status --porcelain)" ]]; then
      git commit -am "ziplark ${VER}"
      git push origin main
      echo "✓ homebrew-tap updated to ${VER}"
    else
      echo "✓ homebrew-tap already at ${VER}"
    fi
  )
fi

if [[ -n "$SHA_WIN_X64" ]]; then
  echo "▶ Updating zhitongblog/scoop-bucket…"
  gh repo clone zhitongblog/scoop-bucket "$WORK/scoop-bucket" -- --depth 1
  cat > "$WORK/scoop-bucket/bucket/ziplark.json" <<EOF
{
  "version": "${VER}",
  "description": "Free, fast, cross-platform archiver — CLI and MCP server (ZIP, RAR, 7z, tar, zstd, lz4, ISO, PKG)",
  "homepage": "https://ziplark.com",
  "license": "MIT",
  "architecture": {
    "64bit": {
      "url": "https://github.com/zhitongblog/ziplark/releases/download/v${VER}/ziplark-v${VER}-x86_64-pc-windows-msvc.zip",
      "hash": "${SHA_WIN_X64}",
      "extract_dir": "ziplark-v${VER}-x86_64-pc-windows-msvc"
    }
  },
  "bin": [
    "ziplark.exe",
    "ziplark-mcp.exe"
  ],
  "checkver": {
    "github": "https://github.com/zhitongblog/ziplark"
  },
  "autoupdate": {
    "architecture": {
      "64bit": {
        "url": "https://github.com/zhitongblog/ziplark/releases/download/v\$version/ziplark-v\$version-x86_64-pc-windows-msvc.zip",
        "extract_dir": "ziplark-v\$version-x86_64-pc-windows-msvc"
      }
    }
  }
}
EOF
  (
    cd "$WORK/scoop-bucket"
    if [[ -n "$(git status --porcelain)" ]]; then
      git commit -am "ziplark ${VER}"
      git push origin main
      echo "✓ scoop-bucket updated to ${VER}"
    else
      echo "✓ scoop-bucket already at ${VER}"
    fi
  )
fi
