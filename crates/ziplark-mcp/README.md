# ziplark-mcp

The [Ziplark](https://github.com/zhitongblog/ziplark) MCP server — drive a fast,
cross-platform archive engine from any LLM over the Model Context Protocol.

It exposes tools to **list, read/preview, grep, extract, test, create, edit and convert** archives:

- Reads **ZIP (incl. `.z01` split, `.jar`, `.apk`, `.whl`, `.crx`), RAR (incl. RAR5, encrypted, multi-volume, self-extracting), 7z (`.7z.001`), tar**,
  the compressed-tar variants (`tar.gz/.bz2/.xz/.zst/.lz4`), single-stream
  `gz/bz2/xz/zst/lz4`, **ISO 9660 / Joliet**, **Debian / ar (`.deb`, `.ar`, `.a`)**, **RPM / CPIO (`.rpm`, `.cpio.*`)**, and **Microsoft Cabinet (`.cab`)**.
- Listings (`ziplark_list`), file reads (`ziplark_read`), and full-text searches (`ziplark_grep`) are **paged** and stream in memory without writing to disk, so a 200 000-entry disc image costs the same context as a small ZIP.
- `keep_broken` salvages what is readable from a damaged or incomplete archive
  and reports what failed; `exact` extracts precisely the entries named in a
  listing; `smart` prevents tarbombs and double-nested folders.
- Creates, modifies (`ziplark_add`, `ziplark_remove`) and converts (`ziplark_convert`) **ZIP (AES-256), 7z (AES-256) and tar** (and the compressed-tar variants).
- Every extraction path is funneled through a single **zip-slip guard**, so no entry
  can escape the destination directory.

Read tools (`ziplark_info`, `ziplark_list`, `ziplark_read`, `ziplark_grep`, `ziplark_test`) are always available; write tools (`ziplark_extract`, `ziplark_create`, `ziplark_add`, `ziplark_remove`, `ziplark_convert`) are gated behind
`--allow-write`.

## Install

```bash
cargo install ziplark-mcp
```

Or get a prebuilt binary (with the `ziplark` CLI and desktop app) from
[ziplark.com](https://ziplark.com) — `brew install zhitongblog/tap/ziplark`.

## Use with an MCP client

```json
{
  "mcpServers": {
    "ziplark": {
      "command": "ziplark-mcp",
      "args": ["--allow-write"]
    }
  }
}
```

## Links

- Homepage: https://ziplark.com
- Repository: https://github.com/zhitongblog/ziplark
- MCP Registry name: `mcp-name: io.github.zhitongblog/ziplark`

Licensed MIT.
