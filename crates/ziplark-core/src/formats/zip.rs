use crate::encoding::NameDecoder;
use crate::error::{Error, Result};
use crate::formats::{
    collect_inputs, create_file, create_symlink, ensure_parent, DestGuard, InputKind,
};
use crate::model::*;
use crate::{CreateOptions, ExtractOptions, Level, ListOptions, ProgressFn};
use std::fs::File;
use std::io::{self, Write};
use std::io::{Read, Seek};
use std::path::Path;
use zip::result::ZipError;
use zip::write::{ExtendedFileOptions, FileOptions};
use zip::{AesMode, CompressionMethod, ZipArchive, ZipWriter};

/// Unix mode bits: the file-type field, and the value meaning "symlink".
const S_IFMT: u32 = 0o170000;
const S_IFLNK: u32 = 0o120000;
/// ZIP's 32-bit size fields stop here; anything at or past it needs Zip64.
const ZIP64_THRESHOLD: u64 = u32::MAX as u64;
/// Header ID of the "extended timestamp" field (see libzip's extrafld.txt).
const EXTENDED_TIMESTAMP_ID: u16 = 0x5455;

fn map_zip_err(e: ZipError) -> Error {
    match e {
        ZipError::Io(e) => Error::Io(e),
        ZipError::InvalidPassword => Error::BadPassword,
        ZipError::UnsupportedArchive(msg) if msg.contains("Password") => Error::PasswordRequired,
        ZipError::UnsupportedArchive(msg) => Error::other(msg.to_string()),
        ZipError::InvalidArchive(msg) => Error::corrupt(msg.to_string()),
        other => Error::corrupt(other.to_string()),
    }
}

fn ts(dt: Option<zip::DateTime>) -> Option<i64> {
    dt.and_then(|d| d.try_into().ok())
        .map(|t: time::OffsetDateTime| t.unix_timestamp())
}

/// Decode every entry name in the archive.
///
/// ZIP only promises UTF-8 when general-purpose bit 11 is set; otherwise the
/// name is raw bytes in the creating machine's code page, and the `zip` crate's
/// `name()` reads those as CP437 — which is how `中文文件.txt` becomes
/// `ÖÐÎÄÎÄ¼þ.txt`. We decode `name_raw()` ourselves instead.
///
/// The whole central directory is already in memory once the archive is open,
/// so every name is sampled before any is decoded: more bytes make the encoding
/// guess markedly better than judging one short filename at a time.
fn decode_names<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<Vec<String>> {
    let mut raw = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        raw.push(archive.by_index_raw(i).map_err(map_zip_err)?.name_raw().to_vec());
    }
    let mut decoder = NameDecoder::new();
    for name in &raw {
        decoder.sample(name);
    }
    Ok(raw.iter().map(|n| decoder.decode(n)).collect())
}

pub fn list(path: &Path, fmt: Format, _opts: &ListOptions) -> Result<ArchiveInfo> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file).map_err(map_zip_err)?;
    let names = decode_names(&mut archive)?;
    let mut entries = Vec::with_capacity(archive.len());
    let mut total_size = 0u64;
    let mut total_compressed = 0u64;
    let mut any_encrypted = false;

    for (i, name) in names.iter().enumerate() {
        // by_index_raw exposes metadata without needing the password.
        let e = archive.by_index_raw(i).map_err(map_zip_err)?;
        let encrypted = e.encrypted();
        any_encrypted |= encrypted;
        total_size += e.size();
        total_compressed += e.compressed_size();
        entries.push(ArchiveEntry {
            path: name.clone(),
            is_dir: e.is_dir(),
            size: e.size(),
            compressed_size: Some(e.compressed_size()),
            encrypted,
            modified: ts(e.last_modified()),
            crc32: Some(e.crc32()),
        });
    }

    Ok(ArchiveInfo {
        format: fmt,
        path: path.to_path_buf(),
        entries,
        encrypted: any_encrypted,
        total_size,
        total_compressed,
    })
}

