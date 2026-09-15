use crate::error::{Error, Result};
use crate::formats::{
    collect_inputs, copy_watched, create_file, create_symlink, ensure_parent, prepare_dir,
    prepare_leaf, report, DestGuard, InputKind,
};
use crate::model::*;
use crate::{CreateOptions, ExtractOptions, Level, ListOptions, ProgressFn};
use sevenz_rust2::encoder_options::{AesEncoderOptions, Lzma2Options};
use sevenz_rust2::{
    Archive, ArchiveEntry as SevenZEntry, ArchiveReader, ArchiveWriter, EncoderConfiguration,
    EncoderMethod, Password, SourceReader,
};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// How much *input* goes into one solid block.
///
/// 7z compresses a block as a single LZMA2 stream, so entries in the same block
/// share a dictionary — which is where nearly all of the compression on a tree
/// of small, similar files comes from. Writing one block per entry (what we did
/// before) throws that away and pays a fresh encoder setup per file.
///
/// The cap exists because a block is also the unit of *decompression*: pulling
/// one file out of a block means decoding everything before it. 256 MiB keeps
/// single-file extraction from a large archive bounded while still giving the
/// dictionary plenty to work with.
const SOLID_BLOCK_BYTES: u64 = 256 * 1024 * 1024;

/// Input per independently-compressed chunk when encoding with several threads.
///
/// The multi-threaded LZMA2 encoder splits the stream into chunks and resets
/// the dictionary at each boundary, so this trades a little ratio for cores.
/// It is clamped up to the dictionary size by the encoder.
const MT_CHUNK_BYTES: u64 = 32 * 1024 * 1024;

/// 7z carries unix permissions in the Windows attribute word: bit 15 says
/// "unix mode in the high half", which is how p7zip and 7-Zip store a mode —
/// and how they mark a symlink.
const ATTR_UNIX_EXTENSION: u32 = 0x8000;
const S_IFMT: u32 = 0o170000;
const S_IFLNK: u32 = 0o120000;

fn map_err(e: sevenz_rust2::Error) -> Error {
    use sevenz_rust2::Error as E;
    match e {
        E::PasswordRequired => Error::PasswordRequired,
        E::MaybeBadPassword(_) => Error::BadPassword,
        E::ChecksumVerificationFailed | E::NextHeaderCrcMismatch => {
            Error::corrupt("7z checksum mismatch")
        }
        E::Io(io, _) | E::FileOpen(io, _) => Error::Io(io),
        other => Error::other(other.to_string()),
    }
}

fn password(opts_pw: &Option<String>) -> Password {
    match opts_pw {
        Some(p) => Password::from(p.as_str()),
        None => Password::empty(),
    }
}

/// Whether any block in the archive is AES-encrypted.
///
/// This reads the archive's own coder chain. The old code reported "encrypted"
/// when the *caller* had supplied a password, which answered a different
/// question entirely.
fn is_encrypted(archive: &Archive) -> bool {
    archive.blocks.iter().any(|block| {
        block
            .coders
            .iter()
            .any(|c| c.encoder_method_id() == EncoderMethod::AES256_SHA256.id())
    })
}

/// The unix mode an entry carries, if it was written by a tool that stores one.
fn unix_mode(entry: &SevenZEntry) -> Option<u32> {
    if !entry.has_windows_attributes {
        return None;
    }
    let attrs = entry.windows_attributes;
    (attrs & ATTR_UNIX_EXTENSION != 0).then_some(attrs >> 16)
}

/// Restore what the archive recorded. Best effort — a mode we cannot apply is
/// not a reason to fail the extraction.
fn restore_metadata(path: &Path, mode: Option<u32>, mtime: Option<i64>) {
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777));
    }
    #[cfg(not(unix))]
    let _ = mode;

    if let Some(secs) = mtime {
        let t = filetime::FileTime::from_unix_time(secs, 0);
        let _ = filetime::set_file_times(path, t, t);
    }
}

