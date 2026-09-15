//! `ziplark-mcp` — a Model Context Protocol server exposing the Ziplark engine to
//! any LLM. Read operations (list / info / test) are always available; the
//! mutating tools (extract / create) require the `--allow-write` flag.
//!
//! Transport: newline-delimited JSON-RPC 2.0 over stdin/stdout (the MCP stdio
//! transport). Logs go to stderr so they never corrupt the protocol stream.

use ziplark_core::{
    create, detect, extract, list, test, CreateOptions, ExtractOptions, Format, Level, ListOptions,
    MatchMode, Selector,
};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;

const PROTOCOL_VERSION: &str = "2024-11-05";

fn main() {
    let allow_write = std::env::args().any(|a| a == "--allow-write");
    eprintln!(
        "ziplark-mcp {} started (write tools: {})",
        env!("CARGO_PKG_VERSION"),
        if allow_write { "enabled" } else { "disabled (read-only)" }
    );

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("ziplark-mcp: bad JSON: {e}");
                continue;
            }
        };
        if let Some(resp) = handle(&req, allow_write) {
            let mut out = stdout.lock();
            let _ = writeln!(out, "{resp}");
            let _ = out.flush();
        }
    }
}

fn handle(req: &Value, allow_write: bool) -> Option<Value> {
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = req.get("id").cloned();

    match method {
        "initialize" => Some(reply(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "ziplark", "version": env!("CARGO_PKG_VERSION") }
            }),
        )),
        // Notifications carry no id and expect no response.
        "notifications/initialized" | "notifications/cancelled" => None,
        "ping" => Some(reply(id, json!({}))),
        "tools/list" => Some(reply(id, json!({ "tools": tool_defs(allow_write) }))),
        "tools/call" => Some(handle_call(id, req, allow_write)),
        other => Some(error(id, -32601, &format!("method not found: {other}"))),
    }
}

fn reply(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Option<Value>, code: i64, msg: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": msg } })
}

/// A tool result: text content, optionally flagged as an error.
fn tool_result(id: Option<Value>, text: String, is_error: bool) -> Value {
    reply(
        id,
        json!({ "content": [ { "type": "text", "text": text } ], "isError": is_error }),
    )
}

