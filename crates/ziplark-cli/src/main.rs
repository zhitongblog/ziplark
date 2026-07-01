//! `ziplark` — the Ziplark command-line archiver.
//!
//! A thin shell over `ziplark-core`: list / extract / create / test / info.
//! Designed to be the self-test driver and a scriptable tool (`--json`).

use ziplark_core::{
    create, detect, extract, list, test, CreateOptions, ExtractOptions, Format, Level, ListOptions,
    Progress,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod shell_integration;

const HELP: &str = "\
ziplark — free, fast, cross-platform archiver

USAGE:
    ziplark <COMMAND> [ARGS]

COMMANDS:
    list|l <archive>                 List archive contents
    extract|x <archive> [-o DIR]     Extract an archive (default: current dir)
    create|c <output> <inputs...>    Create an archive (format from extension)
    test|t <archive>                 Verify archive integrity
    info <archive>                   Detect format
    extract-here <archive...>        Extract each into a sibling folder (for menus)
    compress-zip <item...>           Zip the items into a sibling .zip (for menus)
    shell-integration <install|uninstall|status>
                                     Manage the OS right-click (file-manager) menu

COMMON OPTIONS:
    -p, --password <PW>   Password for encrypted archives
    -o, --output <DIR>    Destination directory (extract)
        --overwrite       Overwrite existing files when extracting
        --include <PAT>   Only entries whose path contains PAT (repeatable)
        --level <L>       store | fast | default | best (create)
        --json            Machine-readable JSON output
    -h, --help            Show this help
    -V, --version         Show version

EXAMPLES:
    ziplark x photos.zip -o ./out
    ziplark c backup.tar.zst ./src ./README.md --level best
    ziplark c secret.zip ./private --password hunter2
    ziplark l movie.rar --json
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("ziplark: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> anyhow::Result<ExitCode> {
    let Some(cmd) = args.first() else {
        print!("{HELP}");
        return Ok(ExitCode::SUCCESS);
    };

    match cmd.as_str() {
        "-h" | "--help" | "help" => {
            print!("{HELP}");
            Ok(ExitCode::SUCCESS)
        }
        "-V" | "--version" => {
            println!("ziplark {}", env!("CARGO_PKG_VERSION"));
            println!("Copyright (c) 2026 doaipm — MIT licensed. https://ziplark.com");
            println!("RAR extraction uses UnRAR (see THIRD_PARTY_LICENSES).");
            Ok(ExitCode::SUCCESS)
        }
        "list" | "l" | "ls" => cmd_list(&args[1..]),
        "extract" | "x" | "e" => cmd_extract(&args[1..]),
        "create" | "c" | "a" | "add" => cmd_create(&args[1..]),
        "test" | "t" => cmd_test(&args[1..]),
        "info" | "i" => cmd_info(&args[1..]),
        "extract-here" => cmd_extract_here(&args[1..]),
        "compress-zip" => cmd_compress_zip(&args[1..]),
        "shell-integration" | "shell" => shell_integration::run(&args[1..]),
        other => {
            eprintln!("ziplark: unknown command '{other}'\n");
            print!("{HELP}");
            Ok(ExitCode::FAILURE)
        }
    }
}

/// Minimal flag parser. Collects positionals and recognized options.
#[derive(Default)]
struct Parsed {
    positionals: Vec<String>,
    password: Option<String>,
    output: Option<String>,
    overwrite: bool,
    include: Vec<String>,
    level: Option<String>,
    json: bool,
}

fn parse(args: &[String]) -> anyhow::Result<Parsed> {
    let mut p = Parsed::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "-p" | "--password" => {
                p.password = Some(next(args, &mut i, "--password")?);
            }
            "-o" | "--output" | "--dest" => {
                p.output = Some(next(args, &mut i, "--output")?);
            }
            "--overwrite" | "-f" => p.overwrite = true,
            "--include" => p.include.push(next(args, &mut i, "--include")?),
            "--level" => p.level = Some(next(args, &mut i, "--level")?),
            "--json" => p.json = true,
            s if s.starts_with('-') && s.len() > 1 => {
                anyhow::bail!("unknown option '{s}'");
            }
            _ => p.positionals.push(a.clone()),
        }
        i += 1;
    }
    Ok(p)
}

fn next(args: &[String], i: &mut usize, flag: &str) -> anyhow::Result<String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))
}

fn parse_level(s: &Option<String>) -> anyhow::Result<Level> {
    Ok(match s.as_deref() {
        None | Some("default") => Level::Default,
        Some("store") | Some("0") => Level::Store,
        Some("fast") => Level::Fast,
        Some("best") | Some("max") => Level::Best,
        Some(other) => anyhow::bail!("invalid --level '{other}' (store|fast|default|best)"),
    })
}