pub fn extract(path: &Path, opts: &ExtractOptions, progress: ProgressFn) -> Result<ExtractReport> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file).map_err(map_zip_err)?;
    let names = decode_names(&mut archive)?;
    std::fs::create_dir_all(&opts.dest)?;

    let total = archive.len() as u64;
    let mut guard = DestGuard::new(&opts.dest);
    let mut report = ExtractReport {
        files_written: 0,
        dirs_created: 0,
        bytes_written: 0,
        dest: opts.dest.clone(),
    };

    for (i, name) in names.iter().enumerate() {
        let mut entry = match &opts.password {
            Some(pw) => archive.by_index_decrypt(i, pw.as_bytes()),
            None => archive.by_index(i),
        }
        .map_err(map_zip_err)?;

        if !matches_filter(name, &opts.include) {
            continue;
        }
        let out_path = guard.join(name)?;

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            report.dirs_created += 1;
            continue;
        }

        ensure_parent(&out_path)?;
        let mode = entry.unix_mode();
        let mtime = entry_mtime(&entry);

        // A symlink is stored as a regular entry whose *content* is the target
        // path, flagged in the unix mode. Writing it out as a file would leave
        // a text file where the link should be.
        if mode.is_some_and(|m| m & S_IFMT == S_IFLNK) {
            let mut target = String::new();
            entry.read_to_string(&mut target)?;
            crate::formats::prepare_leaf(&out_path, opts.overwrite)?;
            if opts.overwrite {
                let _ = std::fs::remove_file(&out_path);
            }
            create_symlink(&target, &out_path)?;
            guard.forget(&out_path);
            report.files_written += 1;
            progress(Progress {
                current_path: name.clone(),
                entries_done: i as u64 + 1,
                entries_total: total,
                bytes_done: report.bytes_written,
                bytes_total: 0,
            });
            continue;
        }

        let mut out = create_file(&out_path, opts.overwrite)?;
        let n = io::copy(&mut entry, &mut out)?;
        drop(out);
        restore_metadata(&out_path, mode, mtime);
        report.files_written += 1;
        report.bytes_written += n;
        progress(Progress {
            current_path: name.clone(),
            entries_done: i as u64 + 1,
            entries_total: total,
            bytes_done: report.bytes_written,
            bytes_total: 0,
        });
    }
    Ok(report)
}

pub fn test(path: &Path, opts: &ListOptions, progress: ProgressFn) -> Result<TestReport> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file).map_err(map_zip_err)?;
    let names = decode_names(&mut archive)?;
    let total = archive.len() as u64;
    let mut bad = Vec::new();
    let mut tested = 0u64;
    let mut sink = io::sink();

    for (i, name) in names.iter().enumerate() {
        let res = match &opts.password {
            Some(pw) => archive.by_index_decrypt(i, pw.as_bytes()),
            None => archive.by_index(i),
        };
        let mut entry = match res {
            Ok(e) => e,
            Err(ZipError::InvalidPassword) => return Err(Error::BadPassword),
            Err(e) => {
                bad.push(format!("entry #{i}: {e}"));
                continue;
            }
        };
        if entry.is_dir() {
            continue;
        }
        tested += 1;
        // Reading to EOF makes the zip crate verify the CRC32.
        if let Err(e) = io::copy(&mut entry, &mut sink) {
            bad.push(format!("{name}: {e}"));
        }
        progress(Progress {
            current_path: name.clone(),
            entries_done: i as u64 + 1,
            entries_total: total,
            bytes_done: 0,
            bytes_total: 0,
        });
    }
    Ok(TestReport {
        ok: bad.is_empty(),
        entries_tested: tested,
        bad_entries: bad,
    })
}

/// The entry's modification time as unix seconds.
///
/// The base ZIP header stores an MS-DOS timestamp: two-second resolution, no
/// time zone, and nothing before 1980. The 0x5455 "extended timestamp" extra
/// field carries real UTC unix time, so prefer it whenever a writer left one.
fn entry_mtime(entry: &zip::read::ZipFile<'_>) -> Option<i64> {
    for field in entry.extra_data_fields() {
        if let zip::extra_fields::ExtraField::ExtendedTimestamp(ts) = field {
            if let Some(secs) = ts.mod_time() {
                return Some(secs as i64);
            }
        }
    }
    ts(entry.last_modified())
}

/// Put back what the archive recorded. Best-effort: a mode we cannot apply (or
/// a filesystem that does not have one) is not a reason to fail an extraction.
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

/// This file's permission bits, as ZIP records them.
#[cfg(unix)]
fn permissions_of(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}

/// Windows has no unix mode; keep the executable bit off and honour read-only.
#[cfg(not(unix))]
fn permissions_of(meta: &std::fs::Metadata) -> u32 {
    if meta.permissions().readonly() {
        0o444
    } else {
        0o644
    }
}

