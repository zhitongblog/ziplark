# Contributing to Ziplark

Thanks for your interest! Ziplark is a small, focused archiver and contributions
are welcome.

## Architecture

One engine, three thin shells:

- `crates/ziplark-core` — all archive logic + the security guard. **Format work
  goes here.**
- `crates/ziplark-cli` — the `ziplark` command-line tool.
- `crates/ziplark-mcp` — the MCP server.
- `src-tauri` + `ui` — the desktop app (Tauri 2; the UI is plain HTML/CSS/JS).

Whatever the app does, the CLI does identically — both call into `ziplark-core`.

## Build & test

```bash
cargo test                 # engine round-trip + security tests
cargo build --release      # all crates, size-optimized
cargo run -p ziplark-gui   # run the desktop app
```

## Adding a format

1. Add a `Format` variant + `label`/`extension`/`can_create` in `model.rs`.
2. Add magic-byte detection in `detect.rs`.
3. Implement it under `crates/ziplark-core/src/formats/` and wire the dispatch
   in `lib.rs`.
4. Add a round-trip test in `crates/ziplark-core/tests/roundtrip.rs`.
5. Reflect it in the CLI (`format_from_name`), the GUI (`parse_format` + the
   Create dropdown), and the README/site tables.

Please keep dependencies lean and avoid anything that requires a heavy system
library (that's why ISO support was deferred — the only capable crate needed FUSE).

## License

By contributing you agree your work is licensed under GPL-3.0.
