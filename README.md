# Ziplark

**Free, fast, cross-platform archiver.** Extracts ZIP, RAR (incl. RAR5), 7z,
tar and the common compressed-tar variants; creates ZIP (with AES-256), 7z and
tar archives. One small Rust engine, three ways to drive it: a desktop app, a
CLI, and an MCP server.

| | Read / Extract | Create | Encryption |
|---|:---:|:---:|---|
| ZIP | ✅ | ✅ | AES-256 (read ZipCrypto) |
| 7z | ✅ | ✅ | AES-256 |
| RAR / RAR5 | ✅ | — | reads encrypted |
| tar | ✅ | ✅ | — |
| tar.gz / .bz2 / .xz / .zst | ✅ | ✅ | — |
| gz / bz2 / xz / zst (single stream) | ✅ | ✅ | — |

> RAR creation is intentionally unsupported — the RAR compression format is
> proprietary. Extraction (including RAR5 and encrypted archives) is supported.

## Why Ziplark
- **Small.** Size-optimized release profile (`opt-level=z`, LTO, stripped,
  `panic=abort`). The desktop app uses the OS webview (no bundled Chromium).
- **Safe.** Every extraction path is funneled through a single zip-slip guard —
  no entry can ever escape the destination directory.
- **One engine.** The GUI, CLI and MCP server are thin shells over
  [`ziplark-core`](crates/ziplark-core); whatever the CLI does, the app does
  identically.

## Repository layout
```
crates/ziplark-core   the archive engine (all formats, the security guard)
crates/ziplark-cli    the `ziplark` command-line tool
crates/ziplark-mcp    the MCP server (drive Ziplark from any LLM)
src-tauri           the Tauri 2 desktop app (Rust commands)
ui                  the desktop frontend (vanilla HTML/CSS/JS)
```

## 1. Desktop app

```bash
# dev run (opens the window)
cargo tauri dev            # or: cargo run -p ziplark-gui

# build a release .app + .dmg (macOS), .exe/.msi (Windows), AppImage/deb (Linux)
cargo tauri build
```
Drag an archive onto the window to inspect & extract it, or switch to **Create**
to drag in files/folders, pick a format + compression level (and optional
password), and save.

## 2. CLI — `ziplark`

```bash
cargo build --release -p ziplark-cli      # binary at target/release/ziplark

ziplark list  movie.rar
ziplark extract photos.zip -o ./out
ziplark create backup.tar.zst ./src ./README.md --level best
ziplark create secret.zip ./private --password hunter2
ziplark test  download.7z
ziplark info  mystery.bin
```
Every command takes `--json` for scripting. `--include <PAT>` filters entries on
extract; `--level store|fast|default|best` and `--password` apply to create.

## 3. MCP server — `ziplark-mcp`

A Model Context Protocol server (JSON-RPC over stdio). Read tools
(`ziplark_info`, `ziplark_list`, `ziplark_test`) are always available; the write tools
(`ziplark_extract`, `ziplark_create`) require `--allow-write`.

```bash
cargo build --release -p ziplark-mcp
```
Register it with an MCP client:
```json
{
  "mcpServers": {
    "ziplark": {
      "command": "/path/to/target/release/ziplark-mcp",
      "args": ["--allow-write"]
    }
  }
}
```

## Building & testing
```bash
cargo test                 # engine round-trip + security tests
cargo build --release      # all crates, size-optimized
```

## License
GPL-3.0. Free as in freedom.
