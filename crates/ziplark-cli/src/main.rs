//! `ziplark` — the Ziplark command-line archiver.
//!
//! A thin shell over `ziplark-core`: list / extract / create / add / cat / test / info.
//! Designed to be the self-test driver and a scriptable tool (`--json`).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use ziplark_core::{
    add, convert, create, detect, detect_by_extension, extract, grep, list, read_entry, remove,
    shell, test, ArchiveInfo, ConvertOptions, CreateOptions, ExtractOptions, Format, GrepOptions,
    Level, ListOptions, MatchMode, Progress, ReadOptions, RemoveOptions,
    DEFAULT_GREP_MAX_FILE_SIZE, DEFAULT_GREP_MAX_MATCHES, DEFAULT_READ_MAX_BYTES,
};

mod shell_integration;

const HELP: &str = "\
ziplark — free, fast, cross-platform archiver

USAGE:
    ziplark <COMMAND> [ARGS]

COMMANDS:
    list|l <archive>                 List archive contents
    extract|x <archive> [-o DIR]     Extract an archive (default: current dir)
    create|c <output> <inputs...>    Create an archive (format from extension)
    add|a <archive> <inputs...>      Add or update files inside an archive
    rm|remove <archive> <entries...> Remove entries from an archive
    convert <source> <output>        Convert an archive to another format
    cat|read <archive> <entry>       Read a single entry to stdout without extracting
    grep|search <pattern> <archive>  Search text inside archive files without extracting
    test|t <archive>                 Verify archive integrity
    info <archive>                   Detect format and archive attributes
    extract-here <archive...>        Smart-extract each into a sibling folder (for menus)
    compress-zip <item...>           Zip the items into a sibling .zip (for menus)
    shell-integration <install|uninstall|status>
                                     Manage the OS right-click (file-manager) menu

COMMON OPTIONS:
    -p, --password <PW>   Password for encrypted archives
    -o, --output <DIR>    Destination directory (extract)
    -i, --ignore-case     Case-insensitive search (grep)
        --overwrite       Overwrite existing files when extracting
        --smart           Smart anti-tarbomb folder (wrap loose files into <stem>/,
                          avoid double-wrapping a single root folder)
        --include <PAT>   Only entries matching PAT (repeatable). A pattern with
                          * or ? is a glob over the whole path; otherwise it is
                          matched as a substring
        --exact           Treat --include / rm patterns as exact entry paths (a
                          directory takes everything under it)
        --keep-broken     Carry on past entries that fail, and list them. For
                          damaged or incomplete archives — without it, the first
                          bad entry stops the extraction
        --max-matches <N> Maximum matching lines in `grep` (default: 500)
        --max-bytes <N>   Maximum bytes to read in `cat` (default: 4 MiB)
        --offset <N>      Byte offset to start reading from in `cat` (default: 0)
        --level <L>       store | fast | default | best (create / add / convert)
        --json            Machine-readable JSON output
    -h, --help            Show this help
    -V, --version         Show version

EXAMPLES:
    ziplark x photos.zip -o ./out --smart
    ziplark c backup.tar.zst ./src ./README.md --level best
    ziplark a backup.zip ./extra.txt
    ziplark rm backup.zip extra.txt --exact
    ziplark convert legacy.rar clean.tar.zst --level best
    ziplark grep \"TODO\" repo.tar.gz -i --include \"*.rs\"
    ziplark cat release.deb DEBIAN/control
    ziplark l movie.part01.rar --json
    ziplark x movie.7z.002 -o ./out       # any volume opens the whole set
    ziplark x half-downloaded.part1.rar --keep-broken
    ziplark x big.rar --include docs/notes.txt --exact
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
        "create" | "c" => cmd_create(&args[1..]),
        "add" | "a" => cmd_add(&args[1..]),
        "rm" | "remove" | "del" | "delete" => cmd_remove(&args[1..]),
        "convert" | "conv" => cmd_convert(&args[1..]),
        "cat" | "read" | "show" => cmd_cat(&args[1..]),
        "grep" | "search" => cmd_grep(&args[1..]),
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
    smart: bool,
    ignore_case: bool,
    include: Vec<String>,
    exact: bool,
    keep_broken: bool,
    max_matches: Option<usize>,
    max_bytes: Option<u64>,
    offset: Option<u64>,
    level: Option<String>,
    json: bool,
}

