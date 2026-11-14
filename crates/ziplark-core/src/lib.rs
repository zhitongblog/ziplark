//! # ziplark-core
//!
//! The Ziplark archive engine. One implementation of every archive operation,
//! shared verbatim by the GUI, the CLI (`ziplark`) and the MCP server. Nothing
//! in this crate knows about a UI — it takes paths and options and does work.
//!
//! Supported formats:
//!
//! | Format        | List | Extract | Create | Read Entry |
//! |---------------|------|---------|--------|------------|
//! | ZIP (+AES256, `.z01`, `.crx`, `.jar`/`.apk`/`.whl`/…) | ✅ | ✅ | ✅ | ✅ |
//! | 7z (+AES256, `.7z.001`) | ✅ | ✅ | ✅ | ✅ |
//! | RAR / RAR5 (multi-volume, SFX) | ✅ | ✅ | — | ✅ |
//! | TAR (+gz/bz2/xz/zst/lz4) | ✅ | ✅ | ✅ | ✅ |
//! | gz/bz2/xz/zst/lz4 (single stream) | ✅ | ✅ | ✅ | ✅ |
//! | ISO 9660 / Joliet (disc image) | ✅ | ✅ | — | ✅ |
//! | DEB / AR (`.deb`, `.ar`, `.a`) | ✅ | ✅ | — | ✅ |
//! | RPM / CPIO (`.rpm`, `.cpio`) | ✅ | ✅ | — | ✅ |
//! | CAB (`.cab`) | ✅ | ✅ | — | ✅ |

mod detect;
mod encoding;
mod error;
mod formats;
mod matcher;
mod model;
pub mod shell;
pub mod volumes;

pub use detect::{detect, detect_by_extension};
pub use error::{Error, Result};
pub use matcher::{MatchMode, Selector};
pub use model::{
    ArchiveAttributes, ArchiveEntry, ArchiveInfo, ConvertReport, CreateReport, DiffEntry,
    DiffReport, DiffStatus, EntryContent, EntryHash, ExtractReport, Format, GrepMatch, GrepReport,
    HashAlgorithm, HashReport, Progress, RemoveReport, RenameReport, TestReport,
};
pub use volumes::VolumeSet;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Compression effort, mapped per-format to the nearest supported level.
#[derive(Debug, Default, Clone, Copy)]
pub enum Level {
    Store,
    Fast,
    #[default]
    Default,
    Best,
}

/// Options for listing an archive.
#[derive(Debug, Default, Clone)]
pub struct ListOptions {
    pub password: Option<String>,
}

/// Default cap when reading an entry into memory (4 MiB).
pub const DEFAULT_READ_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Options for reading a single entry's bytes directly into memory without
/// writing to disk.
#[derive(Debug, Clone)]
pub struct ReadOptions {
    pub password: Option<String>,
    /// Byte offset inside the entry to start reading from (default 0).
    pub offset: u64,
    /// Maximum number of bytes to return (default 4 MiB).
    pub max_bytes: u64,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            password: None,
            offset: 0,
            max_bytes: DEFAULT_READ_MAX_BYTES,
        }
    }
}

/// Options for extracting an archive.
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    pub password: Option<String>,
    /// Directory to extract into (created if missing).
    pub dest: PathBuf,
    /// Overwrite existing files instead of erroring.
    pub overwrite: bool,
    /// Only extract entries matching one of these patterns
    /// (empty = everything). How a pattern is compared is `match_mode`.
    pub include: Vec<String>,
    /// How `include` patterns are matched. [`MatchMode::Auto`] is the
    /// forgiving substring/glob behaviour; [`MatchMode::Exact`] extracts
    /// exactly the entries named, which is what a caller that already has a
    /// listing — a GUI with ticked rows, an agent picking one file — wants.
    pub match_mode: MatchMode,
    /// Keep going when an entry cannot be extracted, listing the casualties in
    /// [`ExtractReport::failed`] instead of failing the whole operation. This
    /// is how you get the readable files out of a damaged or incomplete
    /// archive; without it the first bad entry stops everything.
    pub keep_broken: bool,
    /// Anti-tarbomb smart destination: if the archive has multiple top-level
    /// items, automatically extract into `dest/<archive-stem>/` so files never
    /// scatter across `dest`; if all entries already sit inside a single
    /// top-level folder (or a single file), extract directly into `dest`
    /// without creating a redundant `dest/<stem>/<stem>/` wrapper.
    pub smart: bool,
}

impl ExtractOptions {
    pub fn new(dest: impl Into<PathBuf>) -> Self {
        Self {
            password: None,
            dest: dest.into(),
            overwrite: false,
            include: Vec::new(),
            match_mode: MatchMode::Auto,
            keep_broken: false,
            smart: false,
        }
    }

    /// The entry selector these options describe.
    pub fn selector(&self) -> Selector {
        Selector::new(&self.include, self.match_mode)
    }
}

/// Options for creating an archive.
#[derive(Debug, Clone)]
pub struct CreateOptions {
    pub format: Format,
    pub level: Level,
    /// AES-256 password (ZIP / 7z). Ignored by formats that can't encrypt.
    pub password: Option<String>,
    /// Optional maximum volume size in bytes (`--split`). When set (`> 0`) and
    /// the created archive exceeds `split_size`, the output is sliced into
    /// `<output>.001`, `<output>.002`, … volume files.
    pub split_size: Option<u64>,
}

impl CreateOptions {
    pub fn new(format: Format) -> Self {
        Self {
            format,
            level: Level::Default,
            password: None,
            split_size: None,
        }
    }
}

/// Parse a human-friendly byte size string such as `"100M"`, `"1.5G"`, `"64K"`,
/// `"500MB"`, or raw bytes `"1048576"`.
pub fn parse_byte_size(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        return Err(Error::other("split size must not be empty"));
    }
    let upper = s.to_ascii_uppercase();
    let (num_str, mult): (&str, f64) = if let Some(n) = upper.strip_suffix("GIB").or_else(|| upper.strip_suffix("GB")).or_else(|| upper.strip_suffix('G')) {
        (n.trim(), 1024.0 * 1024.0 * 1024.0)
    } else if let Some(n) = upper.strip_suffix("MIB").or_else(|| upper.strip_suffix("MB")).or_else(|| upper.strip_suffix('M')) {
        (n.trim(), 1024.0 * 1024.0)
    } else if let Some(n) = upper.strip_suffix("KIB").or_else(|| upper.strip_suffix("KB")).or_else(|| upper.strip_suffix('K')) {
        (n.trim(), 1024.0)
    } else if let Some(n) = upper.strip_suffix('B') {
        (n.trim(), 1.0)
    } else {
        (upper.as_str(), 1.0)
    };
    let val: f64 = num_str
        .parse()
        .map_err(|_| Error::other(format!("invalid byte size: {s}")))?;
    if !val.is_finite() || val <= 0.0 {
        return Err(Error::other(format!("split size must be positive: {s}")));
    }
    let bytes = (val * mult).round() as u64;
    if bytes < 64 * 1024 {
        return Err(Error::other("split size must be at least 64 KiB"));
    }
    Ok(bytes)
}

