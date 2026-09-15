# ziplark-mcp

The [Ziplark](https://github.com/zhitongblog/ziplark) MCP server — drive a fast,
cross-platform archive engine from any LLM over the Model Context Protocol.

It exposes tools to **list, extract, test and create** archives:

- Reads **ZIP, RAR (incl. RAR5, encrypted, multi-volume, self-extracting), 7z, tar**,
  the compressed-tar variants (`tar.gz/.bz2/.xz/.zst/.lz4`), single-stream
  `gz/bz2/xz/zst/lz4`, and **ISO 9660 / Joliet**.
- Listings are **paged** (`offset`/`limit`, with per-directory counts when
  truncated), so a 200 000-entry disc image costs the same context as a small ZIP.
- `keep_broken` salvages what is readable from a damaged or incomplete archive
  and reports what failed; `exact` extracts precisely the entries named in a
  listing.
- Creates **ZIP (AES-256), 7z (AES-256) and tar** (and the compressed-tar variants).
- Every extraction path is funneled through a single **zip-slip guard**, so no entry
  can escape the destination directory.

Read tools are always available; write tools (extract/create) are gated behind
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