/// 7z stores Windows FILETIME; the model wants unix seconds.
fn modified(entry: &SevenZEntry) -> Option<i64> {
    if !entry.has_last_modified_date {
        return None;
    }
    let t: std::time::SystemTime = entry.last_modified_date.into();
    t.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

pub fn list(path: &Path, fmt: Format, opts: &ListOptions) -> Result<ArchiveInfo> {
    let reader = ArchiveReader::open(path, password(&opts.password)).map_err(map_err)?;
    let archive = reader.archive();
    let encrypted = is_encrypted(archive);
    let mut entries = Vec::with_capacity(archive.files.len());
    let mut total_size = 0u64;
    for f in &archive.files {
        total_size += f.size();
        entries.push(ArchiveEntry {
            path: f.name().to_string(),
            is_dir: f.is_directory(),
            size: f.size(),
            compressed_size: (f.compressed_size > 0).then_some(f.compressed_size),
            encrypted,
            modified: modified(f),
            crc32: f.has_crc.then_some(f.crc as u32),
            split: false,
        });
    }
    let total_compressed = std::fs::metadata(path)?.len();
    Ok(ArchiveInfo {
        format: fmt,
        path: path.to_path_buf(),
        entries,
        encrypted,
        total_size,
        total_compressed,
        volumes: Vec::new(),
        missing_volume: None,
        comment: None,
        attributes: ArchiveAttributes {
            // 7z records this, and it is worth surfacing: pulling one file out
            // of a solid archive means decompressing everything before it.
            solid: archive.is_solid,
            ..ArchiveAttributes::default()
        },
    })
}

pub fn extract(path: &Path, opts: &ExtractOptions, progress: ProgressFn) -> Result<ExtractReport> {
    let mut reader = ArchiveReader::open(path, password(&opts.password)).map_err(map_err)?;
    std::fs::create_dir_all(&opts.dest)?;
    let selector = opts.selector();

    let mut report = ExtractReport {
        files_written: 0,
        dirs_created: 0,
        bytes_written: 0,
        dest: opts.dest.clone(),
        failed: Vec::new(),
        partial: Vec::new(),
    };
    let mut first_error: Option<Error> = None;
    let mut idx = 0u64;
    let mut guard = DestGuard::new(&opts.dest);

    reader
        .for_each_entries(|entry, rd| {
            idx += 1;
            let name = entry.name().to_string();
            if !selector.matches(&name) {
                return Ok(true);
            }
            // Funnel through the shared extraction guard.
            let out_path = match guard.join(&name) {
                Ok(p) => p,
                Err(e) => {
                    first_error = Some(e);
                    return Ok(false);
                }
            };
            if entry.is_directory() {
                if let Err(e) = prepare_dir(&out_path, &name) {
                    first_error = Some(e);
                    return Ok(false);
                }
                report.dirs_created += 1;
                return Ok(true);
            }
            if let Err(e) = ensure_parent(&out_path) {
                first_error = Some(e);
                return Ok(false);
            }

            let mode = unix_mode(entry);
            let mtime = modified(entry);

            // A symlink's content is its target path; the mode is what says so.
            if mode.is_some_and(|m| m & S_IFMT == S_IFLNK) {
                let mut target = String::new();
                if let Err(e) = rd.read_to_string(&mut target) {
                    first_error = Some(Error::Io(e));
                    return Ok(false);
                }
                let made = prepare_leaf(&out_path, opts.overwrite).and_then(|()| {
                    if opts.overwrite {
                        let _ = std::fs::remove_file(&out_path);
                    }
                    create_symlink(&target, &out_path)
                });
                if let Err(e) = made {
                    first_error = Some(e);
                    return Ok(false);
                }
                guard.forget(&out_path);
                report.files_written += 1;
                return Ok(true);
            }

            let mut out = match create_file(&out_path, opts.overwrite) {
                Ok(f) => f,
                Err(e) => {
                    first_error = Some(e);
                    return Ok(false);
                }
            };
            let done_before = report.bytes_written;
            let copied = copy_watched(rd, &mut out, |so_far| {
                progress(Progress {
                    current_path: name.clone(),
                    entries_done: idx,
                    entries_total: 0,
                    bytes_done: done_before + so_far,
                    bytes_total: 0,
                })
            });
            match copied {
                Ok(n) => {
                    drop(out);
                    restore_metadata(&out_path, mode, mtime);
                    report.files_written += 1;
                    report.bytes_written += n;
                }
                Err(e) => {
                    first_error = Some(e);
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .map_err(map_err)?;

    if let Some(e) = first_error {
        return Err(e);
    }
    Ok(report)
}

pub fn test(path: &Path, opts: &ListOptions, progress: ProgressFn) -> Result<TestReport> {
    let mut reader = ArchiveReader::open(path, password(&opts.password)).map_err(map_err)?;
    let mut tested = 0u64;
    let mut bad = Vec::new();
    let mut cancelled = false;
    reader
        .for_each_entries(|entry, rd| {
            if entry.is_directory() {
                return Ok(true);
            }
            tested += 1;
            let name = entry.name().to_string();
            let mut sink = io::sink();
            if let Err(e) = copy_watched(rd, &mut sink, |so_far| {
                progress(Progress {
                    current_path: name.clone(),
                    entries_done: tested,
                    entries_total: 0,
                    bytes_done: so_far,
                    bytes_total: 0,
                })
            }) {
                if matches!(e, Error::Cancelled) {
                    cancelled = true;
                    return Ok(false);
                }
                bad.push(format!("{name}: {e}"));
            }
            Ok(true)
        })
        .map_err(map_err)?;
    if cancelled {
        return Err(Error::Cancelled);
    }
    Ok(TestReport {
        ok: bad.is_empty(),
        entries_tested: tested,
        bad_entries: bad,
    })
}

/// The coder chain for new archives: optional AES on the outside, then LZMA2
/// (or plain COPY at `Level::Store`).
fn content_methods(opts: &CreateOptions) -> Vec<EncoderConfiguration> {
    let mut methods: Vec<EncoderConfiguration> = Vec::new();
    if let Some(pw) = &opts.password {
        methods.push(AesEncoderOptions::new(Password::from(pw.as_str())).into());
    }
    methods.push(match opts.level {
        Level::Store => EncoderMethod::COPY.into(),
        level => lzma2_options(level).into(),
    });
    methods
}

fn lzma2_options(level: Level) -> Lzma2Options {
    let preset = match level {
        Level::Store => 0,
        Level::Fast => 1,
        Level::Default => 6,
        Level::Best => 9,
    };
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    if threads > 1 {
        Lzma2Options::from_level_mt(preset, threads, MT_CHUNK_BYTES)
    } else {
        Lzma2Options::from_level(preset)
    }
}

pub fn create(
    output: &Path,
    inputs: &[PathBuf],
    opts: &CreateOptions,
    progress: ProgressFn,
) -> Result<CreateReport> {
    let files = collect_inputs(inputs)?;
    let out = File::create(output)?;
    let mut writer = ArchiveWriter::new(out).map_err(map_err)?;
    writer.set_content_methods(content_methods(opts));

    let total = files.len() as u64;
    let mut entries_added = 0u64;
    let mut bytes_in = 0u64;
    // Files accumulate here until they add up to a solid block.
    let mut block: Vec<(SevenZEntry, EntrySource)> = Vec::new();
    let mut block_bytes = 0u64;

    for (idx, input) in files.iter().enumerate() {
        let done = idx as u64 + 1;
        let (src, rel) = (&input.path, &input.rel);

        if input.kind == InputKind::EmptyDir {
            let entry = SevenZEntry::new_directory(rel.trim_end_matches('/'));
            writer
                .push_archive_entry::<&[u8]>(entry, None)
                .map_err(map_err)?;
            continue;
        }

        let (mut entry, source, size) = if input.kind == InputKind::Symlink {
            // Same representation 7-Zip and p7zip use: the target path as the
            // entry's content, with the link marked in the unix mode.
            let target = std::fs::read_link(src)?.to_string_lossy().into_owned();
            let size = target.len() as u64;
            let mut entry = SevenZEntry::from_path(src, rel.clone());
            entry.has_stream = true;
            entry.is_directory = false;
            entry.has_windows_attributes = true;
            entry.windows_attributes = ATTR_UNIX_EXTENSION | ((S_IFLNK | 0o777) << 16);
            (entry, EntrySource::Bytes(io::Cursor::new(target.into_bytes())), size)
        } else {
            // `from_path` fills in the name and timestamps but leaves `size` to
            // the writer, so ask the filesystem — block accounting needs it.
            let meta = std::fs::metadata(src)?;
            let mut entry = SevenZEntry::from_path(src, rel.clone());
            set_unix_mode(&mut entry, &meta);
            (entry, EntrySource::File(LazyFile::new(src.clone())), meta.len())
        };
        entry.size = size;

        block.push((entry, source));
        block_bytes += size;
        bytes_in += size;
        entries_added += 1;

        report(
            progress,
            Progress {
                current_path: rel.clone(),
                entries_done: done,
                entries_total: total,
                bytes_done: bytes_in,
                bytes_total: 0,
            },
        )?;

        if block_bytes >= SOLID_BLOCK_BYTES {
            write_block(&mut writer, &mut block)?;
            block_bytes = 0;
        }
    }
    write_block(&mut writer, &mut block)?;

    let mut finished = writer.finish().map_err(Error::Io)?;
    finished.flush()?;
    let bytes_out = std::fs::metadata(output)?.len();
    Ok(CreateReport {
        output: output.to_path_buf(),
        format: Format::SevenZ,
        entries_added,
        bytes_in,
        bytes_out,
    })
}

/// Compress everything accumulated so far as one solid block.
fn write_block<W: Write + std::io::Seek>(
    writer: &mut ArchiveWriter<W>,
    block: &mut Vec<(SevenZEntry, EntrySource)>,
) -> Result<()> {
    if block.is_empty() {
        return Ok(());
    }
    let (entries, sources): (Vec<_>, Vec<_>) = block.drain(..).unzip();
    let readers: Vec<SourceReader<EntrySource>> =
        sources.into_iter().map(SourceReader::from).collect();
    writer
        .push_archive_entries(entries, readers)
        .map_err(map_err)?;
    Ok(())
}

/// Record this file's permissions the way 7-Zip does, so the executable bit
/// survives a round trip.
#[cfg(unix)]
fn set_unix_mode(entry: &mut SevenZEntry, meta: &std::fs::Metadata) {
    use std::os::unix::fs::PermissionsExt;
    entry.has_windows_attributes = true;
    entry.windows_attributes = ATTR_UNIX_EXTENSION | ((meta.permissions().mode() & 0o7777) << 16);
}

#[cfg(not(unix))]
fn set_unix_mode(_entry: &mut SevenZEntry, _meta: &std::fs::Metadata) {}

/// What an entry's bytes come from: a file on disk, or a symlink target held in
/// memory.
enum EntrySource {
    File(LazyFile),
    Bytes(io::Cursor<Vec<u8>>),
}

impl Read for EntrySource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            EntrySource::File(f) => f.read(buf),
            EntrySource::Bytes(c) => c.read(buf),
        }
    }
}

/// A file that opens on first read and closes itself at EOF.
///
/// The writer takes one reader per entry for the whole block up front, and a
/// block can hold many thousands of small files — handing it that many open
/// files at once would blow through the process's descriptor limit. Entries are
/// read strictly in order, so at most one file is ever actually open.
struct LazyFile {
    path: PathBuf,
    file: Option<File>,
    finished: bool,
}

impl LazyFile {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            file: None,
            finished: false,
        }
    }
}

impl Read for LazyFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.finished {
            return Ok(0);
        }
        if self.file.is_none() {
            self.file = Some(File::open(&self.path)?);
        }
        let n = self.file.as_mut().unwrap().read(buf)?;
        if n == 0 {
            self.file = None;
            self.finished = true;
        }
        Ok(n)
    }
}

