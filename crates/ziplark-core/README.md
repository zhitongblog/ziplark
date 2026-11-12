# ziplark-core

The archive engine behind [Ziplark](https://github.com/zhitongblog/ziplark) — a free,
fast, cross-platform archiver. This crate is the shared core used by the Ziplark desktop
app, the `ziplark` CLI and the `ziplark-mcp` server.

- **Extracts, previews & searches** ZIP (incl. `.z01`/`.zip.001` split, `.jar`, `.apk`, `.whl`, `.crx`, `.epub`, `.docx`), RAR (incl. RAR5, encrypted, multi-volume, self-extracting), 7z (incl. `.7z.001` multi-volume), tar, the compressed-tar variants (`tar.gz/.bz2/.xz/.zst/.lz4`), single-stream `gz/bz2/xz/zst/lz4`, ISO 9660 / Joliet, Debian & Unix `ar` (`.deb`, `.udeb`, `.ipk`, `.ar`, `.a`), RPM & CPIO (`.rpm`, `.cpio.*`), and Microsoft Cabinet (`.cab`).
- **Creates, edits & converts** ZIP (AES-256), 7z (AES-256) and tar (and the compressed-tar variants) — including in-place entry append (`add`), zero-recompression entry deletion (`remove`), in-memory file preview (`read_entry`), in-archive full-text search (`grep`), and one-step format conversion (`convert`).
- Every extraction path is funneled through a single **zip-slip guard**, so no entry can
  escape the destination directory — not from a crafted ZIP, RAR or tar.

RAR gets first-class treatment, because it is the format archives *arrive* in: a
multi-volume set (`x.part01.rar`, or the legacy `x.rar` + `x.r00`) is one
archive whichever part you open, a missing volume is named rather than guessed
at, damaged archives can be salvaged entry by entry, integrity is checked
without writing anything, self-extracting `.exe` payloads open, and permissions,
timestamps, symlinks and comments all survive. libunrar's C API is driven
directly for it (`src/formats/rar/raw.rs`).

RAR and ISO are extract-only: RAR's compression format is proprietary, and ISO is a
disc-image container.

Licensed MIT · https://ziplark.com