fn tool_defs(allow_write: bool) -> Vec<Value> {
    let pw = json!({ "type": "string", "description": "Password for encrypted archives" });
    let mut tools = vec![
        json!({
            "name": "ziplark_info",
            "description": "What kind of archive this is and what shape it is in, without listing \
its entries: format, entry count, sizes, whether it is encrypted, and — for a RAR set — which \
volumes are present, whether one is missing, whether it is solid, and its comment.",
            "inputSchema": { "type": "object", "required": ["path"],
                "properties": { "path": { "type": "string" }, "password": pw.clone() } }
        }),
        json!({
            "name": "ziplark_list",
            "description": "List the entries inside an archive (zip, 7z, rar, tar.*, gz/bz2/xz/zst, iso). \
Paged: returns at most `limit` entries (default 200) starting at `offset`, plus the total count, \
so a huge archive never floods the context. Narrow with `include` before paging when you are \
looking for something specific. A multi-volume RAR set (`x.part01.rar`, or the older `x.rar` + \
`x.r00`) is one archive: pass any one of its files and the whole set is listed, with the volumes \
reported and any missing one named.",
            "inputSchema": { "type": "object", "required": ["path"],
                "properties": {
                    "path": { "type": "string" },
                    "password": pw.clone(),
                    "offset": { "type": "integer", "minimum": 0,
                        "description": "Index of the first entry to return (default 0). Use the \
`next_offset` from the previous response to page forward." },
                    "limit": { "type": "integer", "minimum": 1,
                        "description": "Maximum entries to return (default 200, capped at 2000)." },
                    "include": { "type": "array", "items": { "type": "string" },
                        "description": "Only entries matching one of these path patterns. A pattern \
with `*` or `?` is matched as a glob against the whole path; otherwise it is a substring match. \
Filtering happens before paging." },
                    "dirs": { "type": "boolean",
                        "description": "false = files only, true = directories only. Omit for both." }
                } }
        }),
        json!({
            "name": "ziplark_test",
            "description": "Verify archive integrity by decompressing every entry and checking its \
checksum. Writes nothing. Reports every entry that fails, not just the first, so a damaged archive \
can be triaged — and says so if a volume of a multi-volume set is missing.",
            "inputSchema": { "type": "object", "required": ["path"],
                "properties": { "path": { "type": "string" }, "password": pw.clone() } }
        }),
    ];
    if allow_write {
        tools.push(json!({
            "name": "ziplark_extract",
            "description": "Extract an archive into a destination directory. No entry can be \
written outside `dest`, whatever the archive claims. For a multi-volume RAR set, pass any one of \
its files.",
            "inputSchema": { "type": "object", "required": ["path", "dest"],
                "properties": {
                    "path": { "type": "string" },
                    "dest": { "type": "string", "description": "Destination directory" },
                    "password": pw.clone(),
                    "overwrite": { "type": "boolean" },
                    "include": { "type": "array", "items": { "type": "string" },
                        "description": "Only extract entries matching one of these patterns. A \
pattern with `*` or `?` is matched as a glob against the whole path; otherwise it is a substring \
match — unless `exact` is set." },
                    "exact": { "type": "boolean",
                        "description": "Treat `include` as complete entry paths, exactly as \
ziplark_list reports them, instead of patterns. Use this to pull out specific files you have \
already seen in a listing: `docs/a.txt` then means that entry and nothing else. A pattern naming \
a directory takes everything under it." },
                    "keep_broken": { "type": "boolean",
                        "description": "Carry on past entries that cannot be extracted and list \
them in `failed`, instead of failing the whole operation at the first one. This is how you get the \
readable files out of a damaged archive, or out of a volume set with a volume missing." }
                } }
        }));
        tools.push(json!({
            "name": "ziplark_create",
            "description": "Create an archive from files/directories. Format inferred from the output extension unless 'format' is given.",
            "inputSchema": { "type": "object", "required": ["output", "inputs"],
                "properties": {
                    "output": { "type": "string" },
                    "inputs": { "type": "array", "items": { "type": "string" } },
                    "format": { "type": "string", "description": "zip|7z|tar|tar.gz|tar.bz2|tar.xz|tar.zst|gz|bz2|xz|zst" },
                    "level": { "type": "string", "description": "store|fast|default|best" },
                    "password": pw
                } }
        }));
    }
    tools
}

fn handle_call(id: Option<Value>, req: &Value, allow_write: bool) -> Value {
    let params = req.get("params").cloned().unwrap_or(json!({}));
    let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let result: Result<String, String> = match name {
        "ziplark_info" => call_info(&args),
        "ziplark_list" => call_list(&args),
        "ziplark_test" => call_test(&args),
        "ziplark_extract" if allow_write => call_extract(&args),
        "ziplark_create" if allow_write => call_create(&args),
        "ziplark_extract" | "ziplark_create" => {
            Err("write tools are disabled; start ziplark-mcp with --allow-write".into())
        }
        other => return error(id, -32602, &format!("unknown tool: {other}")),
    };

    match result {
        Ok(text) => tool_result(id, text, false),
        Err(e) => tool_result(id, format!("error: {e}"), true),
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing required string argument '{key}'"))
}

fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(String::from)
}

fn jsonify<T: serde::Serialize>(v: &T) -> Result<String, String> {
    serde_json::to_string_pretty(v).map_err(|e| e.to_string())
}

