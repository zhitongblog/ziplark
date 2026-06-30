# Changelog

All notable changes to Ziplark are documented here.
This project adheres to [Semantic Versioning](https://semver.org).

## [0.1.0] — 2026-06

First public release.

### Added
- **Archive engine** (Rust):
  - Extract: ZIP, RAR / RAR5 (incl. encrypted), 7z, tar, tar.gz/.bz2/.xz/.zst/.lz4,
    and single-stream gz / bz2 / xz / zst / lz4.
  - Create: ZIP (AES-256), 7z (AES-256), tar and all of the compression variants above.
- **Three interfaces over one engine**: a Tauri 2 desktop app, the `ziplark` CLI
  (with `--json` on every command), and the `ziplark-mcp` MCP server.
- **OS right-click integration** — `ziplark shell-integration install` adds
  "Extract here" / "Compress to ZIP" to Finder, Explorer and KDE/Nautilus.
- **Security**: every extraction path is funnelled through a single zip-slip /
  path-traversal guard.
- **Distribution**: notarized macOS universal `.dmg`, Windows `.msi`/`.exe`,
  Linux `.deb`/`.AppImage`, a Homebrew tap and a Scoop bucket (winget submitted).

[0.1.0]: https://github.com/zhitongblog/ziplark/releases/tag/v0.1.0
