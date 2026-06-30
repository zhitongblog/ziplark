//! # ziplark-core
//!
//! The Ziplark archive engine. One implementation of every archive operation,
//! shared verbatim by the GUI, the CLI (`ziplark`) and the MCP server. Nothing
//! in this crate knows about a UI — it takes paths and options and does work.
//!
//! Supported formats:
//!
//! | Format        | List | Extract | Create |
//! |---------------|------|---------|--------|
//! | ZIP (+AES256) | ✅   | ✅      | ✅     |
//! | 7z            | ✅   | ✅      | ✅     |
//! | RAR / RAR5    | ✅   | ✅      | —      |
//! | TAR (+gz/bz2/xz/zst/lz4) | ✅ | ✅ | ✅ |
//! | gz/bz2/xz/zst/lz4 (single stream) | ✅ | ✅ | ✅ |

mod detect;
mod error;
mod formats;
mod model;

pub use detect::detect;
pub use error::{Error, Result};
pub use model::{
    ArchiveEntry, ArchiveInfo, CreateReport, ExtractReport, Format, Progress, TestReport,
};

use std::path::{Path, PathBuf};

/// Compression effort, mapped per-format to the nearest supported level.
#[derive(Debug, Clone, Copy)]
pub enum Level {
    Store,
    Fast,
    Default,
    Best,
}

impl Default for Level {
    fn default() -> Self {
        Level::Default
    }
}

/// Options for listing an archive.
#[derive(Debug, Default, Clone)]
pub struct ListOptions {
    pub password: Option<String>,
}

/// Options for extracting an archive.
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    pub password: Option<String>,
    /// Directory to extract into (created if missing).
    pub dest: PathBuf,
    /// Overwrite existing files instead of erroring.
    pub overwrite: bool,
    /// Only extract entries whose path contains one of these substrings
    /// (empty = everything).
    pub include: Vec<String>,
}

impl ExtractOptions {
    pub fn new(dest: impl Into<PathBuf>) -> Self {
        Self {
            password: None,
            dest: dest.into(),
            overwrite: false,
            include: Vec::new(),
        }
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

/// A progress callback. The engine calls it as work proceeds; return value is
/// ignored. Use `|_| {}` if you don't care.
pub type ProgressFn<'a> = &'a mut dyn FnMut(Progress);

fn noop_progress(_: Progress) {}

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
        | Format::TarLz4 => {
            formats::tar::list(path, fmt, opts)
        }
        Format::Gz | Format::Bz2 | Format::Xz | Format::Zst | Format::Lz4 => {
            formats::stream::list(path, fmt, opts)
        }
    }
}

/// Extract an archive to `opts.dest`.
pub fn extract(
    path: impl AsRef<Path>,
    opts: &ExtractOptions,
    progress: Option<ProgressFn<'_>>,
) -> Result<ExtractReport> {
    let path = path.as_ref();
    let fmt = detect(path).ok_or_else(|| Error::UnsupportedFormat(Some(path.to_path_buf())))?;
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
        | Format::TarLz4 => {
            formats::tar::extract(path, fmt, opts, progress)
        }
        Format::Gz | Format::Bz2 | Format::Xz | Format::Zst | Format::Lz4 => {
            formats::stream::extract(path, fmt, opts, progress)
        }
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
        | Format::TarLz4 => {
            formats::tar::create(output, inputs, opts, progress)
        }
        Format::Gz | Format::Bz2 | Format::Xz | Format::Zst | Format::Lz4 => {
            formats::stream::create(output, inputs, opts, progress)
        }
        Format::Rar => Err(Error::CreateUnsupported("RAR".into())),
    }
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
        | Format::TarLz4 => {
            formats::tar::test(path, fmt, opts, progress)
        }
        Format::Gz | Format::Bz2 | Format::Xz | Format::Zst | Format::Lz4 => {
            formats::stream::test(path, fmt, opts, progress)
        }
    }
}