fn call_info(args: &Value) -> Result<String, String> {
    let path = str_arg(args, "path")?;
    match detect(std::path::Path::new(path)) {
        Some(f) => {
            let mut out = json!({ "path": path, "format": f.extension(),
                "label": f.label(), "can_create": f.can_create() });
            // Reading the headers is what makes this worth calling: volumes,
            // solidity, a comment, whether the set is complete. An archive we
            // cannot open still reports its format, with the reason attached.
            match list(path, &ListOptions { password: opt_str(args, "password") }) {
                Ok(info) => {
                    out["entries"] = json!(info.entries.len());
                    out["total_size"] = json!(info.total_size);
                    out["total_compressed"] = json!(info.total_compressed);
                    out["encrypted"] = json!(info.encrypted);
                    out["attributes"] = serde_json::to_value(info.attributes)
                        .unwrap_or(Value::Null);
                    if !info.volumes.is_empty() {
                        out["volumes"] = json!(info.volumes);
                    }
                    if let Some(missing) = info.missing_volume {
                        out["missing_volume"] = json!(missing);
                        out["hint"] = json!("This multi-volume set is incomplete. Find the \
missing volume, or extract with keep_broken=true to get what the volumes on hand contain.");
                    }
                    if let Some(comment) = info.comment {
                        out["comment"] = json!(comment);
                    }
                }
                Err(e) => out["error"] = json!(e.to_string()),
            }
            jsonify(&out)
        }
        None => Err(format!("unrecognized archive format: {path}")),
    }
}

/// Default page size for `ziplark_list`. An archive can hold hundreds of
/// thousands of entries; returning them all is what turns one tool call into a
/// million tokens, so the caller has to ask for more explicitly.
const LIST_DEFAULT_LIMIT: usize = 200;
const LIST_MAX_LIMIT: usize = 2000;

fn call_list(args: &Value) -> Result<String, String> {
    let path = str_arg(args, "path")?;
    let info = list(path, &ListOptions { password: opt_str(args, "password") })
        .map_err(|e| e.to_string())?;

    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).clamp(1, LIST_MAX_LIMIT))
        .unwrap_or(LIST_DEFAULT_LIMIT);
    let include: Vec<String> = args
        .get("include")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    // The engine's matcher, so `include` here means exactly what it means to
    // ziplark_extract and to the CLI.
    let selector = Selector::new(&include, MatchMode::Auto);
    let dirs_only = args.get("dirs").and_then(|v| v.as_bool());

    let total_entries = info.entries.len();
    let matched: Vec<&ziplark_core::ArchiveEntry> = info
        .entries
        .iter()
        .filter(|e| match dirs_only {
            Some(true) => e.is_dir,
            Some(false) => !e.is_dir,
            None => true,
        })
        .filter(|e| selector.matches(&e.path))
        .collect();

    let matched_entries = matched.len();
    let page: Vec<&ziplark_core::ArchiveEntry> =
        matched.into_iter().skip(offset).take(limit).collect();
    let returned = page.len();
    let next_offset = offset + returned;
    let truncated = next_offset < matched_entries;

    let mut out = json!({
        "path": info.path,
        "format": info.format,
        "encrypted": info.encrypted,
        "total_size": info.total_size,
        "total_compressed": info.total_compressed,
        "total_entries": total_entries,
        "matched_entries": matched_entries,
        "offset": offset,
        "returned": returned,
        "truncated": truncated,
        "next_offset": if truncated { json!(next_offset) } else { Value::Null },
        "entries": page,
        "attributes": serde_json::to_value(info.attributes).unwrap_or(Value::Null),
    });
    // A RAR set is several files; say which, and say so when one is absent
    // rather than letting the listing look complete.
    if !info.volumes.is_empty() {
        out["volumes"] = json!(info.volumes);
    }
    if let Some(missing) = &info.missing_volume {
        out["missing_volume"] = json!(missing);
        out["warning"] = json!(format!(
            "This listing is incomplete: the archive continues into {}, which is not on disk.",
            missing.display()
        ));
    }
    if let Some(comment) = &info.comment {
        out["comment"] = json!(comment);
    }
    if truncated {
        out["hint"] = json!(format!(
            "Showing {returned} of {matched_entries} matching entries. Call ziplark_list again \
with offset={next_offset} for the next page, or pass `include` to narrow the search instead \
of paging through everything."
        ));
        // An arbitrary 200-of-20000 slice tells you nothing about the shape of
        // the archive. A count per top-level directory does, for a few hundred
        // bytes, and is what makes a huge archive explorable instead of just
        // pageable.
        out["top_level"] = json!(top_level_summary(&info.entries));
    }
    jsonify(&out)
}

