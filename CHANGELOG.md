# Changelog

All notable changes to Ziplark are documented here.
This project adheres to [Semantic Versioning](https://semver.org).

## [0.2.2] — 2026-08

### Fixed
- **Windows: the binaries now run on a clean install.** Every Windows build up to
  and including 0.2.1 imported `VCRUNTIME140.dll` / `VCRUNTIME140_1.dll` (from the
  bzip2, xz2 and zstd C dependencies) and `MSVCP140.dll` (libunrar is C++). None of
  those ship with Windows, so on any machine without the Visual C++ redistributable
  the desktop app, the CLI and the MCP server all died with `STATUS_DLL_NOT_FOUND`
  (0xC0000135) before `main()` ran. The MSVC runtime is now statically linked, so
  the binaries depend only on DLLs that are part of Windows itself.
  This is what failed winget validation (microsoft/winget-pkgs#395332); it also
  affected Scoop installs of the CLI.
- Windows installers embed the WebView2 bootstrapper instead of downloading it
  during setup, so an offline or locked-down machine still gets the runtime.

### Added
- `scripts/windows-smoke.ps1` — a clean-machine smoke test. CI runners have the
  Visual C++ redistributable installed, so *running* a binary there cannot catch
  the bug above; the script instead parses each PE import table (and the delay-load
  table) and fails on any DLL that is not part of a base Windows install. It then
  runs a CLI create/list/test/extract roundtrip, an MCP stdio handshake, and a GUI
  launch. CI runs it on every push and the release workflow runs it before
  uploading any artifact.

[0.2.2]: https://github.com/zhitongblog/ziplark/releases/tag/v0.2.2

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
