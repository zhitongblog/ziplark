use crate::error::{Error, Result};
use crate::formats::{collect_inputs, ensure_parent, safe_join};
use crate::model::*;
use crate::{CreateOptions, ExtractOptions, Level, ListOptions, ProgressFn};
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use zip::result::ZipError;
use zip::{AesMode, CompressionMethod, ZipArchive, ZipWriter};

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

pub fn list(path: &Path, fmt: Format, _opts: &ListOptions) -> Result<ArchiveInfo> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file).map_err(map_zip_err)?;
    let mut entries = Vec::with_capacity(archive.len());
    let mut total_size = 0u64;
    let mut total_compressed = 0u64;
    let mut any_encrypted = false;

    for i in 0..archive.len() {
        // by_index_raw exposes metadata without needing the password.
        let e = archive.by_index_raw(i).map_err(map_zip_err)?;
        let encrypted = e.encrypted();
        any_encrypted |= encrypted;
        total_size += e.size();
        total_compressed += e.compressed_size();
        entries.push(ArchiveEntry {
            path: e.name().to_string(),
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
    std::fs::create_dir_all(&opts.dest)?;

    let total = archive.len() as u64;
    let mut report = ExtractReport {
        files_written: 0,
        dirs_created: 0,
        bytes_written: 0,
        dest: opts.dest.clone(),
    };

    for i in 0..archive.len() {
        let mut entry = match &opts.password {
            Some(pw) => archive.by_index_decrypt(i, pw.as_bytes()),
            None => archive.by_index(i),
        }
        .map_err(map_zip_err)?;

        let name = entry.name().to_string();
        if !matches_filter(&name, &opts.include) {
            continue;
        }
        let out_path = safe_join(&opts.dest, &name)?;

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            report.dirs_created += 1;
            continue;
        }

        ensure_parent(&out_path)?;
        if out_path.exists() && !opts.overwrite {
            return Err(Error::other(format!(
                "{} already exists (use overwrite)",
                out_path.display()
            )));
        }
        let mut out = File::create(&out_path)?;
        let n = io::copy(&mut entry, &mut out)?;
        report.files_written += 1;
        report.bytes_written += n;
        progress(Progress {
            current_path: name,
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
    let total = archive.len() as u64;
    let mut bad = Vec::new();
    let mut tested = 0u64;
    let mut sink = io::sink();

    for i in 0..archive.len() {
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
        let name = entry.name().to_string();
        if entry.is_dir() {
            continue;
        }
        tested += 1;
        // Reading to EOF makes the zip crate verify the CRC32.
        if let Err(e) = io::copy(&mut entry, &mut sink) {
            bad.push(format!("{name}: {e}"));
        }
        progress(Progress {
            current_path: name,
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

pub fn create(
    output: &Path,
    inputs: &[std::path::PathBuf],
    opts: &CreateOptions,
    progress: ProgressFn,
) -> Result<CreateReport> {
    let files = collect_inputs(inputs)?;
    let out = File::create(output)?;
    let mut zipw = ZipWriter::new(out);

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

    let total = files.len() as u64;
    let mut report = CreateReport {
        output: output.to_path_buf(),
        format: Format::Zip,
        entries_added: 0,
        bytes_in: 0,
        bytes_out: 0,
    };

    for (idx, (src, rel)) in files.iter().enumerate() {
        let mut fileopts = zip::write::SimpleFileOptions::default()
            .compression_method(method)
            .compression_level(level)
            .unix_permissions(0o644);
        if let Some(pw) = &opts.password {
            fileopts = fileopts.with_aes_encryption(AesMode::Aes256, pw);
        }

        if rel.ends_with('/') {
            zipw.add_directory(rel.trim_end_matches('/'), fileopts)
                .map_err(map_zip_err)?;
            continue;
        }

        zipw.start_file(rel.as_str(), fileopts).map_err(map_zip_err)?;
        let mut f = File::open(src)?;
        let n = io::copy(&mut f, &mut zipw)?;
        report.entries_added += 1;
        report.bytes_in += n;
        progress(Progress {
            current_path: rel.clone(),
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