/// Entry counts grouped by first path segment, so a caller can see the layout of
/// a large archive without paging through it. Capped so the summary itself can
/// never become the thing that floods the context.
const SUMMARY_MAX_GROUPS: usize = 50;

fn group_at_depth(entries: &[ziplark_core::ArchiveEntry], depth: usize) -> (Vec<String>, std::collections::HashMap<String, u64>) {
    let mut order: Vec<String> = Vec::new();
    let mut counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for e in entries {
        let segs: Vec<&str> = e.path.split('/').filter(|s| !s.is_empty()).collect();
        if segs.is_empty() {
            continue;
        }
        let take = depth.min(segs.len());
        let key = segs[..take].join("/");
        let n = counts.entry(key.clone()).or_insert(0);
        if *n == 0 {
            order.push(key);
        }
        *n += 1;
    }
    (order, counts)
}

fn top_level_summary(entries: &[ziplark_core::ArchiveEntry]) -> Vec<Value> {
    // A single top-level directory ("everything is under src/") tells the caller
    // nothing, so descend until the tree actually branches.
    let mut chosen = group_at_depth(entries, 1);
    for depth in 2..=3 {
        if chosen.0.len() > 1 {
            break;
        }
        let deeper = group_at_depth(entries, depth);
        if deeper.0.len() <= chosen.0.len() {
            break; // no more structure to find
        }
        chosen = deeper;
    }
    let (order, counts) = chosen;

    let shown = order.len().min(SUMMARY_MAX_GROUPS);
    let mut out: Vec<Value> = order[..shown]
        .iter()
        .map(|k| json!({ "prefix": k, "entries": counts[k] }))
        .collect();
    if order.len() > shown {
        out.push(json!({ "prefix": "…", "entries": Value::Null,
            "note": format!("{} more groups not shown", order.len() - shown) }));
    }
    out
}

/// A thoroughly corrupt archive can report every entry as bad, which is the
/// same context blowup as an unpaged listing. Report the count in full but only
/// name the first few.
const TEST_MAX_LISTED_BAD: usize = 50;

fn call_test(args: &Value) -> Result<String, String> {
    let path = str_arg(args, "path")?;
    let report = test(path, &ListOptions { password: opt_str(args, "password") }, None)
        .map_err(|e| e.to_string())?;

    let bad_total = report.bad_entries.len();
    let listed: Vec<&String> = report.bad_entries.iter().take(TEST_MAX_LISTED_BAD).collect();
    let mut out = json!({
        "ok": report.ok,
        "entries_tested": report.entries_tested,
        "bad_entry_count": bad_total,
        "bad_entries": listed,
    });
    if bad_total > listed.len() {
        out["bad_entries_truncated"] = json!(true);
        out["hint"] = json!(format!(
            "{bad_total} entries failed; naming the first {}. The archive is badly damaged.",
            listed.len()
        ));
    }
    jsonify(&out)
}

fn call_extract(args: &Value) -> Result<String, String> {
    let path = str_arg(args, "path")?;
    let dest = str_arg(args, "dest")?;
    let exact = args.get("exact").and_then(|v| v.as_bool()).unwrap_or(false);
    let opts = ExtractOptions {
        password: opt_str(args, "password"),
        dest: PathBuf::from(dest),
        overwrite: args.get("overwrite").and_then(|v| v.as_bool()).unwrap_or(false),
        include: args
            .get("include")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        match_mode: if exact { MatchMode::Exact } else { MatchMode::Auto },
        keep_broken: args.get("keep_broken").and_then(|v| v.as_bool()).unwrap_or(false),
    };
    let report = extract(path, &opts, None).map_err(|e| e.to_string())?;

    let mut out = serde_json::to_value(&report).map_err(|e| e.to_string())?;
    // A partial success has to read as one: the counts on their own look like a
    // clean extraction.
    if !report.failed.is_empty() || !report.partial.is_empty() {
        let listed = report.failed.len().min(TEST_MAX_LISTED_BAD);
        out["failed"] = json!(report.failed[..listed]);
        out["failed_count"] = json!(report.failed.len());
        let mut hint = format!("{} files were written.", report.files_written);
        if !report.failed.is_empty() {
            hint.push_str(&format!(
                " {} thing(s) could not be read — see `failed`.",
                report.failed.len()
            ));
        }
        if !report.partial.is_empty() {
            hint.push_str(&format!(
                " {} file(s) are on disk but incomplete, holding only what the archive actually contained — see `partial`.",
                report.partial.len()
            ));
        }
        out["hint"] = json!(hint);
    }
    jsonify(&out)
}

