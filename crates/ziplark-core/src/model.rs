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
    Lz4,
    TarLz4,
    /// ISO 9660 / Joliet disc image (extract-only).
    Iso,
}

impl Format {
    /// Whether the engine can create (write) this format.
    pub fn can_create(self) -> bool {
        !matches!(self, Format::Rar | Format::Iso)
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
            Format::Lz4 => "LZ4",
            Format::TarLz4 => "TAR.LZ4",
            Format::Iso => "ISO",
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
            Format::Lz4 => "lz4",
            Format::TarLz4 => "tar.lz4",
            Format::Iso => "iso",
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
    /// The entry's data is split across volumes of a multi-volume archive, so
    /// reading it needs more than the volume it starts in.
    #[serde(default, skip_serializing_if = "is_false")]
    pub split: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Properties of the archive as a whole, as opposed to its entries. Formats
/// that cannot express one of these leave it `false`.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ArchiveAttributes {
    /// Entries share one compression stream. Reading the last file means
    /// decompressing everything before it, which is why picking one file out
    /// of a solid archive is slow.
    pub solid: bool,
    /// Carries a recovery record: enough redundancy to repair damage.
    pub recovery_record: bool,
    /// The entry *names* are encrypted too, so even listing needs the password.
    pub encrypted_headers: bool,
    /// Locked against modification by the tool that wrote it (RAR `-k`).
    pub locked: bool,
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
    /// Every file that makes up this archive, in order, when it is split into
    /// volumes (`part1.rar`, `part2.rar`, …). Empty for a single-file archive.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub volumes: Vec<PathBuf>,
    /// The volume that should come next but is not on disk. `Some` means the
    /// set is incomplete and the tail of the archive cannot be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_volume: Option<PathBuf>,
    /// The archive's comment, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub attributes: ArchiveAttributes,
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
    /// Entries that could not be extracted, each as `path: reason`. Only ever
    /// non-empty when the caller asked to keep going past failures
    /// (`ExtractOptions::keep_broken`); otherwise the first failure is an error.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<String>,
    /// Entries that were written but are **incomplete** — as much of the file
    /// as the archive actually contained. Salvaging leaves these on disk on
    /// purpose; they are listed so nobody mistakes one for the whole file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub partial: Vec<String>,
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
