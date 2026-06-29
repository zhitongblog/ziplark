use crate::error::{Error, Result};
use crate::formats::{ensure_parent, safe_join};
use crate::model::*;
use crate::{ExtractOptions, ListOptions, ProgressFn};
use std::path::Path;
use unrar::error::{Code, UnrarError};
use unrar::Archive;

fn map_err(e: UnrarError) -> Error {
    match e.code {
        Code::MissingPassword => Error::PasswordRequired,
        Code::BadPassword => Error::BadPassword,
        Code::BadData | Code::BadArchive => Error::corrupt("RAR data is corrupt"),
        Code::UnknownFormat => Error::UnsupportedFormat(None),
        other => Error::other(format!("RAR error: {other:?}")),
    }
}

pub fn list(path: &Path, fmt: Format, opts: &ListOptions) -> Result<ArchiveInfo> {
    let p = path.to_path_buf();
    let iter = match &opts.password {
        Some(pw) => Archive::with_password(&p, pw).open_for_listing(),
        None => Archive::new(&p).open_for_listing(),
    }
    .map_err(map_err)?;

    let mut entries = Vec::new();
    let mut total_size = 0u64;
    let mut any_encrypted = false;
    for header in iter {
        let h = header.map_err(map_err)?;
        let encrypted = h.is_encrypted();
        any_encrypted |= encrypted;
        total_size += h.unpacked_size;
        entries.push(ArchiveEntry {
            path: h.filename.to_string_lossy().to_string(),
            is_dir: h.is_directory(),
            size: h.unpacked_size,
            compressed_size: None,
            encrypted,
            modified: None,
            crc32: Some(h.file_crc),
        });
    }

    let total_compressed = std::fs::metadata(path)?.len();
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
    let p = path.to_path_buf();
    std::fs::create_dir_all(&opts.dest)?;
    let mut arch = match &opts.password {
        Some(pw) => Archive::with_password(&p, pw).open_for_processing(),
        None => Archive::new(&p).open_for_processing(),
    }
    .map_err(map_err)?;

    let mut report = ExtractReport {
        files_written: 0,
        dirs_created: 0,
        bytes_written: 0,
        dest: opts.dest.clone(),
    };
    let mut idx = 0u64;

    loop {
        let bf = match arch.read_header().map_err(map_err)? {
            Some(a) => a,
            None => break,
        };
        // Copy header fields out before consuming the cursor.
        let (name, is_dir, size) = {
            let e = bf.entry();
            (
                e.filename.to_string_lossy().to_string(),
                e.is_directory(),
                e.unpacked_size,
            )
        };

        let wanted = matches_filter(&name, &opts.include);
        if is_dir {
            if wanted {
                let out_path = safe_join(&opts.dest, &name)?;
                std::fs::create_dir_all(&out_path)?;
                report.dirs_created += 1;
            }
            arch = bf.skip().map_err(map_err)?;
            continue;
        }
        if !wanted {
            arch = bf.skip().map_err(map_err)?;
            continue;
        }

        let out_path = safe_join(&opts.dest, &name)?;
        ensure_parent(&out_path)?;
        if out_path.exists() && !opts.overwrite {
            return Err(Error::other(format!(
                "{} already exists (use overwrite)",
                out_path.display()
            )));
        }
        arch = bf.extract_to(&out_path).map_err(map_err)?;
        report.files_written += 1;
        report.bytes_written += size;
        idx += 1;
        progress(Progress {
            current_path: name,
            entries_done: idx,
            entries_total: 0,
            bytes_done: report.bytes_written,
            bytes_total: 0,
        });
    }
    Ok(report)
}

pub fn test(path: &Path, opts: &ListOptions, progress: ProgressFn) -> Result<TestReport> {
    // libunrar verifies each file's CRC on extraction, so we extract to a
    // throwaway directory and report any entry that fails.
    let tmp = std::env::temp_dir().join(format!("ziplark-rartest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let xopts = ExtractOptions {
        password: opts.password.clone(),
        dest: tmp.clone(),
        overwrite: true,
        include: Vec::new(),
    };
    let result = extract(path, &xopts, progress);
    let _ = std::fs::remove_dir_all(&tmp);

    match result {
        Ok(report) => Ok(TestReport {
            ok: true,
            entries_tested: report.files_written,
            bad_entries: Vec::new(),
        }),
        Err(Error::BadPassword) => Err(Error::BadPassword),
        Err(Error::PasswordRequired) => Err(Error::PasswordRequired),
        Err(e) => Ok(TestReport {
            ok: false,
            entries_tested: 0,
            bad_entries: vec![e.to_string()],
        }),
    }
}

fn matches_filter(name: &str, include: &[String]) -> bool {
    include.is_empty() || include.iter().any(|p| name.contains(p.as_str()))
}
