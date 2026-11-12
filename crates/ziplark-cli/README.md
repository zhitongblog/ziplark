# ziplark

The [Ziplark](https://github.com/zhitongblog/ziplark) command-line archiver —
list, preview, grep, extract, create, edit, convert and verify archives from a shell or a cron job.

- Reads **ZIP (incl. `.z01` split, `.jar`, `.apk`, `.whl`, `.crx`), RAR (incl. RAR5, encrypted, multi-volume, self-extracting), 7z (`.7z.001`), tar**,
  the compressed-tar variants (`tar.gz/.bz2/.xz/.zst/.lz4`), single-stream
  `gz/bz2/xz/zst/lz4`, **ISO 9660 / Joliet**, **Debian / ar (`.deb`, `.ar`, `.a`)**, **RPM / CPIO (`.rpm`, `.cpio.*`)**, and **Microsoft Cabinet (`.cab`)**.
- Creates, modifies (`add` / `rm`) and converts (`convert`) **ZIP (AES-256), 7z (AES-256) and tar** (and the compressed-tar variants).
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
ziplark cat repo.tar.gz src/main.rs     # stream a single entry to stdout
ziplark grep "TODO" repo.tar.gz -i      # search text inside an archive in memory
ziplark x photos.zip -o ./out --smart   # smart anti-tarbomb extraction
ziplark x half-downloaded.part1.rar --keep-broken   # salvage what is readable
ziplark x big.rar --include docs/notes.txt --exact  # exactly that entry
ziplark c backup.tar.zst ./src ./README.md --level best
ziplark add backup.tar.zst ./CHANGELOG.md
ziplark rm backup.tar.zst CHANGELOG.md --exact
ziplark convert legacy.rar clean.tar.zst
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