/// Default maximum matches returned by [`grep`].
pub const DEFAULT_GREP_MAX_MATCHES: usize = 500;
/// Default maximum uncompressed size of a single file scanned by [`grep`] (16 MiB).
pub const DEFAULT_GREP_MAX_FILE_SIZE: u64 = 16 * 1024 * 1024;

/// Options for searching text inside an archive ([`grep`]).
#[derive(Debug, Clone)]
pub struct GrepOptions {
    pub password: Option<String>,
    /// Case-insensitive substring matching (`-i`).
    pub case_insensitive: bool,
    /// Only search entries matching one of these path patterns (empty = all).
    pub include: Vec<String>,
    pub match_mode: MatchMode,
    /// Stop after finding this many matching lines (default 500).
    pub max_matches: usize,
    /// Skip entries larger than this many bytes (default 16 MiB).
    pub max_file_size: u64,
}

impl Default for GrepOptions {
    fn default() -> Self {
        Self {
            password: None,
            case_insensitive: false,
            include: Vec::new(),
            match_mode: MatchMode::Auto,
            max_matches: DEFAULT_GREP_MAX_MATCHES,
            max_file_size: DEFAULT_GREP_MAX_FILE_SIZE,
        }
    }
}

impl GrepOptions {
    pub fn selector(&self) -> Selector {
        Selector::new(&self.include, self.match_mode)
    }
}

/// Options for removing entries from an archive ([`remove`]).
#[derive(Debug, Clone)]
pub struct RemoveOptions {
    pub password: Option<String>,
    pub include: Vec<String>,
    pub match_mode: MatchMode,
    pub level: Level,
}

impl Default for RemoveOptions {
    fn default() -> Self {
        Self {
            password: None,
            include: Vec::new(),
            match_mode: MatchMode::Auto,
            level: Level::Default,
        }
    }
}

impl RemoveOptions {
    pub fn selector(&self) -> Selector {
        Selector::new(&self.include, self.match_mode)
    }
}

/// Options for converting an archive from one format to another ([`convert`]).
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    /// Password for unlocking the source archive, if encrypted.
    pub read_password: Option<String>,
    /// Options for creating the destination archive.
    pub create: CreateOptions,
}

/// Options for renaming or moving entries inside an archive ([`rename`]).
#[derive(Debug, Default, Clone)]
pub struct RenameOptions {
    pub password: Option<String>,
    pub level: Level,
}

/// Options for computing entry hashes inside an archive ([`hash`]).
#[derive(Debug, Default, Clone)]
pub struct HashOptions {
    pub password: Option<String>,
    pub algorithm: HashAlgorithm,
    pub include: Vec<String>,
    pub match_mode: MatchMode,
}

impl HashOptions {
    pub fn selector(&self) -> Selector {
        Selector::new(&self.include, self.match_mode)
    }
}

/// Options for comparing two archives ([`diff`]).
#[derive(Debug, Default, Clone)]
pub struct DiffOptions {
    pub left_password: Option<String>,
    pub right_password: Option<String>,
    /// Generate unified text diff patches for modified text files.
    pub content: bool,
    pub include: Vec<String>,
    pub match_mode: MatchMode,
}

impl DiffOptions {
    pub fn selector(&self) -> Selector {
        Selector::new(&self.include, self.match_mode)
    }
}


/// A progress callback. The engine calls it as work proceeds — between entries,
/// and part-way through large ones — and **returning `false` asks it to stop**,
/// which surfaces to the caller as [`Error::Cancelled`]. Use `|_| true` if you
/// only want the reports, or pass `None` if you want neither.
///
/// A cancelled operation leaves whatever it had already written on disk; it is
/// the caller's job to clean up if that matters.
pub type ProgressFn<'a> = &'a mut dyn FnMut(Progress) -> bool;

fn noop_progress(_: Progress) -> bool {
    true
}

/// Strip archive extensions (including compound ones like `.tar.gz`, `.part01.rar`,
/// `.7z.001`) to produce a clean folder name for smart extraction.
pub fn archive_stem(name: &str) -> String {
    let mut work = name;
    let lower = work.to_ascii_lowercase();

    // Strip `.001` .. `.9999` suffix if preceded by another dot.
    if let Some((base, ext)) = work.rsplit_once('.') {
        if (2..=4).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_digit()) && base.contains('.') {
            work = base;
        }
    }

    let l = work.to_ascii_lowercase();
    for ext in [
        ".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tar.lz4", ".cpio.gz", ".cpio.bz2",
        ".cpio.xz", ".cpio.zst", ".cpio.lz4",
    ] {
        if l.ends_with(ext) {
            return work[..work.len() - ext.len()].to_string();
        }
    }

    // Strip `.partNN.rar`
    if let Some(head) = l.strip_suffix(".rar") {
        if let Some(cut) = head.rfind(".part") {
            let digits = &head[cut + 5..];
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) && cut > 0 {
                return work[..cut].to_string();
            }
        }
    }
    let _ = lower;

    match work.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => work.to_string(),
    }
}

/// Inspect `entries` and return `Some(single_root_name)` if every entry in the
/// archive sits under a single top-level item (ignoring macOS `__MACOSX`
/// resource-fork clutter), or `None` if the archive has multiple top-level items
/// ("tarbomb").
pub fn single_top_level_item(entries: &[ArchiveEntry]) -> Option<String> {
    let mut roots: HashSet<String> = HashSet::new();
    for e in entries {
        let p = e.path.replace('\\', "/");
        let p = p.trim_start_matches("./").trim_matches('/');
        if p.is_empty() {
            continue;
        }
        let first = p.split('/').next().unwrap_or("");
        if first.is_empty() || first == "__MACOSX" || first == ".DS_Store" {
            continue;
        }
        roots.insert(first.to_string());
        if roots.len() > 1 {
            return None;
        }
    }
    roots.into_iter().next()
}

/// Compute the smart destination directory for extracting `archive_path` into
/// `base_dest`.
///
/// * If the archive has **multiple** top-level entries (a tarbomb), returns
///   `base_dest/<archive-stem>` so files are grouped into one folder.
/// * If the archive has **a single** top-level directory or file `root`, returns
///   `base_dest` directly (avoiding a redundant `base_dest/<stem>/<root>`
///   wrapper) — unless `avoid_clobber` is set and `base_dest/<root>` already
///   exists on disk, in which case a unique `base_dest/<stem> (N)` folder is
///   chosen so nothing existing is overwritten.
pub fn smart_dest(
    archive_path: &Path,
    base_dest: &Path,
    entries: &[ArchiveEntry],
    avoid_clobber: bool,
) -> PathBuf {
    let fname = archive_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Archive");
    let stem = archive_stem(fname);

    let dest_name = base_dest.file_name().and_then(|s| s.to_str());

    match single_top_level_item(entries) {
        Some(root) => {
            let root_is_dir = entries.iter().any(|e| {
                let p = e.path.replace('\\', "/");
                let p = p.trim_start_matches("./").trim_matches('/');
                e.is_dir || p.contains('/')
            });
            if root_is_dir && dest_name == Some(root.as_str()) {
                if let Some(parent) = base_dest.parent() {
                    if !parent.as_os_str().is_empty() && (!avoid_clobber || !base_dest.exists()) {
                        return parent.to_path_buf();
                    }
                }
            }
            let target_on_disk = base_dest.join(&root);
            if avoid_clobber && target_on_disk.exists() {
                unique_sibling_dir(base_dest.join(&stem))
            } else {
                base_dest.to_path_buf()
            }
        }
        None => {
            let sub = if dest_name == Some(stem.as_str()) {
                base_dest.to_path_buf()
            } else {
                base_dest.join(&stem)
            };
            if avoid_clobber {
                unique_sibling_dir(sub)
            } else {
                sub
            }
        }
    }
}

