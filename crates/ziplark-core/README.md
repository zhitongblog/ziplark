# ziplark-core

The archive engine behind [Ziplark](https://github.com/zhitongblog/ziplark) — a free,
fast, cross-platform archiver. This crate is the shared core used by the Ziplark desktop
app, the `ziplark` CLI and the `ziplark-mcp` server.

- **Extracts** ZIP, RAR (incl. RAR5, encrypted), 7z, tar, the compressed-tar variants
  (`tar.gz/.bz2/.xz/.zst/.lz4`), single-stream `gz/bz2/xz/zst/lz4`, and ISO 9660 / Joliet.
- **Creates** ZIP (AES-256), 7z (AES-256) and tar (and the compressed-tar variants).
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
