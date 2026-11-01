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
    ArchiveAttributes, ArchiveEntry, ArchiveInfo, CreateReport, EntryContent, ExtractReport,
    Format, Progress, TestReport,
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
}

impl CreateOptions {
    pub fn new(format: Format) -> Self {
        Self {
            format,
            level: Level::Default,
            password: None,
        }
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
    let mut local = noop_progress;
    let progress: ProgressFn = progress.unwrap_or(&mut local);
    match opts.format {
        Format::Zip => formats::zip::create(output, inputs, opts, progress),
        Format::SevenZ => formats::sevenz::create(output, inputs, opts, progress),
        Format::Tar
        | Format::TarGz
        | Format::TarBz2
        | Format::TarXz
        | Format::TarZst
        | Format::TarLz4 => formats::tar::create(output, inputs, opts, progress),
        Format::Gz | Format::Bz2 | Format::Xz | Format::Zst | Format::Lz4 => {
            formats::stream::create(output, inputs, opts, progress)
        }
        other => Err(Error::CreateUnsupported(other.label().into())),
    }
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

    // For 7z and tar variants: unpack existing entries into a clean sibling
    // staging directory, overlay `inputs`, and re-pack atomically.
    let parent = archive.parent().unwrap_or(Path::new("."));
    let stage = parent.join(format!(
        ".ziplark-add-{}-{}",
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

        let tmp_archive = archive.with_extension(format!("tmp.{}", std::process::id()));
        let create_opts = CreateOptions {
            format: fmt,
            level: opts.level,
            password: opts.password.clone(),
        };
        let mut rep = create(&tmp_archive, &merged_inputs, &create_opts, Some(progress))?;
        std::fs::rename(&tmp_archive, archive)?;
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
    }
}