/// Pick a non-existent path by appending `" (2)"`, `" (3)"`, … if `path`
/// already exists.
pub fn unique_sibling_dir(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("Archive");
    for n in 2..1000 {
        let cand = parent.join(format!("{name} ({n})"));
        if !cand.exists() {
            return cand;
        }
    }
    path
}

/// List the contents of an archive without extracting.
pub fn list(path: impl AsRef<Path>, opts: &ListOptions) -> Result<ArchiveInfo> {
    let path = path.as_ref();
    let fmt = detect(path).ok_or_else(|| Error::UnsupportedFormat(Some(path.to_path_buf())))?;
    match fmt {
        Format::Zip => formats::zip::list(path, fmt, opts),
        Format::SevenZ => formats::sevenz::list(path, fmt, opts),
        Format::Rar => formats::rar::list(path, fmt, opts),
        Format::Tar
        | Format::TarGz
        | Format::TarBz2
        | Format::TarXz
        | Format::TarZst
        | Format::TarLz4 => formats::tar::list(path, fmt, opts),
        Format::Gz | Format::Bz2 | Format::Xz | Format::Zst | Format::Lz4 => {
            formats::stream::list(path, fmt, opts)
        }
        Format::Iso => formats::iso::list(path, fmt, opts),
        Format::Ar | Format::Deb => formats::ar::list(path, fmt, opts),
        Format::Cpio | Format::Rpm => formats::cpio::list(path, fmt, opts),
        Format::Cab => formats::cab::list(path, fmt, opts),
        Format::Xar | Format::Pkg => formats::xar::list(path, fmt, opts),
    }
}

/// Read a single entry's bytes from an archive into memory without writing
/// anything to disk.
pub fn read_entry(
    path: impl AsRef<Path>,
    entry_path: &str,
    opts: &ReadOptions,
) -> Result<EntryContent> {
    let path = path.as_ref();
    let fmt = detect(path).ok_or_else(|| Error::UnsupportedFormat(Some(path.to_path_buf())))?;
    match fmt {
        Format::Zip => formats::zip::read_entry(path, entry_path, opts),
        Format::SevenZ => formats::sevenz::read_entry(path, entry_path, opts),
        Format::Rar => formats::rar::read_entry(path, entry_path, opts),
        Format::Tar
        | Format::TarGz
        | Format::TarBz2
        | Format::TarXz
        | Format::TarZst
        | Format::TarLz4 => formats::tar::read_entry(path, fmt, entry_path, opts),
        Format::Gz | Format::Bz2 | Format::Xz | Format::Zst | Format::Lz4 => {
            formats::stream::read_entry(path, fmt, entry_path, opts)
        }
        Format::Iso => formats::iso::read_entry(path, entry_path, opts),
        Format::Ar | Format::Deb => formats::ar::read_entry(path, fmt, entry_path, opts),
        Format::Cpio | Format::Rpm => formats::cpio::read_entry(path, fmt, entry_path, opts),
        Format::Cab => formats::cab::read_entry(path, entry_path, opts),
        Format::Xar | Format::Pkg => formats::xar::read_entry(path, fmt, entry_path, opts),
    }
}

/// Extract an archive to `opts.dest` (or a smart subfolder if `opts.smart` is set).
pub fn extract(
    path: impl AsRef<Path>,
    opts: &ExtractOptions,
    progress: Option<ProgressFn<'_>>,
) -> Result<ExtractReport> {
    let path = path.as_ref();
    let fmt = detect(path).ok_or_else(|| Error::UnsupportedFormat(Some(path.to_path_buf())))?;

    let resolved_opts;
    let opts = if opts.smart && opts.include.is_empty() {
        let info = list(
            path,
            &ListOptions {
                password: opts.password.clone(),
            },
        )?;
        let effective_dest = smart_dest(&info.path, &opts.dest, &info.entries, !opts.overwrite);
        resolved_opts = ExtractOptions {
            dest: effective_dest,
            smart: false,
            ..opts.clone()
        };
        &resolved_opts
    } else {
        opts
    };

    let mut local = noop_progress;
    let progress: ProgressFn = progress.unwrap_or(&mut local);
    match fmt {
        Format::Zip => formats::zip::extract(path, opts, progress),
        Format::SevenZ => formats::sevenz::extract(path, opts, progress),
        Format::Rar => formats::rar::extract(path, opts, progress),
        Format::Tar
        | Format::TarGz
        | Format::TarBz2
        | Format::TarXz
        | Format::TarZst
        | Format::TarLz4 => formats::tar::extract(path, fmt, opts, progress),
        Format::Gz | Format::Bz2 | Format::Xz | Format::Zst | Format::Lz4 => {
            formats::stream::extract(path, fmt, opts, progress)
        }
        Format::Iso => formats::iso::extract(path, opts, progress),
        Format::Ar | Format::Deb => formats::ar::extract(path, fmt, opts, progress),
        Format::Cpio | Format::Rpm => formats::cpio::extract(path, fmt, opts, progress),
        Format::Cab => formats::cab::extract(path, opts, progress),
        Format::Xar | Format::Pkg => formats::xar::extract(path, fmt, opts, progress),
    }
}

/// Create an archive at `output` containing `inputs` (files and/or directories).
pub fn create(
    output: impl AsRef<Path>,
    inputs: &[PathBuf],
    opts: &CreateOptions,
    progress: Option<ProgressFn<'_>>,
) -> Result<CreateReport> {
    let output = output.as_ref();
    if !opts.format.can_create() {
        return Err(Error::CreateUnsupported(opts.format.label().to_string()));
    }
    // Strip a trailing `.001` suffix if the caller passed `archive.zip.001` as
    // the output path when requesting `--split`.
    let base_output = strip_volume_suffix(output);
    let mut local = noop_progress;
    let progress: ProgressFn = progress.unwrap_or(&mut local);
    let mut rep = match opts.format {
        Format::Zip => formats::zip::create(&base_output, inputs, opts, progress),
        Format::SevenZ => formats::sevenz::create(&base_output, inputs, opts, progress),
        Format::Tar
        | Format::TarGz
        | Format::TarBz2
        | Format::TarXz
        | Format::TarZst
        | Format::TarLz4 => formats::tar::create(&base_output, inputs, opts, progress),
        Format::Gz | Format::Bz2 | Format::Xz | Format::Zst | Format::Lz4 => {
            formats::stream::create(&base_output, inputs, opts, progress)
        }
        other => Err(Error::CreateUnsupported(other.label().into())),
    }?;

    if let Some(split_bytes) = opts.split_size.filter(|&n| n > 0) {
        if split_bytes < 64 * 1024 {
            let _ = std::fs::remove_file(&base_output);
            return Err(Error::other("split size must be at least 64 KiB"));
        }
        let volumes = slice_into_volumes(&base_output, split_bytes)?;
        if let Some(first) = volumes.first() {
            rep.output = first.clone();
        }
        rep.volumes = volumes;
    }

    Ok(rep)
}