fn call_create(args: &Value) -> Result<String, String> {
    let output = str_arg(args, "output")?;
    let inputs: Vec<PathBuf> = args
        .get("inputs")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(PathBuf::from)).collect())
        .unwrap_or_default();
    if inputs.is_empty() {
        return Err("'inputs' must be a non-empty array of paths".into());
    }
    let fmt = match opt_str(args, "format") {
        Some(s) => parse_format(&s)?,
        None => detect(std::path::Path::new(output))
            .or_else(|| format_from_name(output))
            .ok_or_else(|| format!("cannot infer format from '{output}'; pass 'format'"))?,
    };
    let opts = CreateOptions {
        format: fmt,
        level: parse_level(opt_str(args, "level").as_deref()),
        password: opt_str(args, "password"),
    };
    let report = create(output, &inputs, &opts, None).map_err(|e| e.to_string())?;
    jsonify(&report)
}

fn parse_level(s: Option<&str>) -> Level {
    match s {
        Some("store") => Level::Store,
        Some("fast") => Level::Fast,
        Some("best") | Some("max") => Level::Best,
        _ => Level::Default,
    }
}

fn parse_format(s: &str) -> Result<Format, String> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "zip" => Format::Zip,
        "7z" | "sevenz" => Format::SevenZ,
        "tar" => Format::Tar,
        "tar.gz" | "tgz" => Format::TarGz,
        "tar.bz2" | "tbz2" => Format::TarBz2,
        "tar.xz" | "txz" => Format::TarXz,
        "tar.zst" | "tzst" => Format::TarZst,
        "gz" => Format::Gz,
        "bz2" => Format::Bz2,
        "xz" => Format::Xz,
        "zst" => Format::Zst,
        other => return Err(format!("unknown format '{other}'")),
    })
}

