# Changelog

All notable changes to Ziplark are documented here.
This project adheres to [Semantic Versioning](https://semver.org).

## [0.2.1] — 2026-07

### Changed
- **Relicensed to MIT** (© 2026 doaipm). No dependency ever required copyleft;
  GPL-3.0 was only a scaffolding default. MIT also avoids the GPL-vs-UnRAR
  "no additional restrictions" conflict.
- Added copyright / doaipm attribution across the app, CLI (`--version`), site
  and packaging, and a `THIRD_PARTY_LICENSES.md` inventory with a prominent
  **UnRAR** acknowledgement. CLI archives now ship the third-party notices.

[0.2.1]: https://github.com/zhitongblog/ziplark/releases/tag/v0.2.1

## [0.2.0] — 2026-06

### Added
- **LZ4** — extract and create `.lz4` (single stream) and `.tar.lz4` / `.tlz4`,
  via the pure-Rust `lz4_flex` frame codec.
- **ISO 9660 / Joliet** — extract disc images (`.iso`), including Unicode/long
  (Joliet) names and nested directories. Read with our own dependency-free
  parser — no FUSE, no bundled C, no third-party license.
- **Install via package managers** — Homebrew tap (`brew install --cask
  zhitongblog/tap/ziplark`) and a Scoop bucket; winget submission pending.

[0.2.0]: https://github.com/zhitongblog/ziplark/releases/tag/v0.2.0

## [0.1.0] — 2026-06

First public release.

### Added
- **Archive engine** (Rust):
  - Extract: ZIP, RAR / RAR5 (incl. encrypted), 7z, tar, tar.gz/.bz2/.xz/.zst,
    and single-stream gz / bz2 / xz / zst.
  - Create: ZIP (AES-256), 7z (AES-256), tar and all of the compression variants above.
- **Three interfaces over one engine**: a Tauri 2 desktop app, the `ziplark` CLI
  (with `--json` on every command), and the `ziplark-mcp` MCP server.
- **OS right-click integration** — `ziplark shell-integration install` adds
  "Extract here" / "Compress to ZIP" to Finder, Explorer and KDE/Nautilus.
- **Security**: every extraction path is funnelled through a single zip-slip /
  path-traversal guard.
- **Distribution**: notarized macOS universal `.dmg`, Windows `.msi`/`.exe`,
  Linux `.deb`/`.AppImage`.

[0.1.0]: https://github.com/zhitongblog/ziplark/releases/tag/v0.1.0