fn strip_volume_suffix(path: &Path) -> PathBuf {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if let Some((base, ext)) = name.rsplit_once('.') {
            if (2..=4).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_digit()) && base.contains('.') {
                return path.with_file_name(base);
            }
        }
    }
    path.to_path_buf()
}

/// Slice a newly created archive file at `archive_path` into fixed-size
/// `<archive_path>.001`, `<archive_path>.002`, … volume files and remove the
/// unsplit file.
fn slice_into_volumes(archive_path: &Path, split_bytes: u64) -> Result<Vec<PathBuf>> {
    use std::io::{Read, Write};
    let mut src = std::fs::File::open(archive_path)?;
    let total_len = src.metadata()?.len();
    let base_name = archive_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::other("invalid output archive filename"))?;

    let mut volumes = Vec::new();
    let mut buf = vec![0u8; 128 * 1024];
    let mut remaining_total = total_len;
    let mut part_num = 1usize;

    while remaining_total > 0 || part_num == 1 {
        let vol_path = archive_path.with_file_name(format!("{base_name}.{part_num:03}"));
        let mut out = std::fs::File::create(&vol_path)?;
        let mut written_in_vol = 0u64;
        while written_in_vol < split_bytes {
            let max_chunk = ((split_bytes - written_in_vol).min(buf.len() as u64)) as usize;
            let n = src.read(&mut buf[..max_chunk])?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            written_in_vol += n as u64;
            remaining_total = remaining_total.saturating_sub(n as u64);
        }
        out.flush()?;
        volumes.push(vol_path);
        part_num += 1;
        if remaining_total == 0 {
            break;
        }
    }
    drop(src);
    let _ = std::fs::remove_file(archive_path);
    Ok(volumes)
}