fn progress_printer() -> impl FnMut(Progress) {
    let mut last = String::new();
    move |p: Progress| {
        if p.current_path != last {
            eprintln!("  {}", p.current_path);
            last = p.current_path;
        }
    }
}

fn cmd_list(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    let path = p
        .positionals
        .first()
        .ok_or_else(|| anyhow::anyhow!("list requires an archive path"))?;
    let info = list(
        path,
        &ListOptions {
            password: p.password,
        },
    )?;

    if p.json {
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(ExitCode::SUCCESS);
    }

    println!(
        "{} archive: {}  ({} entries{})",
        info.format.label(),
        info.path.display(),
        info.entries.len(),
        if info.encrypted { ", encrypted" } else { "" }
    );
    println!("{:>12}  {:>5}  {}", "SIZE", "TYPE", "NAME");
    for e in &info.entries {
        println!(
            "{:>12}  {:>5}  {}{}",
            e.size,
            if e.is_dir { "dir" } else { "file" },
            e.path,
            if e.encrypted { "  *" } else { "" }
        );
    }
    println!(
        "total: {} bytes uncompressed, {} bytes on disk",
        info.total_size, info.total_compressed
    );
    Ok(ExitCode::SUCCESS)
}

fn cmd_extract(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    let path = p
        .positionals
        .first()
        .ok_or_else(|| anyhow::anyhow!("extract requires an archive path"))?
        .clone();
    let dest = p.output.clone().unwrap_or_else(|| ".".to_string());
    let json = p.json;
    let opts = ExtractOptions {
        password: p.password,
        dest: PathBuf::from(dest),
        overwrite: p.overwrite,
        include: p.include,
    };
    let mut pr = progress_printer();
    let report = extract(&path, &opts, Some(&mut pr))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "extracted {} files, {} dirs ({} bytes) to {}",
            report.files_written,
            report.dirs_created,
            report.bytes_written,
            report.dest.display()
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_create(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    if p.positionals.len() < 2 {
        anyhow::bail!("create requires <output> and at least one input");
    }
    let output = PathBuf::from(&p.positionals[0]);
    let inputs: Vec<PathBuf> = p.positionals[1..].iter().map(PathBuf::from).collect();
    let fmt = detect(&output)
        .or_else(|| format_from_name(&p.positionals[0]))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cannot infer format from '{}' — use a known extension (.zip, .7z, .tar.gz, ...)",
                output.display()
            )
        })?;
    let json = p.json;
    let opts = CreateOptions {
        format: fmt,
        level: parse_level(&p.level)?,
        password: p.password,
    };
    let mut pr = progress_printer();
    let report = create(&output, &inputs, &opts, Some(&mut pr))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        let ratio = if report.bytes_in > 0 {
            100.0 * report.bytes_out as f64 / report.bytes_in as f64
        } else {
            0.0
        };
        println!(
            "created {} ({}): {} entries, {} -> {} bytes ({:.1}%)",
            report.output.display(),
            report.format.label(),
            report.entries_added,
            report.bytes_in,
            report.bytes_out,
            ratio
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_test(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    let path = p
        .positionals
        .first()
        .ok_or_else(|| anyhow::anyhow!("test requires an archive path"))?;
    let mut pr = progress_printer();
    let report = test(
        path,
        &ListOptions {
            password: p.password,
        },
        Some(&mut pr),
    )?;
    if p.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if report.ok {
        println!("OK — {} entries verified", report.entries_tested);
    } else {
        println!("FAILED — {} bad entries:", report.bad_entries.len());
        for b in &report.bad_entries {
            println!("  {b}");
        }
    }
    Ok(if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn cmd_info(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    let path = p
        .positionals
        .first()
        .ok_or_else(|| anyhow::anyhow!("info requires a path"))?;
    match detect(std::path::Path::new(path)) {
        Some(fmt) => {
            if p.json {
                println!(
                    "{{\"path\":{:?},\"format\":\"{}\",\"can_create\":{}}}",
                    path,
                    fmt.extension(),
                    fmt.can_create()
                );
            } else {
                println!(
                    "{}: {} (create supported: {})",
                    path,
                    fmt.label(),
                    fmt.can_create()
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        None => {
            eprintln!("{path}: unrecognized archive format");
            Ok(ExitCode::FAILURE)
        }
    }
}

/// Strip the archive extension(s) from a file name, handling double extensions
/// like `.tar.gz`. Returns the base name to use for a sibling extract folder.
fn archive_stem(name: &str) -> String {
    let l = name.to_ascii_lowercase();
    for ext in [".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst"] {
        if l.ends_with(ext) {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => name.to_string(),
    }
}

/// Pick a non-existent path by appending " (2)", " (3)", … before the extension
/// if `path` already exists — so menu actions never clobber.
fn unique_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let ext = path.extension().and_then(|s| s.to_str());
    for n in 2..1000 {
        let name = match ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let cand = parent.join(name);
        if !cand.exists() {
            return cand;
        }
    }
    path
}

/// Best-effort macOS desktop notification (no-op elsewhere). Used so the
/// right-click actions give visible feedback when launched from Finder.
fn notify(title: &str, body: &str) {
    if cfg!(target_os = "macos") {
        let script = format!(
            "display notification {:?} with title {:?}",
            body, title
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .status();
    }
}

/// `extract-here`: extract each archive into a sibling folder named after it.
/// This is what the OS right-click "Extract here" menu invokes.
fn cmd_extract_here(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    if p.positionals.is_empty() {
        anyhow::bail!("extract-here requires at least one archive");
    }
    let mut done = 0usize;
    let mut errors = Vec::new();
    for a in &p.positionals {
        let path = Path::new(a);
        let parent = path.parent().unwrap_or(Path::new("."));
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or(a);
        let dest = unique_path(parent.join(archive_stem(name)));
        let opts = ExtractOptions {
            password: p.password.clone(),
            dest: dest.clone(),
            overwrite: true,
            include: Vec::new(),
        };
        match extract(a, &opts, None) {
            Ok(r) => {
                done += 1;
                println!("extracted {} -> {}", a, r.dest.display());
            }
            Err(e) => errors.push(format!("{a}: {e}")),
        }
    }
    if errors.is_empty() {
        notify("Ziplark", &format!("Extracted {done} archive(s)"));
        Ok(ExitCode::SUCCESS)
    } else {
        let msg = errors.join("; ");
        notify("Ziplark — extract failed", &msg);
        anyhow::bail!(msg)
    }
}

/// `compress-zip`: zip the given files/folders into a sibling .zip.
/// This is what the OS right-click "Compress to ZIP" menu invokes.
fn cmd_compress_zip(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    if p.positionals.is_empty() {
        anyhow::bail!("compress-zip requires at least one file or folder");
    }
    let first = Path::new(&p.positionals[0]);
    let parent = first.parent().unwrap_or(Path::new(".")).to_path_buf();
    let out = if p.positionals.len() == 1 {
        let base = first.file_name().and_then(|s| s.to_str()).unwrap_or("Archive");
        parent.join(format!("{base}.zip"))
    } else {
        parent.join("Archive.zip")
    };
    let out = unique_path(out);
    let inputs: Vec<PathBuf> = p.positionals.iter().map(PathBuf::from).collect();
    let opts = CreateOptions {
        format: Format::Zip,
        level: parse_level(&p.level)?,
        password: p.password,
    };
    let report = create(&out, &inputs, &opts, None)?;
    println!(
        "created {} ({} entries)",
        report.output.display(),
        report.entries_added
    );
    notify(
        "Ziplark",
        &format!(
            "Created {}",
            out.file_name().and_then(|s| s.to_str()).unwrap_or("archive")
        ),
    );
    Ok(ExitCode::SUCCESS)
}

/// Fall back to extension-based format guessing for non-existent output files
/// (detect() reads magic bytes, which a to-be-created file doesn't have yet).
fn format_from_name(name: &str) -> Option<Format> {
    let l = name.to_ascii_lowercase();
    let table = [
        (".tar.gz", Format::TarGz),
        (".tgz", Format::TarGz),
        (".tar.bz2", Format::TarBz2),
        (".tbz2", Format::TarBz2),
        (".tar.xz", Format::TarXz),
        (".txz", Format::TarXz),
        (".tar.zst", Format::TarZst),
        (".tzst", Format::TarZst),
        (".tar.lz4", Format::TarLz4),
        (".tlz4", Format::TarLz4),
        (".tar", Format::Tar),
        (".zip", Format::Zip),
        (".7z", Format::SevenZ),
        (".gz", Format::Gz),
        (".bz2", Format::Bz2),
        (".xz", Format::Xz),
        (".zst", Format::Zst),
        (".lz4", Format::Lz4),
        (".iso", Format::Iso),
    ];
    table
        .iter()
        .find(|(ext, _)| l.ends_with(ext))
        .map(|(_, f)| *f)
}
