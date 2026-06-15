use serde::Serialize;
use std::path::PathBuf;

/// Archive formats the engine understands. `can_create()` reports whether we
/// can *write* the format (some, like RAR, are extract-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Zip,
    SevenZ,
    Rar,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    Gz,
    Bz2,
    Xz,
    Zst,
}

impl Format {
    /// Whether the engine can create (write) this format.
    pub fn can_create(self) -> bool {
        !matches!(self, Format::Rar)
    }

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Format::Zip => "ZIP",
            Format::SevenZ => "7z",
            Format::Rar => "RAR",
            Format::Tar => "TAR",
            Format::TarGz => "TAR.GZ",
            Format::TarBz2 => "TAR.BZ2",
            Format::TarXz => "TAR.XZ",
            Format::TarZst => "TAR.ZST",
            Format::Gz => "GZIP",
            Format::Bz2 => "BZIP2",
            Format::Xz => "XZ",
            Format::Zst => "ZSTD",
        }
    }

    /// Canonical extension (without leading dot) used when creating.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::SevenZ => "7z",
            Format::Rar => "rar",
            Format::Tar => "tar",
            Format::TarGz => "tar.gz",
            Format::TarBz2 => "tar.bz2",
            Format::TarXz => "tar.xz",
            Format::TarZst => "tar.zst",
            Format::Gz => "gz",
            Format::Bz2 => "bz2",
            Format::Xz => "xz",
            Format::Zst => "zst",
        }
    }
}

/// One entry (file or directory) inside an archive.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveEntry {
    /// Path of the entry as stored in the archive (forward slashes).
    pub path: String,
    pub is_dir: bool,
    /// Uncompressed size in bytes.
    pub size: u64,
    /// Compressed size in bytes if known.
    pub compressed_size: Option<u64>,
    /// Whether this individual entry is encrypted.
    pub encrypted: bool,
    /// Last-modified time as a unix timestamp (seconds) if known.
    pub modified: Option<i64>,
    /// CRC32 if the format records one.
    pub crc32: Option<u32>,
}

/// Summary of an archive's contents.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveInfo {
    pub format: Format,
    pub path: PathBuf,
    pub entries: Vec<ArchiveEntry>,
    /// True if any entry (or the header) is encrypted.
    pub encrypted: bool,
    pub total_size: u64,
    pub total_compressed: u64,
}

/// Per-entry progress during extract/create/test.
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub current_path: String,
    pub entries_done: u64,
    pub entries_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// Result of an extract operation.
#[derive(Debug, Clone, Serialize)]
pub struct ExtractReport {
    pub files_written: u64,
    pub dirs_created: u64,
    pub bytes_written: u64,
    pub dest: PathBuf,
}

/// Result of a create operation.
#[derive(Debug, Clone, Serialize)]
pub struct CreateReport {
    pub output: PathBuf,
    pub format: Format,
    pub entries_added: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// Result of an integrity test.
#[derive(Debug, Clone, Serialize)]
pub struct TestReport {
    pub ok: bool,
    pub entries_tested: u64,
    pub bad_entries: Vec<String>,
}
