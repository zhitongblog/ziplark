//! `packr-mcp` — a Model Context Protocol server exposing the Packr engine to
//! any LLM. Read operations (list / info / test) are always available; the
//! mutating tools (extract / create) require the `--allow-write` flag.
//!
//! Transport: newline-delimited JSON-RPC 2.0 over stdin/stdout (the MCP stdio
//! transport). Logs go to stderr so they never corrupt the protocol stream.

use packr_core::{
    create, detect, extract, list, test, CreateOptions, ExtractOptions, Format, Level, ListOptions,
};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;

const PROTOCOL_VERSION: &str = "2024-11-05";

fn main() {
    let allow_write = std::env::args().any(|a| a == "--allow-write");
    eprintln!(
        "packr-mcp {} started (write tools: {})",
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
                eprintln!("packr-mcp: bad JSON: {e}");
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
                "serverInfo": { "name": "packr", "version": env!("CARGO_PKG_VERSION") }
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
            "name": "packr_info",
            "description": "Detect an archive's format from its contents.",
            "inputSchema": { "type": "object", "required": ["path"],
                "properties": { "path": { "type": "string" } } }
        }),
        json!({
            "name": "packr_list",
            "description": "List the entries inside an archive (zip, 7z, rar, tar.*, gz/bz2/xz/zst).",
            "inputSchema": { "type": "object", "required": ["path"],
                "properties": { "path": { "type": "string" }, "password": pw.clone() } }
        }),
        json!({
            "name": "packr_test",
            "description": "Verify archive integrity by decompressing every entry.",
            "inputSchema": { "type": "object", "required": ["path"],
                "properties": { "path": { "type": "string" }, "password": pw.clone() } }
        }),
    ];
    if allow_write {
        tools.push(json!({
            "name": "packr_extract",
            "description": "Extract an archive into a destination directory.",
            "inputSchema": { "type": "object", "required": ["path", "dest"],
                "properties": {
                    "path": { "type": "string" },
                    "dest": { "type": "string", "description": "Destination directory" },
                    "password": pw.clone(),
                    "overwrite": { "type": "boolean" },
                    "include": { "type": "array", "items": { "type": "string" },
                        "description": "Only entries whose path contains one of these substrings" }
                } }
        }));
        tools.push(json!({
            "name": "packr_create",
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
        "packr_info" => call_info(&args),
        "packr_list" => call_list(&args),
        "packr_test" => call_test(&args),
        "packr_extract" if allow_write => call_extract(&args),
        "packr_create" if allow_write => call_create(&args),
        "packr_extract" | "packr_create" => {
            Err("write tools are disabled; start packr-mcp with --allow-write".into())
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
        Some(f) => Ok(json!({ "path": path, "format": f.extension(),
            "label": f.label(), "can_create": f.can_create() })
        .to_string()),
        None => Err(format!("unrecognized archive format: {path}")),
    }
}

fn call_list(args: &Value) -> Result<String, String> {
    let path = str_arg(args, "path")?;
    let info = list(path, &ListOptions { password: opt_str(args, "password") })
        .map_err(|e| e.to_string())?;
    jsonify(&info)
}

fn call_test(args: &Value) -> Result<String, String> {
    let path = str_arg(args, "path")?;
    let report = test(path, &ListOptions { password: opt_str(args, "password") }, None)
        .map_err(|e| e.to_string())?;
    jsonify(&report)
}

fn call_extract(args: &Value) -> Result<String, String> {
    let path = str_arg(args, "path")?;
    let dest = str_arg(args, "dest")?;
    let opts = ExtractOptions {
        password: opt_str(args, "password"),
        dest: PathBuf::from(dest),
        overwrite: args.get("overwrite").and_then(|v| v.as_bool()).unwrap_or(false),
        include: args
            .get("include")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default(),
    };
    let report = extract(path, &opts, None).map_err(|e| e.to_string())?;
    jsonify(&report)
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