fn unix_seconds(t: std::time::SystemTime) -> Option<i64> {
    t.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// Per-entry write options carrying the file's real permissions and timestamp.
fn entry_options<'a>(
    opts: &'a CreateOptions,
    meta: Option<&std::fs::Metadata>,
    mode_override: Option<u32>,
) -> Result<FileOptions<'a, ExtendedFileOptions>> {
    let level = match opts.level {
        Level::Store => None,
        Level::Fast => Some(1),
        Level::Default => Some(6),
        Level::Best => Some(9),
    };
    let method = if matches!(opts.level, Level::Store) {
        CompressionMethod::Stored
    } else {
        CompressionMethod::Deflated
    };

    let mode = mode_override
        .or_else(|| meta.map(permissions_of))
        .unwrap_or(0o644);
    let mut o = FileOptions::<ExtendedFileOptions>::default()
        .compression_method(method)
        .compression_level(level)
        .unix_permissions(mode)
        // Without this, the writer refuses any entry that crosses 4 GiB.
        .large_file(meta.is_some_and(|m| m.len() >= ZIP64_THRESHOLD));

    if let Some(modified) = meta.and_then(|m| m.modified().ok()) {
        let odt = time::OffsetDateTime::from(modified);
        if let Ok(dt) = zip::DateTime::try_from(odt) {
            o = o.last_modified_time(dt);
        }
        if let Some(secs) = unix_seconds(modified) {
            // flags byte (bit 0 = modification time present), then unix time.
            let mut data = Vec::with_capacity(5);
            data.push(0b0000_0001);
            data.extend_from_slice(&(secs as i32).to_le_bytes());
            o.add_extra_data(EXTENDED_TIMESTAMP_ID, data.into_boxed_slice(), false)
                .map_err(map_zip_err)?;
        }
    }

    if let Some(pw) = &opts.password {
        o = o.with_aes_encryption(AesMode::Aes256, pw);
    }
    Ok(o)
}

pub fn create(
    output: &Path,
    inputs: &[std::path::PathBuf],
    opts: &CreateOptions,
    progress: ProgressFn,
) -> Result<CreateReport> {
    let files = collect_inputs(inputs)?;
    let out = File::create(output)?;
    let mut zipw = ZipWriter::new(out);

    let total = files.len() as u64;
    let mut report = CreateReport {
        output: output.to_path_buf(),
        format: Format::Zip,
        entries_added: 0,
        bytes_in: 0,
        bytes_out: 0,
    };

    for (idx, input) in files.iter().enumerate() {
        match input.kind {
            InputKind::EmptyDir => {
                let meta = std::fs::metadata(&input.path).ok();
                let o = entry_options(opts, meta.as_ref(), None)?;
                zipw.add_directory(input.rel.trim_end_matches('/'), o)
                    .map_err(map_zip_err)?;
            }
            InputKind::Symlink => {
                let target = std::fs::read_link(&input.path)?;
                let meta = std::fs::symlink_metadata(&input.path).ok();
                // 0o777 on a symlink is conventional; the type bits are what
                // actually mark it, and readers key off those.
                let o = entry_options(opts, meta.as_ref(), Some(S_IFLNK | 0o777))?;
                zipw.add_symlink(
                    input.rel.as_str(),
                    target.to_string_lossy().replace('\\', "/"),
                    o,
                )
                .map_err(map_zip_err)?;
                report.entries_added += 1;
            }
            InputKind::File => {
                let meta = std::fs::metadata(&input.path)?;
                let o = entry_options(opts, Some(&meta), None)?;
                zipw.start_file(input.rel.as_str(), o).map_err(map_zip_err)?;
                let mut f = File::open(&input.path)?;
                let n = io::copy(&mut f, &mut zipw)?;
                report.entries_added += 1;
                report.bytes_in += n;
            }
        }
        progress(Progress {
            current_path: input.rel.clone(),
            entries_done: idx as u64 + 1,
            entries_total: total,
            bytes_done: report.bytes_in,
            bytes_total: 0,
        });
    }

    let mut finished = zipw.finish().map_err(map_zip_err)?;
    finished.flush()?;
    report.bytes_out = finished.metadata()?.len();
    Ok(report)
}

fn matches_filter(name: &str, include: &[String]) -> bool {
    include.is_empty() || include.iter().any(|p| name.contains(p.as_str()))
}
