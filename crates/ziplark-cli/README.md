# ziplark

The [Ziplark](https://github.com/zhitongblog/ziplark) command-line archiver —
list, extract, create and verify archives from a shell or a cron job.

- Reads **ZIP, RAR (incl. RAR5, encrypted, multi-volume, self-extracting), 7z, tar**,
  the compressed-tar variants (`tar.gz/.bz2/.xz/.zst/.lz4`), single-stream
  `gz/bz2/xz/zst/lz4`, and **ISO 9660 / Joliet**.
- Creates **ZIP (AES-256), 7z (AES-256) and tar** (and the compressed-tar variants).
- `--json` on every command, so output is scriptable.
- Every extraction path is funneled through a single **zip-slip guard**, so no
  entry can escape the destination directory.

## Install

```bash
cargo install ziplark-cli
```

Or get a prebuilt binary (with the MCP server and the desktop app) from
[ziplark.com](https://ziplark.com) — `brew install zhitongblog/tap/ziplark`.

## Use

```bash
ziplark l movie.part01.rar              # any volume lists the whole set
ziplark x photos.zip -o ./out
ziplark x half-downloaded.part1.rar --keep-broken   # salvage what is readable
ziplark x big.rar --include docs/notes.txt --exact  # exactly that entry
ziplark c backup.tar.zst ./src ./README.md --level best
ziplark c secret.zip ./private --password hunter2
ziplark t archive.7z                    # verify, writing nothing
ziplark info movie.r03 --json
```

`ziplark shell-integration install` adds the right-click menu to your file
manager.

## Links

- Website: https://ziplark.com
- Source & issues: https://github.com/zhitongblog/ziplark

Licensed MIT. RAR extraction uses UnRAR — see `THIRD_PARTY_LICENSES.md`.