impl Parsed {
    fn match_mode(&self) -> MatchMode {
        if self.exact {
            MatchMode::Exact
        } else {
            MatchMode::Auto
        }
    }
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
            "-i" | "--ignore-case" => p.ignore_case = true,
            "--overwrite" | "-f" => p.overwrite = true,
            "--smart" | "-s" => p.smart = true,
            "--include" => p.include.push(next(args, &mut i, "--include")?),
            "--exact" => p.exact = true,
            "--keep-broken" | "--keep" => p.keep_broken = true,
            "--max-matches" => {
                let v = next(args, &mut i, "--max-matches")?;
                p.max_matches = Some(
                    v.parse::<usize>()
                        .map_err(|_| anyhow::anyhow!("invalid --max-matches '{v}'"))?,
                );
            }
            "--max-bytes" => {
                let v = next(args, &mut i, "--max-bytes")?;
                p.max_bytes = Some(
                    v.parse::<u64>()
                        .map_err(|_| anyhow::anyhow!("invalid --max-bytes '{v}'"))?,
                );
            }
            "--offset" => {
                let v = next(args, &mut i, "--offset")?;
                p.offset = Some(
                    v.parse::<u64>()
                        .map_err(|_| anyhow::anyhow!("invalid --offset '{v}'"))?,
                );
            }
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

fn progress_printer() -> impl FnMut(Progress) -> bool {
    let mut last = String::new();
    move |p: Progress| {
        if p.current_path != last {
            eprintln!("  {}", p.current_path);
            last = p.current_path;
        }
        true
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
    print_archive_facts(&info);
    println!("{:>12}  {:>5}  NAME", "SIZE", "TYPE");
    for e in &info.entries {
        println!(
            "{:>12}  {:>5}  {}{}{}",
            e.size,
            if e.is_dir { "dir" } else { "file" },
            e.path,
            if e.encrypted { "  *" } else { "" },
            if e.split { "  (spans volumes)" } else { "" }
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
        match_mode: p.match_mode(),
        keep_broken: p.keep_broken,
        password: p.password,
        dest: PathBuf::from(dest),
        overwrite: p.overwrite,
        include: p.include,
        smart: p.smart,
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
        if !report.failed.is_empty() {
            println!("{} entries could not be extracted:", report.failed.len());
            for f in &report.failed {
                println!("  {f}");
            }
        }
        if !report.partial.is_empty() {
            println!(
                "{} files were written incomplete — as much as the archive held:",
                report.partial.len()
            );
            for f in &report.partial {
                println!("  {f}");
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_cat(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    if p.positionals.len() < 2 {
        anyhow::bail!("cat requires <archive> and <entry>");
    }
    let archive = &p.positionals[0];
    let entry = &p.positionals[1];
    let opts = ReadOptions {
        password: p.password,
        offset: p.offset.unwrap_or(0),
        max_bytes: p.max_bytes.unwrap_or(DEFAULT_READ_MAX_BYTES),
    };
    let content = read_entry(archive, entry, &opts)?;
    if p.json {
        let is_binary = content.bytes.contains(&0) || std::str::from_utf8(&content.bytes).is_err();
        let out = serde_json::json!({
            "path": content.path,
            "size": content.size,
            "offset": content.offset,
            "returned_bytes": content.bytes.len(),
            "truncated": content.truncated,
            "is_binary": is_binary,
            "text": std::str::from_utf8(&content.bytes).ok(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(&content.bytes)?;
        stdout.flush()?;
    }
    Ok(ExitCode::SUCCESS)
}

/// The things that are true of the archive rather than of one entry. Most
/// formats have none of them; RAR has all of them.
fn print_archive_facts(info: &ArchiveInfo) {
    if !info.volumes.is_empty() {
        println!("volumes: {} present", info.volumes.len());
        for v in &info.volumes {
            println!(
                "  {}",
                v.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            );
        }
    }
    if let Some(missing) = &info.missing_volume {
        println!(
            "INCOMPLETE: the next volume is missing — {}",
            missing.display()
        );
        println!("  (--keep-broken extracts what the volumes on hand contain)");
    }
    let a = &info.attributes;
    let mut flags = Vec::new();
    if a.solid {
        flags.push("solid");
    }
    if a.recovery_record {
        flags.push("recovery record");
    }
    if a.encrypted_headers {
        flags.push("encrypted headers");
    }
    if a.locked {
        flags.push("locked");
    }
    if !flags.is_empty() {
        println!("flags: {}", flags.join(", "));
    }
    if let Some(comment) = &info.comment {
        println!("comment:");
        for line in comment.lines() {
            println!("  {line}");
        }
    }
}

fn resolve_create_format(output: &Path, raw_name: &str) -> anyhow::Result<Format> {
    detect(output)
        .or_else(|| detect_by_extension(&raw_name.to_ascii_lowercase()))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cannot infer format from '{}' — use a known extension (.zip, .7z, .tar.gz, ...)",
                output.display()
            )
        })
}

fn cmd_create(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    if p.positionals.len() < 2 {
        anyhow::bail!("create requires <output> and at least one input");
    }
    let output = PathBuf::from(&p.positionals[0]);
    let inputs: Vec<PathBuf> = p.positionals[1..].iter().map(PathBuf::from).collect();
    let fmt = resolve_create_format(&output, &p.positionals[0])?;
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

fn cmd_add(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    if p.positionals.len() < 2 {
        anyhow::bail!("add requires <archive> and at least one input");
    }
    let archive = PathBuf::from(&p.positionals[0]);
    let inputs: Vec<PathBuf> = p.positionals[1..].iter().map(PathBuf::from).collect();
    let fmt = resolve_create_format(&archive, &p.positionals[0])?;
    let json = p.json;
    let opts = CreateOptions {
        format: fmt,
        level: parse_level(&p.level)?,
        password: p.password,
    };
    let mut pr = progress_printer();
    let report = add(&archive, &inputs, &opts, Some(&mut pr))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "updated {} ({}): added {} entries ({} bytes total on disk)",
            report.output.display(),
            report.format.label(),
            report.entries_added,
            report.bytes_out
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_remove(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    if p.positionals.is_empty() {
        anyhow::bail!("remove requires <archive> and at least one entry pattern");
    }
    let archive = PathBuf::from(&p.positionals[0]);
    let mut patterns: Vec<String> = p.positionals[1..].to_vec();
    patterns.extend(p.include.clone());
    if patterns.is_empty() {
        anyhow::bail!("remove requires at least one entry path or --include pattern");
    }
    let json = p.json;
    let match_mode = p.match_mode();
    let opts = RemoveOptions {
        password: p.password,
        include: patterns,
        match_mode,
        level: parse_level(&p.level)?,
    };
    let mut pr = progress_printer();
    let report = remove(&archive, &opts, Some(&mut pr))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "updated {} ({}): removed {} entries, {} remaining ({} bytes on disk)",
            report.output.display(),
            report.format.label(),
            report.entries_removed,
            report.entries_remaining,
            report.bytes_out
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_convert(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    if p.positionals.len() < 2 {
        anyhow::bail!("convert requires <source> and <output>");
    }
    let source = PathBuf::from(&p.positionals[0]);
    let output = PathBuf::from(&p.positionals[1]);
    let out_fmt = resolve_create_format(&output, &p.positionals[1])?;
    let json = p.json;
    let opts = ConvertOptions {
        read_password: p.password.clone(),
        create: CreateOptions {
            format: out_fmt,
            level: parse_level(&p.level)?,
            password: None,
        },
    };
    let mut pr = progress_printer();
    let report = convert(&source, &output, &opts, Some(&mut pr))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "converted {} ({}) -> {} ({}): {} entries, {} bytes",
            report.source.display(),
            report.source_format.label(),
            report.output.display(),
            report.output_format.label(),
            report.entries_converted,
            report.bytes_out
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_grep(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    if p.positionals.len() < 2 {
        anyhow::bail!("grep requires <pattern> and <archive>");
    }
    let pattern = &p.positionals[0];
    let archive = &p.positionals[1];
    let json = p.json;
    let match_mode = p.match_mode();
    let opts = GrepOptions {
        password: p.password,
        case_insensitive: p.ignore_case,
        include: p.include.clone(),
        match_mode,
        max_matches: p.max_matches.unwrap_or(DEFAULT_GREP_MAX_MATCHES),
        max_file_size: p.max_bytes.unwrap_or(DEFAULT_GREP_MAX_FILE_SIZE),
    };
    let report = grep(archive, pattern, &opts, None)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(ExitCode::SUCCESS);
    }
    for m in &report.matches {
        println!("{}:{}:{}", m.path, m.line_number, m.line);
    }
    if report.truncated {
        eprintln!(
            "  (truncated at {} matches — pass --max-matches <N> for more)",
            report.matches.len()
        );
    }
    Ok(if report.matches.is_empty() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
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
    let Some(fmt) = detect(std::path::Path::new(path)) else {
        eprintln!("{path}: unrecognized archive format");
        return Ok(ExitCode::FAILURE);
    };

    let info = list(
        path,
        &ListOptions {
            password: p.password,
        },
    );

    if p.json {
        match &info {
            Ok(info) => println!("{}", serde_json::to_string_pretty(info)?),
            Err(e) => println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "path": path,
                    "format": fmt.extension(),
                    "can_create": fmt.can_create(),
                    "error": e.to_string(),
                }))?
            ),
        }
        return Ok(ExitCode::SUCCESS);
    }

    println!(
        "{}: {} (create supported: {})",
        path,
        fmt.label(),
        fmt.can_create()
    );
    match info {
        Ok(info) => {
            println!(
                "{} entries, {} bytes uncompressed, {} bytes on disk{}",
                info.entries.len(),
                info.total_size,
                info.total_compressed,
                if info.encrypted { ", encrypted" } else { "" }
            );
            print_archive_facts(&info);
        }
        Err(e) => println!("could not read the contents: {e}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_extract_here(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    let reports = shell::extract_here_batch(&p.positionals, p.password, p.keep_broken)?;
    for (a, r) in p.positionals.iter().zip(reports.iter()) {
        println!("extracted {a} -> {}", r.dest.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_compress_zip(args: &[String]) -> anyhow::Result<ExitCode> {
    let p = parse(args)?;
    let report = shell::compress_zip_batch(&p.positionals, parse_level(&p.level)?, p.password)?;
    println!(
        "created {} ({} entries)",
        report.output.display(),
        report.entries_added
    );
    Ok(ExitCode::SUCCESS)
}