/// Add (or replace) `inputs` inside an existing archive at `archive`.
///
/// If `archive` does not exist yet on disk, creates it fresh. For existing
/// `.zip` archives, untouched entries are copied raw without recompression;
/// for `.7z` and `.tar.*`, the archive is updated atomically via a temporary
/// staging directory.
pub fn add(
    archive: impl AsRef<Path>,
    inputs: &[PathBuf],
    opts: &CreateOptions,
    progress: Option<ProgressFn<'_>>,
) -> Result<CreateReport> {
    let archive = archive.as_ref();
    if !archive.exists() {
        return create(archive, inputs, opts, progress);
    }
    let fmt = detect(archive).unwrap_or(opts.format);
    if !fmt.can_modify() {
        return Err(Error::CreateUnsupported(fmt.label().to_string()));
    }
    let mut local = noop_progress;
    let progress: ProgressFn = progress.unwrap_or(&mut local);

    if fmt == Format::Zip {
        return formats::zip::add(archive, inputs, opts, progress);
    }

    // For 7z and tar variants: unpack existing entries into a clean temporary
    // staging directory, overlay `inputs`, and re-pack atomically (or overwrite
    // in place when running inside a sandbox without parent-dir write access).
    let stage = std::env::temp_dir().join(format!(
        "ziplark-add-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&stage)?;

    let result = (|| -> Result<CreateReport> {
        let mut ex_opts = ExtractOptions::new(&stage);
        ex_opts.overwrite = true;
        ex_opts.password = opts.password.clone();
        extract(archive, &ex_opts, None)?;

        // Copy each new input into `stage/<file_name>`.
        for input in inputs {
            let name = input
                .file_name()
                .ok_or_else(|| Error::other(format!("invalid input path: {}", input.display())))?;
            let dst = stage.join(name);
            copy_tree(input, &dst)?;
        }

        let mut merged_inputs = Vec::new();
        for entry in std::fs::read_dir(&stage)? {
            let entry = entry?;
            merged_inputs.push(entry.path());
        }
        merged_inputs.sort();

        let (tmp_archive, tmp_file) = formats::zip::create_temp_for(archive, "add.tmp")?;
        drop(tmp_file);
        let create_opts = CreateOptions {
            format: fmt,
            level: opts.level,
            password: opts.password.clone(),
            split_size: None,
        };
        let mut rep = create(&tmp_archive, &merged_inputs, &create_opts, Some(progress))?;
        formats::zip::replace_archive_file(&tmp_archive, archive)?;
        rep.output = archive.to_path_buf();
        Ok(rep)
    })();

    let _ = std::fs::remove_dir_all(&stage);
    result
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    let ty = meta.file_type();
    if ty.is_symlink() {
        let target = std::fs::read_link(src)?;
        if std::fs::symlink_metadata(dst).is_ok() {
            let _ = std::fs::remove_file(dst);
        }
        formats::create_symlink(&target.to_string_lossy(), dst)?;
    } else if ty.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        if let Some(p) = dst.parent() {
            std::fs::create_dir_all(p)?;
        }
        if std::fs::symlink_metadata(dst).is_ok() {
            let _ = std::fs::remove_file(dst);
        }
        std::fs::copy(src, dst)?;
        let mtime = filetime::FileTime::from_last_modification_time(&meta);
        let _ = filetime::set_file_mtime(dst, mtime);
    }
    Ok(())
}

/// Test archive integrity by decompressing every entry and checking it.
pub fn test(
    path: impl AsRef<Path>,
    opts: &ListOptions,
    progress: Option<ProgressFn<'_>>,
) -> Result<TestReport> {
    let path = path.as_ref();
    let fmt = detect(path).ok_or_else(|| Error::UnsupportedFormat(Some(path.to_path_buf())))?;
    let mut local = noop_progress;
    let progress: ProgressFn = progress.unwrap_or(&mut local);
    match fmt {
        Format::Zip => formats::zip::test(path, opts, progress),
        Format::SevenZ => formats::sevenz::test(path, opts, progress),
        Format::Rar => formats::rar::test(path, opts, progress),
        Format::Tar
        | Format::TarGz
        | Format::TarBz2
        | Format::TarXz
        | Format::TarZst
        | Format::TarLz4 => formats::tar::test(path, fmt, opts, progress),
        Format::Gz | Format::Bz2 | Format::Xz | Format::Zst | Format::Lz4 => {
            formats::stream::test(path, fmt, opts, progress)
        }
        Format::Iso => formats::iso::test(path, opts, progress),
        Format::Ar | Format::Deb => formats::ar::test(path, fmt, opts, progress),
        Format::Cpio | Format::Rpm => formats::cpio::test(path, fmt, opts, progress),
        Format::Cab => formats::cab::test(path, opts, progress),
        Format::Xar | Format::Pkg => formats::xar::test(path, fmt, opts, progress),
    }
}

/// Remove entries matching `opts.include` from an existing `.zip`, `.7z`, or
/// `.tar.*` archive atomically.
pub fn remove(
    archive: impl AsRef<Path>,
    opts: &RemoveOptions,
    progress: Option<ProgressFn<'_>>,
) -> Result<RemoveReport> {
    let archive = archive.as_ref();
    let selector = opts.selector();
    if selector.matches_everything() {
        return Err(Error::other(
            "refusing to remove entries without at least one pattern",
        ));
    }
    let fmt = detect(archive).ok_or_else(|| Error::UnsupportedFormat(Some(archive.to_path_buf())))?;
    if !fmt.can_modify() {
        return Err(Error::CreateUnsupported(fmt.label().to_string()));
    }
    let mut local = noop_progress;
    let progress: ProgressFn = progress.unwrap_or(&mut local);

    if fmt == Format::Zip {
        return formats::zip::remove(archive, &selector, progress);
    }

    // For 7z and tar variants: check if any entry matches first.
    let info = list(
        archive,
        &ListOptions {
            password: opts.password.clone(),
        },
    )?;
    let total_entries = info.entries.len() as u64;
    let entries_removed = info
        .entries
        .iter()
        .filter(|e| selector.matches(&e.path))
        .count() as u64;

    if entries_removed == 0 {
        let bytes_out = std::fs::metadata(archive)?.len();
        return Ok(RemoveReport {
            output: archive.to_path_buf(),
            format: fmt,
            entries_removed: 0,
            entries_remaining: total_entries,
            bytes_out,
        });
    }

    let stage = std::env::temp_dir().join(format!(
        "ziplark-rm-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&stage)?;

    let result = (|| -> Result<RemoveReport> {
        let mut ex_opts = ExtractOptions::new(&stage);
        ex_opts.overwrite = true;
        ex_opts.password = opts.password.clone();
        extract(archive, &ex_opts, None)?;

        // Delete matched paths inside `stage` (deepest paths first so children
        // are removed before parents).
        let mut matched_paths: Vec<String> = info
            .entries
            .iter()
            .filter(|e| selector.matches(&e.path))
            .map(|e| e.path.clone())
            .collect();
        matched_paths.sort_by(|a, b| b.len().cmp(&a.len()));

        for rel in &matched_paths {
            if let Ok(target) = formats::safe_join(&stage, rel) {
                if let Ok(md) = std::fs::symlink_metadata(&target) {
                    if md.file_type().is_dir() && !md.file_type().is_symlink() {
                        let _ = std::fs::remove_dir_all(&target);
                    } else {
                        let _ = std::fs::remove_file(&target);
                    }
                }
            }
        }

        let mut remaining_inputs = Vec::new();
        for entry in std::fs::read_dir(&stage)? {
            let entry = entry?;
            remaining_inputs.push(entry.path());
        }
        remaining_inputs.sort();

        let (tmp_archive, tmp_file) = formats::zip::create_temp_for(archive, "rm.tmp")?;
        drop(tmp_file);
        let create_opts = CreateOptions {
            format: fmt,
            level: opts.level,
            password: opts.password.clone(),
            split_size: None,
        };
        let rep = create(&tmp_archive, &remaining_inputs, &create_opts, Some(progress))?;
        formats::zip::replace_archive_file(&tmp_archive, archive)?;
        let bytes_out = std::fs::metadata(archive)?.len();
        Ok(RemoveReport {
            output: archive.to_path_buf(),
            format: fmt,
            entries_removed,
            entries_remaining: rep.entries_added,
            bytes_out,
        })
    })();

    let _ = std::fs::remove_dir_all(&stage);
    result
}

/// Convert an archive from any supported readable format (including RAR, 7z,
/// CAB, DEB, RPM, ISO, ZIP, TAR.*) into `output` in `opts.create.format`.
pub fn convert(
    source: impl AsRef<Path>,
    output: impl AsRef<Path>,
    opts: &ConvertOptions,
    progress: Option<ProgressFn<'_>>,
) -> Result<ConvertReport> {
    let source = source.as_ref();
    let output = output.as_ref();
    let source_format =
        detect(source).ok_or_else(|| Error::UnsupportedFormat(Some(source.to_path_buf())))?;
    if !opts.create.format.can_create() {
        return Err(Error::CreateUnsupported(
            opts.create.format.label().to_string(),
        ));
    }

    let mut local = noop_progress;
    let progress: ProgressFn = progress.unwrap_or(&mut local);

    if let Some(out_parent) = output.parent() {
        if !out_parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(out_parent);
        }
    }
    let stage = std::env::temp_dir().join(format!(
        "ziplark-conv-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&stage)?;

    let result = (|| -> Result<ConvertReport> {
        let mut ex_opts = ExtractOptions::new(&stage);
        ex_opts.overwrite = true;
        ex_opts.password = opts.read_password.clone();
        extract(source, &ex_opts, Some(progress))?;

        let mut inputs = Vec::new();
        for entry in std::fs::read_dir(&stage)? {
            let entry = entry?;
            inputs.push(entry.path());
        }
        inputs.sort();

        let (tmp_output, tmp_file) = formats::zip::create_temp_for(output, "conv.tmp")?;
        drop(tmp_file);
        let rep = create(&tmp_output, &inputs, &opts.create, Some(progress))?;
        formats::zip::replace_archive_file(&tmp_output, output)?;
        let bytes_out = std::fs::metadata(output)?.len();

        Ok(ConvertReport {
            source: source.to_path_buf(),
            source_format,
            output: output.to_path_buf(),
            output_format: opts.create.format,
            entries_converted: rep.entries_added,
            bytes_in: rep.bytes_in,
            bytes_out,
        })
    })();

    let _ = std::fs::remove_dir_all(&stage);
    result
}

/// Search for text matching `pattern` across files inside an archive without
/// extracting anything to disk.
pub fn grep(
    path: impl AsRef<Path>,
    pattern: &str,
    opts: &GrepOptions,
    progress: Option<ProgressFn<'_>>,
) -> Result<GrepReport> {
    if pattern.is_empty() {
        return Err(Error::other("search pattern must not be empty"));
    }
    let path = path.as_ref();
    let fmt = detect(path).ok_or_else(|| Error::UnsupportedFormat(Some(path.to_path_buf())))?;
    let mut local = noop_progress;
    let progress: ProgressFn = progress.unwrap_or(&mut local);

    let selector = opts.selector();
    let max_matches = opts.max_matches.max(1);
    let max_file_size = if opts.max_file_size == 0 {
        DEFAULT_GREP_MAX_FILE_SIZE
    } else {
        opts.max_file_size
    };
    let needle = if opts.case_insensitive {
        pattern.to_lowercase()
    } else {
        pattern.to_string()
    };

    let mut report = GrepReport {
        pattern: pattern.to_string(),
        matches: Vec::new(),
        files_searched: 0,
        files_matched: 0,
        truncated: false,
    };

    let mut scan_one = |entry_path: &str, _size: u64, rd: &mut dyn std::io::Read| -> Result<bool> {
        if is_binary_extension(entry_path) {
            return Ok(true);
        }
        formats::report(
            progress,
            Progress {
                current_path: entry_path.to_string(),
                entries_done: report.files_searched,
                entries_total: 0,
                bytes_done: 0,
                bytes_total: 0,
            },
        )?;

        let mut bytes = Vec::new();
        {
            use std::io::Read as _;
            rd.take(max_file_size).read_to_end(&mut bytes)?;
        }
        if bytes.is_empty() {
            return Ok(true);
        }
        // Follow ripgrep / git-grep heuristic: if the first 8 KiB contains a
        // NUL byte, treat the entry as binary and skip it.
        let probe_len = bytes.len().min(8192);
        if bytes[..probe_len].contains(&0) {
            return Ok(true);
        }

        report.files_searched += 1;
        let text = String::from_utf8_lossy(&bytes);
        let mut matched_this_file = false;

        for (idx, raw_line) in text.lines().enumerate() {
            let line_hit = if opts.case_insensitive {
                raw_line.to_lowercase().contains(&needle)
            } else {
                raw_line.contains(&needle)
            };
            if line_hit {
                if !matched_this_file {
                    matched_this_file = true;
                    report.files_matched += 1;
                }
                let trimmed = raw_line.trim_end_matches('\r');
                let display_line = if trimmed.chars().count() > 500 {
                    let mut s: String = trimmed.chars().take(500).collect();
                    s.push('…');
                    s
                } else {
                    trimmed.to_string()
                };
                report.matches.push(GrepMatch {
                    path: entry_path.to_string(),
                    line_number: idx as u64 + 1,
                    line: display_line,
                });
                if report.matches.len() >= max_matches {
                    report.truncated = true;
                    return Ok(false);
                }
            }
        }
        Ok(true)
    };

    match fmt {
        Format::Zip => {
            formats::zip::scan_files(path, &opts.password, &selector, max_file_size, &mut scan_one)?
        }
        Format::Tar
        | Format::TarGz
        | Format::TarBz2
        | Format::TarXz
        | Format::TarZst
        | Format::TarLz4 => {
            formats::tar::scan_files(path, fmt, &selector, max_file_size, &mut scan_one)?
        }
        Format::SevenZ => {
            formats::sevenz::scan_files(
                path,
                &opts.password,
                &selector,
                max_file_size,
                &mut scan_one,
            )?
        }
        _ => {
            let info = list(
                path,
                &ListOptions {
                    password: opts.password.clone(),
                },
            )?;
            let size_unknown_ok = matches!(
                fmt,
                Format::Gz | Format::Bz2 | Format::Xz | Format::Zst | Format::Lz4
            );
            for entry in &info.entries {
                if entry.is_dir
                    || (entry.size == 0 && !size_unknown_ok)
                    || entry.size > max_file_size
                    || !selector.matches(&entry.path)
                    || is_binary_extension(&entry.path)
                {
                    continue;
                }
                let content = read_entry(
                    path,
                    &entry.path,
                    &ReadOptions {
                        password: opts.password.clone(),
                        offset: 0,
                        max_bytes: max_file_size,
                    },
                )?;
                let mut cursor = std::io::Cursor::new(&content.bytes);
                if !scan_one(&entry.path, content.size, &mut cursor)? {
                    break;
                }
            }
        }
    }

    Ok(report)
}

fn is_binary_extension(entry_path: &str) -> bool {
    let lower = entry_path.to_ascii_lowercase();
    const BIN_EXTS: &[&str] = &[
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".ico", ".mp3", ".mp4", ".mkv", ".avi",
        ".mov", ".wav", ".flac", ".ogg", ".woff", ".woff2", ".ttf", ".otf", ".eot", ".pdf", ".zip",
        ".7z", ".rar", ".gz", ".bz2", ".xz", ".zst", ".lz4", ".iso", ".deb", ".rpm", ".cab",
        ".pkg", ".xar", ".o", ".a", ".so", ".dylib", ".dll", ".exe", ".class", ".pyc", ".wasm",
    ];
    BIN_EXTS.iter().any(|ext| lower.ends_with(ext))
}

/// Rename or move an entry (or directory prefix) inside an existing `.zip`,
/// `.7z`, or `.tar.*` archive atomically.
pub fn rename(
    archive: impl AsRef<Path>,
    from: &str,
    to: &str,
    opts: &RenameOptions,
    progress: Option<ProgressFn<'_>>,
) -> Result<RenameReport> {
    let archive = archive.as_ref();
    let from_clean = from.replace('\\', "/");
    let from_clean = from_clean.trim_start_matches("./").trim_matches('/');
    let to_clean = to.replace('\\', "/");
    let to_clean = to_clean.trim_start_matches("./").trim_matches('/');
    if from_clean.is_empty() || to_clean.is_empty() {
        return Err(Error::other("rename source and destination paths must not be empty"));
    }

    let fmt = detect(archive).ok_or_else(|| Error::UnsupportedFormat(Some(archive.to_path_buf())))?;
    if !fmt.can_modify() {
        return Err(Error::CreateUnsupported(fmt.label().to_string()));
    }
    let mut local = noop_progress;
    let progress: ProgressFn = progress.unwrap_or(&mut local);

    if fmt == Format::Zip {
        return formats::zip::rename(archive, from_clean, to_clean, progress);
    }

    let info = list(
        archive,
        &ListOptions {
            password: opts.password.clone(),
        },
    )?;
    let from_prefix = format!("{from_clean}/");
    let entries_renamed = info
        .entries
        .iter()
        .filter(|e| {
            let trimmed = e.path.trim_start_matches("./").trim_end_matches('/');
            trimmed == from_clean || trimmed.starts_with(&from_prefix)
        })
        .count() as u64;

    if entries_renamed == 0 {
        return Err(Error::other(format!("entry not found in archive: {from_clean}")));
    }

    let stage = std::env::temp_dir().join(format!(
        "ziplark-mv-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&stage)?;

    let result = (|| -> Result<RenameReport> {
        let mut ex_opts = ExtractOptions::new(&stage);
        ex_opts.overwrite = true;
        ex_opts.password = opts.password.clone();
        extract(archive, &ex_opts, None)?;

        let src_path = formats::safe_join(&stage, from_clean)?;
        let dst_path = formats::safe_join(&stage, to_clean)?;
        if let Some(parent) = dst_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&src_path, &dst_path)?;

        let mut remaining_inputs = Vec::new();
        for entry in std::fs::read_dir(&stage)? {
            let entry = entry?;
            remaining_inputs.push(entry.path());
        }
        remaining_inputs.sort();

        let (tmp_archive, tmp_file) = formats::zip::create_temp_for(archive, "mv.tmp")?;
        drop(tmp_file);
        let create_opts = CreateOptions {
            format: fmt,
            level: opts.level,
            password: opts.password.clone(),
            split_size: None,
        };
        create(&tmp_archive, &remaining_inputs, &create_opts, Some(progress))?;
        formats::zip::replace_archive_file(&tmp_archive, archive)?;
        let bytes_out = std::fs::metadata(archive)?.len();
        Ok(RenameReport {
            output: archive.to_path_buf(),
            format: fmt,
            from: from_clean.to_string(),
            to: to_clean.to_string(),
            entries_renamed,
            bytes_out,
        })
    })();

    let _ = std::fs::remove_dir_all(&stage);
    result
}

/// Compute cryptographic or CRC32 digests for all matching files inside an
/// archive without writing anything to disk.
pub fn hash(
    path: impl AsRef<Path>,
    opts: &HashOptions,
    progress: Option<ProgressFn<'_>>,
) -> Result<HashReport> {
    let path = path.as_ref();
    let fmt = detect(path).ok_or_else(|| Error::UnsupportedFormat(Some(path.to_path_buf())))?;
    let mut local = noop_progress;
    let progress: ProgressFn = progress.unwrap_or(&mut local);
    let selector = opts.selector();
    let algo = opts.algorithm;

    let mut entries = Vec::new();
    let mut files_hashed = 0u64;
    let mut total_bytes = 0u64;

    let mut hash_one = |entry_path: &str, _size: u64, rd: &mut dyn std::io::Read| -> Result<bool> {
        formats::report(
            progress,
            Progress {
                current_path: entry_path.to_string(),
                entries_done: files_hashed,
                entries_total: 0,
                bytes_done: total_bytes,
                bytes_total: 0,
            },
        )?;
        let (digest, n) = compute_stream_digest(rd, algo)?;
        files_hashed += 1;
        total_bytes += n;
        entries.push(EntryHash {
            path: entry_path.to_string(),
            size: n,
            digest,
        });
        Ok(true)
    };

    match fmt {
        Format::Zip => {
            formats::zip::scan_files(path, &opts.password, &selector, u64::MAX, &mut hash_one)?
        }
        Format::Tar
        | Format::TarGz
        | Format::TarBz2
        | Format::TarXz
        | Format::TarZst
        | Format::TarLz4 => {
            formats::tar::scan_files(path, fmt, &selector, u64::MAX, &mut hash_one)?
        }
        Format::SevenZ => {
            formats::sevenz::scan_files(
                path,
                &opts.password,
                &selector,
                u64::MAX,
                &mut hash_one,
            )?
        }
        _ => {
            let info = list(
                path,
                &ListOptions {
                    password: opts.password.clone(),
                },
            )?;
            for entry in &info.entries {
                if entry.is_dir || !selector.matches(&entry.path) {
                    continue;
                }
                if entry.size == 0
                    && !matches!(
                        fmt,
                        Format::Gz | Format::Bz2 | Format::Xz | Format::Zst | Format::Lz4
                    )
                {
                    let mut empty = std::io::empty();
                    hash_one(&entry.path, 0, &mut empty)?;
                    continue;
                }
                let content = read_entry(
                    path,
                    &entry.path,
                    &ReadOptions {
                        password: opts.password.clone(),
                        offset: 0,
                        max_bytes: 64 * 1024 * 1024,
                    },
                )?;
                let mut cursor = std::io::Cursor::new(&content.bytes);
                hash_one(&entry.path, content.size, &mut cursor)?;
            }
        }
    }

    // Notice: `scan_files` skips 0-byte files (created for `grep`), so also
    // include any 0-byte regular files from `list()` if we used `scan_files`.
    if matches!(
        fmt,
        Format::Zip
            | Format::SevenZ
            | Format::Tar
            | Format::TarGz
            | Format::TarBz2
            | Format::TarXz
            | Format::TarZst
            | Format::TarLz4
    ) {
        if let Ok(info) = list(
            path,
            &ListOptions {
                password: opts.password.clone(),
            },
        ) {
            let seen: HashSet<String> = entries.iter().map(|e| e.path.clone()).collect();
            for e in info.entries {
                if !e.is_dir && e.size == 0 && selector.matches(&e.path) && !seen.contains(&e.path)
                {
                    let (digest, _) = compute_stream_digest(&mut std::io::empty(), algo)?;
                    files_hashed += 1;
                    entries.push(EntryHash {
                        path: e.path,
                        size: 0,
                        digest,
                    });
                }
            }
        }
    }

    Ok(HashReport {
        path: path.to_path_buf(),
        format: fmt,
        algorithm: algo,
        entries,
        files_hashed,
        total_bytes,
    })
}

fn compute_stream_digest(
    rd: &mut dyn std::io::Read,
    algo: HashAlgorithm,
) -> Result<(String, u64)> {
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    match algo {
        HashAlgorithm::Sha256 => {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            loop {
                let n = rd.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                total += n as u64;
            }
            Ok((hex_encode(&hasher.finalize()), total))
        }
        HashAlgorithm::Sha1 => {
            use sha1::{Digest, Sha1};
            let mut hasher = Sha1::new();
            loop {
                let n = rd.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                total += n as u64;
            }
            Ok((hex_encode(&hasher.finalize()), total))
        }
        HashAlgorithm::Crc32 => {
            let mut hasher = crc32fast::Hasher::new();
            loop {
                let n = rd.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                total += n as u64;
            }
            Ok((format!("{:08x}", hasher.finalize()), total))
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

/// Compare two archives (`left` and `right`) of any readable format without
/// extracting to disk, reporting added, removed, and modified entries (plus
/// optional unified text diff patches when `opts.content` is set).
pub fn diff(
    left: impl AsRef<Path>,
    right: impl AsRef<Path>,
    opts: &DiffOptions,
    progress: Option<ProgressFn<'_>>,
) -> Result<DiffReport> {
    use std::collections::BTreeMap;
    let left = left.as_ref();
    let right = right.as_ref();
    let mut local = noop_progress;
    let progress: ProgressFn = progress.unwrap_or(&mut local);

    let left_info = list(
        left,
        &ListOptions {
            password: opts.left_password.clone(),
        },
    )?;
    let right_info = list(
        right,
        &ListOptions {
            password: opts.right_password.clone(),
        },
    )?;
    let selector = opts.selector();

    let mut left_map: BTreeMap<String, &ArchiveEntry> = BTreeMap::new();
    for e in &left_info.entries {
        if !e.is_dir && selector.matches(&e.path) {
            left_map.insert(e.path.trim_start_matches("./").to_string(), e);
        }
    }
    let mut right_map: BTreeMap<String, &ArchiveEntry> = BTreeMap::new();
    for e in &right_info.entries {
        if !e.is_dir && selector.matches(&e.path) {
            right_map.insert(e.path.trim_start_matches("./").to_string(), e);
        }
    }

    let mut all_paths: Vec<String> = left_map
        .keys()
        .chain(right_map.keys())
        .cloned()
        .collect();
    all_paths.sort();
    all_paths.dedup();

    let mut added = 0u64;
    let mut removed = 0u64;
    let mut modified = 0u64;
    let mut unchanged = 0u64;
    let mut changes = Vec::new();
    let total_paths = all_paths.len() as u64;

    for (idx, p) in all_paths.iter().enumerate() {
        formats::report(
            progress,
            Progress {
                current_path: p.clone(),
                entries_done: idx as u64 + 1,
                entries_total: total_paths,
                bytes_done: 0,
                bytes_total: 0,
            },
        )?;

        match (left_map.get(p), right_map.get(p)) {
            (None, Some(r)) => {
                added += 1;
                changes.push(DiffEntry {
                    path: p.clone(),
                    status: DiffStatus::Added,
                    is_dir: r.is_dir,
                    old_size: None,
                    new_size: Some(r.size),
                    old_crc32: None,
                    new_crc32: r.crc32,
                    patch: None,
                });
            }
            (Some(l), None) => {
                removed += 1;
                changes.push(DiffEntry {
                    path: p.clone(),
                    status: DiffStatus::Removed,
                    is_dir: l.is_dir,
                    old_size: Some(l.size),
                    new_size: None,
                    old_crc32: l.crc32,
                    new_crc32: None,
                    patch: None,
                });
            }
            (Some(l), Some(r)) => {
                // Determine whether `l` and `r` differ.
                let mut is_mod = l.size != r.size;
                let mut left_bytes_cache: Option<Vec<u8>> = None;
                let mut right_bytes_cache: Option<Vec<u8>> = None;

                if !is_mod {
                    if let (Some(lc), Some(rc)) = (l.crc32, r.crc32) {
                        if lc != 0 || rc != 0 || l.size == 0 {
                            is_mod = lc != rc;
                        }
                    } else if l.size > 0 && l.size <= 16 * 1024 * 1024 {
                        // Formats like tar don't store per-entry CRC32 in headers;
                        // compare bytes directly when sizes match.
                        let lb = read_entry(
                            left,
                            &l.path,
                            &ReadOptions {
                                password: opts.left_password.clone(),
                                offset: 0,
                                max_bytes: 16 * 1024 * 1024,
                            },
                        )
                        .ok()
                        .map(|c| c.bytes);
                        let rb = read_entry(
                            right,
                            &r.path,
                            &ReadOptions {
                                password: opts.right_password.clone(),
                                offset: 0,
                                max_bytes: 16 * 1024 * 1024,
                            },
                        )
                        .ok()
                        .map(|c| c.bytes);
                        if let (Some(lb), Some(rb)) = (lb, rb) {
                            is_mod = lb != rb;
                            left_bytes_cache = Some(lb);
                            right_bytes_cache = Some(rb);
                        }
                    }
                }

                if is_mod {
                    modified += 1;
                    let patch = if opts.content
                        && !is_binary_extension(p)
                        && l.size <= 4 * 1024 * 1024
                        && r.size <= 4 * 1024 * 1024
                    {
                        let lb = left_bytes_cache.or_else(|| {
                            read_entry(
                                left,
                                &l.path,
                                &ReadOptions {
                                    password: opts.left_password.clone(),
                                    offset: 0,
                                    max_bytes: 4 * 1024 * 1024,
                                },
                            )
                            .ok()
                            .map(|c| c.bytes)
                        });
                        let rb = right_bytes_cache.or_else(|| {
                            read_entry(
                                right,
                                &r.path,
                                &ReadOptions {
                                    password: opts.right_password.clone(),
                                    offset: 0,
                                    max_bytes: 4 * 1024 * 1024,
                                },
                            )
                            .ok()
                            .map(|c| c.bytes)
                        });
                        match (lb, rb) {
                            (Some(lb), Some(rb)) => unified_text_diff(p, &lb, &rb),
                            _ => None,
                        }
                    } else {
                        None
                    };

                    changes.push(DiffEntry {
                        path: p.clone(),
                        status: DiffStatus::Modified,
                        is_dir: false,
                        old_size: Some(l.size),
                        new_size: Some(r.size),
                        old_crc32: l.crc32,
                        new_crc32: r.crc32,
                        patch,
                    });
                } else {
                    unchanged += 1;
                }
            }
            (None, None) => {}
        }
    }

    Ok(DiffReport {
        left: left.to_path_buf(),
        left_format: left_info.format,
        right: right.to_path_buf(),
        right_format: right_info.format,
        added,
        removed,
        modified,
        unchanged,
        changes,
    })
}

/// Produce a compact unified diff for two text buffers (skipping binary blobs).
fn unified_text_diff(path: &str, old_bytes: &[u8], new_bytes: &[u8]) -> Option<String> {
    if old_bytes[..old_bytes.len().min(8192)].contains(&0)
        || new_bytes[..new_bytes.len().min(8192)].contains(&0)
    {
        return None;
    }
    let old_str = String::from_utf8_lossy(old_bytes);
    let new_str = String::from_utf8_lossy(new_bytes);
    let old_lines: Vec<&str> = old_str.lines().collect();
    let new_lines: Vec<&str> = new_str.lines().collect();
    if old_lines == new_lines {
        return None;
    }

    // Cap LCS matrix size so huge files fall back to a fast prefix/suffix diff.
    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    const MAX_DIFF_LINES: usize = 2000;
    if old_lines.len() + new_lines.len() > MAX_DIFF_LINES {
        let mut p = 0;
        while p < old_lines.len() && p < new_lines.len() && old_lines[p] == new_lines[p] {
            p += 1;
        }
        let mut s = 0;
        while s < (old_lines.len() - p)
            && s < (new_lines.len() - p)
            && old_lines[old_lines.len() - 1 - s] == new_lines[new_lines.len() - 1 - s]
        {
            s += 1;
        }
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            p + 1,
            old_lines.len() - p - s,
            p + 1,
            new_lines.len() - p - s
        ));
        for &l in &old_lines[p..old_lines.len() - s] {
            out.push('-');
            out.push_str(l);
            out.push('\n');
        }
        for &l in &new_lines[p..new_lines.len() - s] {
            out.push('+');
            out.push_str(l);
            out.push('\n');
        }
        return Some(out);
    }

    // Trim common prefix and suffix before LCS.
    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < (old_lines.len() - prefix)
        && suffix < (new_lines.len() - prefix)
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let a = &old_lines[prefix..old_lines.len() - suffix];
    let b = &new_lines[prefix..new_lines.len() - suffix];
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let ctx_before = prefix.saturating_sub(2);
    let ctx_after_old = (old_lines.len() - suffix + 2).min(old_lines.len());
    let ctx_after_new = (new_lines.len() - suffix + 2).min(new_lines.len());
    out.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        ctx_before + 1,
        ctx_after_old - ctx_before,
        ctx_before + 1,
        ctx_after_new - ctx_before
    ));
    for &l in &old_lines[ctx_before..prefix] {
        out.push(' ');
        out.push_str(l);
        out.push('\n');
    }
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            out.push(' ');
            out.push_str(a[i]);
            out.push('\n');
            i += 1;
            j += 1;
        } else if j < m && (i == n || dp[i][j + 1] >= dp[i + 1][j]) {
            out.push('+');
            out.push_str(b[j]);
            out.push('\n');
            j += 1;
        } else {
            out.push('-');
            out.push_str(a[i]);
            out.push('\n');
            i += 1;
        }
    }
    for &l in &old_lines[old_lines.len() - suffix..ctx_after_old] {
        out.push(' ');
        out.push_str(l);
        out.push('\n');
    }
    Some(out)
}