fn format_from_name(name: &str) -> Option<Format> {
    let l = name.to_ascii_lowercase();
    for (ext, f) in [
        (".tar.gz", Format::TarGz),
        (".tgz", Format::TarGz),
        (".tar.bz2", Format::TarBz2),
        (".tar.xz", Format::TarXz),
        (".tar.zst", Format::TarZst),
        (".tar", Format::Tar),
        (".zip", Format::Zip),
        (".7z", Format::SevenZ),
        (".gz", Format::Gz),
        (".bz2", Format::Bz2),
        (".xz", Format::Xz),
        (".zst", Format::Zst),
    ] {
        if l.ends_with(ext) {
            return Some(f);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use ziplark_core::{create, CreateOptions, Format, Level};

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ziplark-mcp-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// An archive with `n` files spread over `n/10` directories.
    fn archive_with(n: usize, name: &str) -> (PathBuf, PathBuf) {
        let root = tmpdir(name);
        let src = root.join("src");
        for i in 0..n {
            let d = src.join(format!("d{:03}", i / 10));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(format!("f{i:05}.txt")), b"x").unwrap();
        }
        let out = root.join("a.zip");
        create(
            &out,
            &[src.clone()],
            &CreateOptions { format: Format::Zip, level: Level::Fast, password: None },
            None,
        )
        .unwrap();
        (root, out)
    }

    fn list_call(path: &PathBuf, extra: Value) -> Value {
        let mut args = json!({ "path": path.to_str().unwrap() });
        if let Some(map) = extra.as_object() {
            for (k, v) in map {
                args[k] = v.clone();
            }
        }
        serde_json::from_str(&call_list(&args).unwrap()).unwrap()
    }

    #[test]
    fn list_is_paged_by_default_and_reports_the_true_total() {
        let (_root, zip) = archive_with(500, "paged");
        let v = list_call(&zip, json!({}));
        assert_eq!(v["returned"], json!(LIST_DEFAULT_LIMIT));
        assert!(v["total_entries"].as_u64().unwrap() > LIST_DEFAULT_LIMIT as u64);
        assert_eq!(v["truncated"], json!(true));
        assert_eq!(v["next_offset"], json!(LIST_DEFAULT_LIMIT));
        assert!(v["hint"].is_string());
        // The shape of the archive is summarised even though it is truncated.
        assert!(!v["top_level"].as_array().unwrap().is_empty());
    }

    #[test]
    fn paging_walks_every_entry_exactly_once() {
        let (_root, zip) = archive_with(120, "walk");
        let total = list_call(&zip, json!({}))["matched_entries"].as_u64().unwrap() as usize;

        let mut seen: Vec<String> = Vec::new();
        let mut offset = 0u64;
        loop {
            let v = list_call(&zip, json!({ "offset": offset, "limit": 25 }));
            for e in v["entries"].as_array().unwrap() {
                seen.push(e["path"].as_str().unwrap().to_string());
            }
            match v["next_offset"].as_u64() {
                Some(n) => offset = n,
                None => break,
            }
        }
        assert_eq!(seen.len(), total);
        let mut uniq = seen.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), total, "paging returned duplicates");
    }

    #[test]
    fn limit_is_clamped_so_a_caller_cannot_flood_itself() {
        let (_root, zip) = archive_with(30, "clamp");
        let v = list_call(&zip, json!({ "limit": 1_000_000 }));
        assert!(v["returned"].as_u64().unwrap() <= LIST_MAX_LIMIT as u64);
        // Below the cap the caller still gets exactly what it asked for.
        let v = list_call(&zip, json!({ "limit": 5 }));
        assert_eq!(v["returned"], json!(5));
    }

    #[test]
    fn include_filters_before_paging() {
        let (_root, zip) = archive_with(200, "filter");
        let v = list_call(&zip, json!({ "include": ["*/d001/*"] }));
        let n = v["matched_entries"].as_u64().unwrap();
        assert!(n > 0 && n < v["total_entries"].as_u64().unwrap());
        for e in v["entries"].as_array().unwrap() {
            assert!(e["path"].as_str().unwrap().contains("/d001/"));
        }
        assert_eq!(v["returned"].as_u64().unwrap(), n.min(LIST_DEFAULT_LIMIT as u64));
    }

    #[test]
    fn dirs_flag_selects_files_or_directories() {
        let (_root, zip) = archive_with(30, "dirs");
        let files = list_call(&zip, json!({ "dirs": false, "limit": LIST_MAX_LIMIT }));
        let dirs = list_call(&zip, json!({ "dirs": true, "limit": LIST_MAX_LIMIT }));
        assert!(files["entries"].as_array().unwrap().iter().all(|e| e["is_dir"] == json!(false)));
        assert!(dirs["entries"].as_array().unwrap().iter().all(|e| e["is_dir"] == json!(true)));
        assert!(files["matched_entries"].as_u64().unwrap() >= 30);
    }

    #[test]
    fn offset_past_the_end_is_empty_not_an_error() {
        let (_root, zip) = archive_with(10, "past-end");
        let v = list_call(&zip, json!({ "offset": 10_000 }));
        assert_eq!(v["returned"], json!(0));
        assert_eq!(v["truncated"], json!(false));
        assert_eq!(v["next_offset"], Value::Null);
    }

    #[test]
    fn write_tools_are_hidden_without_allow_write() {
        let names: Vec<String> = tool_defs(false)
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"ziplark_list".to_string()));
        assert!(!names.contains(&"ziplark_extract".to_string()));
        assert!(!names.contains(&"ziplark_create".to_string()));
        assert!(tool_defs(true).len() > names.len());
    }

    #[test]
    fn calling_a_write_tool_without_allow_write_is_a_tool_error() {
        let req = json!({ "id": 1, "method": "tools/call",
            "params": { "name": "ziplark_extract", "arguments": { "path": "x", "dest": "y" } } });
        let resp = handle(&req, false).unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("--allow-write"), "unhelpful message: {text}");
    }

    #[test]
    fn summary_descends_past_a_single_root_directory() {
        let (_root, zip) = archive_with(300, "summary");
        let v = list_call(&zip, json!({ "limit": 1 }));
        let groups = v["top_level"].as_array().unwrap();
        // Everything lives under src/, so a depth-1 summary would be one useless
        // group; the summary must go deeper than that.
        assert!(groups.len() > 1, "summary did not descend: {groups:?}");
        assert!(groups[0]["prefix"].as_str().unwrap().contains('/'));
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ziplark-core/tests/fixtures")
            .join(name)
    }

    fn call(name: &str, args: Value) -> Value {
        let req = json!({ "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": args } });
        let resp = handle(&req, true).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap().to_string();
        assert_eq!(resp["result"]["isError"], json!(false), "{text}");
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn listing_a_volume_set_reports_its_volumes() {
        // Any volume opens the set, so an agent handed `part3` sees the whole
        // archive and which files make it up.
        let v = call(
            "ziplark_list",
            json!({ "path": fixture("multi.part3.rar").to_str().unwrap() }),
        );
        assert_eq!(v["total_entries"], json!(5));
        assert_eq!(v["volumes"].as_array().unwrap().len(), 3);
        assert!(v["missing_volume"].is_null());
        assert!(v["path"].as_str().unwrap().ends_with("multi.part1.rar"));
    }

    #[test]
    fn a_solid_archive_with_a_comment_says_so() {
        let v = call(
            "ziplark_info",
            json!({ "path": fixture("solid.rar").to_str().unwrap() }),
        );
        assert_eq!(v["attributes"]["solid"], json!(true));
        assert_eq!(v["attributes"]["recovery_record"], json!(true));
        assert!(v["comment"].as_str().unwrap().contains("Ziplark test fixture"));
    }

    #[test]
    fn exact_selection_extracts_only_what_was_named() {
        let dest = tmpdir("mcp-exact");
        let v = call(
            "ziplark_extract",
            json!({
                "path": fixture("multi.part1.rar").to_str().unwrap(),
                "dest": dest.to_str().unwrap(),
                "include": ["tree/docs/readme.txt"],
                "exact": true
            }),
        );
        assert_eq!(v["files_written"], json!(1));
        assert!(dest.join("tree/docs/readme.txt").exists());
        assert!(!dest.join("tree/big.bin").exists());
    }

    #[test]
    fn a_damaged_archive_reports_what_it_could_not_extract() {
        let dest = tmpdir("mcp-damaged");
        // Without keep_broken the tool errors rather than half-succeeding.
        let req = json!({ "id": 1, "method": "tools/call", "params": {
            "name": "ziplark_extract",
            "arguments": { "path": fixture("damaged.rar").to_str().unwrap(),
                           "dest": dest.to_str().unwrap() } } });
        let resp = handle(&req, true).unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));

        let v = call(
            "ziplark_extract",
            json!({
                "path": fixture("damaged.rar").to_str().unwrap(),
                "dest": tmpdir("mcp-salvage").to_str().unwrap(),
                "keep_broken": true
            }),
        );
        assert_eq!(v["files_written"], json!(2));
        assert_eq!(v["failed_count"], json!(1));
        assert!(v["hint"].as_str().unwrap().contains("could not be read"));
    }

    #[test]
    fn notifications_get_no_response() {
        assert!(handle(&json!({ "method": "notifications/initialized" }), false).is_none());
    }
}
