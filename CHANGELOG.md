# Changelog

All notable changes to Ziplark are documented here.
This project adheres to [Semantic Versioning](https://semver.org).

## [Unreleased]

### Added
- **RAR volume sets are one archive.** `movie.part03.rar` — or the older
  `movie.r01`, whose first volume is `movie.rar` and not `movie.r01` — now opens
  the whole set: any part resolves to the first volume, the parts on disk are
  listed, and a file that spans volumes comes out whole. A set with a hole in it
  names the volume it needs next instead of failing obscurely, and refuses to
  start writing rather than stopping half-way through.
- **Damaged and incomplete archives can be salvaged.** `ziplark x --keep-broken`
  (in the app: "Extract what's readable", offered automatically when an
  extraction hits damage) writes every intact file and reports which entries
  failed and which were left incomplete. Verifying now reports *every* bad
  entry rather than stopping at the first, so you know which file to replace.
- **Self-extracting archives open.** A `.exe` with a RAR payload behind the
  executable stub is detected and read as the archive it is, and `.r00`-style
  volume names are recognised by name as well as by content.
- **Extract exactly the entries you name.** `--exact` treats `--include` as
  complete entry paths instead of substrings, which is what picking three files
  out of a listing needs; the desktop app grew tick boxes per row and an
  "Extract N selected…" button on top of it. `--include` also takes globs now
  (`*/docs/*.txt`), with one matcher shared by every format, the CLI and MCP.
- **RAR 7 archives with very large dictionaries extract.** RAR 7 can pack with
  a dictionary of up to 64 GiB, and libunrar refuses anything over 4 GiB unless
  the caller confirms it — a question nobody was answering, so those entries
  failed with an opaque error. Ziplark now allows up to half of the machine's
  memory (capped at 32 GiB) and, above that, says exactly how much the archive
  wants instead of reporting an error code.
- **Reference and hard-link entries are restored.** RAR5 stores a second copy of
  identical content as a reference to the first (`-oi`) and hard links as links
  (`-oh`); both are now materialised, with their targets checked by the same
  guard every other link goes through.
- **Double-clicking `movie.r01` opens Ziplark.** The legacy volume extensions
  `.r00`–`.r09` are registered by the desktop app and the Windows right-click
  menu, alongside `.rar`.
- **The archive itself is described, not just its entries.** Volumes, a missing
  volume, solid, recovery record, encrypted headers, locked, and the archive
  comment (RAR and ZIP) are reported by `ziplark info`, the MCP tools and the
  app's header.

- **The desktop app shows progress and can be stopped.** Long operations used to
  put up an indeterminate spinner with no way out. There is now a real progress
  bar — a percentage wherever the total is known — the current entry, running
  counts, and a Cancel button. Cancelling is honoured within a fraction of a
  second even in the middle of a multi-gigabyte file, and says plainly that the
  files already written were kept.
- **Archives open with a folder tree instead of a flat list**, and only the rows
  on screen are in the DOM, so a disc image with 200 000 entries opens as fast
  as a small ZIP.
- **"Open with Ziplark" works.** The bundle now declares the archive file types,
  and an archive handed over by the OS — as a command-line argument, or by
  macOS after launch — is opened in the window.
- Extract and Create finish with a **Show** button that reveals the result in
  the file manager.

### Changed
- **The engine can be asked to stop.** The progress callback returns a bool now;
  answering `false` aborts the operation with `Error::Cancelled`. Progress is
  also reported *during* a large entry rather than only between entries, so a
  single big file no longer looks frozen and can be interrupted. ZIP extraction
  reports a real byte total, taken from the central directory.
- **The app asks before replacing files.** It used to pass "overwrite" on every
  extraction, silently clobbering whatever was in the destination.
- **A new archive's password has to be typed twice.** A typo in a password that
  is never shown again makes the archive unopenable.
- The desktop commands run off the main thread. A plain Tauri command runs on
  it, so extracting a large archive froze the entire window — spinner included —
  until it finished.
- **7z archives are now solid, and compression uses every core.** We were
  writing one compression block per file, so LZMA2 restarted — new dictionary,
  new encoder — for every entry, and nothing was ever compressed against
  anything else. On a 235 MB / 15,043-file tree that cost 584 s and produced a
  48.7 MB archive; it is now 104 s and 33.9 MB (system 7-Zip, for reference:
  40.8 s and 32.4 MB). Entries are packed into solid blocks of up to 256 MiB —
  capped so that pulling one file out of a big archive stays bounded — and
  encoded with the multi-threaded LZMA2 encoder. Archives are verified
  interoperable with 7-Zip in both directions, encrypted ones included.
- **`--level` now does something for 7z.** It was accepted and ignored; store /
  fast / default / best map to COPY and LZMA2 presets 1 / 6 / 9.
- 7z gained the codecs needed to *open* archives other tools produce: bzip2,
  deflate, lz4, zstd and PPMd, alongside the LZMA/LZMA2 we already had.
- `ziplark list` on a 7z now reports each entry's real modification time, CRC
  and compressed size, and whether the archive is encrypted is read from the
  archive's own coder chain instead of from whether the caller passed a
  password.

### Fixed
- **RAR metadata was being dropped or read from the wrong place.** Permissions
  (an executable now stays executable), RAR5's 100-nanosecond timestamps,
  symlinks as symlinks rather than copies, and per-entry compressed sizes all
  survive extraction. The library's headers are `#pragma pack(1)`, and the
  bindings we were using declare them unpacked — so every field past
  `file_attr`, timestamps and link targets among them, was read from the wrong
  offset. Ziplark now drives libunrar's C API directly with layouts that match.
- **Verifying a RAR no longer extracts it.** Integrity was checked by unpacking
  the entire archive into a temporary directory and deleting it afterwards —
  40 GB of writes to check a 40 GB archive. It now decompresses and discards.
- **A multi-volume RAR could corrupt memory.** The wrapper crate's
  volume-change callback copies a fixed 2048 wide characters out of a much
  shorter string every time an archive crosses a volume boundary; rustc's
  undefined-behaviour checks abort on it. Gone with the wrapper.
- **A directory entry could be used to write outside the destination.** A
  symlink standing where a directory entry wants to be was followed by
  `create_dir_all`, so `mkdir` landed outside the destination; directory
  entries now go through the same on-disk check as files, in every format.
- **Permissions, timestamps and symlinks survive a round trip.** Creating an
  archive hardcoded mode 0644 and stamped every entry with the current time;
  extraction restored neither. A zipped executable came out unable to run, and
  every extracted file looked brand new to `make`, rsync and backup tools.
  ZIP, 7z and tar now carry the real mode and mtime in both directions. ZIP also
  writes the 0x5455 extended timestamp: the base ZIP header holds an MS-DOS
  time with no time zone, which every UTC-writing tool shifts by the local
  offset, and the extra field pins the exact instant.
- **Symlinks are stored as symlinks.** Archiving followed them and wrote a full
  copy of the target, which inflates a tree of links and breaks macOS `.app`
  bundles, and a link pointing back up its own tree made the directory walk
  recurse forever. Links are now recorded as links by ZIP, 7z (the unix-mode
  attribute p7zip uses) and tar, and restored as links on extraction.
- **ZIP entries of 4 GiB or more can be created.** The writer was left on its
  32-bit default, so a single large file failed with "Large file option has not
  been set". Zip64 is enabled per entry when the file needs it.
- **Security: a crafted tar could write outside the destination directory.**
  `tar`/`tar.gz`/… restore symlinks, and the guard only ever checked entry
  *names*. An archive storing `evil -> /somewhere` followed by an entry named
  `evil/owned.txt` used neither `..` nor an absolute path, so the name check
  passed it and the write followed the link out of the destination — enough to
  drop a file in `~/.zshrc` or `~/Library/LaunchAgents`. Extraction now runs
  through a `DestGuard` that also refuses to descend through a symlink, and
  never writes through one sitting at the target path (`symlink_metadata`, so
  even a dangling link is caught). Hard links pointing outside the destination
  are refused too. Links that stay inside are still restored as links.
  The guard is shared, so ZIP, 7z, RAR and ISO get the same protection against
  symlinks already present in the destination.
- **Non-UTF-8 filenames are no longer mojibake.** A ZIP written by Windows
  Explorer or WinRAR on a Chinese, Japanese or Korean system stores names as raw
  code-page bytes with general-purpose bit 11 clear; reading those as CP437 (the
  `zip` crate's default) turned `中文文件.txt` into `ÖÐÎÄÎÄ¼þ.txt`, in the
  listing and on disk. ZIP and tar names are now decoded per archive: valid
  UTF-8 is taken as-is, anything else is decoded with an encoding detected from
  all of the archive's names at once (chardetng), falling back to the machine's
  own code page when the sample is too short to call.

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
