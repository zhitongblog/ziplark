use crate::error::{Error, Result};
use crate::formats::{collect_inputs, ensure_parent, safe_join};
use crate::model::*;
use crate::{CreateOptions, ExtractOptions, ListOptions, ProgressFn};
use sevenz_rust2::{
    AesEncoderOptions, Password, SevenZArchiveEntry, SevenZMethod, SevenZReader, SevenZWriter,
};
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

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

pub fn list(path: &Path, fmt: Format, opts: &ListOptions) -> Result<ArchiveInfo> {
    let reader = SevenZReader::open(path, password(&opts.password)).map_err(map_err)?;
    let archive = reader.archive();
    let mut entries = Vec::with_capacity(archive.files.len());
    let mut total_size = 0u64;
    for f in &archive.files {
        total_size += f.size();
        entries.push(ArchiveEntry {
            path: f.name().to_string(),
            is_dir: f.is_directory(),
            size: f.size(),
            compressed_size: None,
            encrypted: opts.password.is_some(),
            modified: None,
            crc32: None,
        });
    }
    let total_compressed = std::fs::metadata(path)?.len();
    Ok(ArchiveInfo {
        format: fmt,
        path: path.to_path_buf(),
        entries,
        encrypted: opts.password.is_some(),
        total_size,
        total_compressed,
    })
}

pub fn extract(path: &Path, opts: &ExtractOptions, progress: ProgressFn) -> Result<ExtractReport> {
    let mut reader = SevenZReader::open(path, password(&opts.password)).map_err(map_err)?;
    std::fs::create_dir_all(&opts.dest)?;

    let mut report = ExtractReport {
        files_written: 0,
        dirs_created: 0,
        bytes_written: 0,
        dest: opts.dest.clone(),
    };
    let mut first_error: Option<Error> = None;
    let mut idx = 0u64;

    reader
        .for_each_entries(|entry, rd| {
            idx += 1;
            let name = entry.name().to_string();
            if !matches_filter(&name, &opts.include) {
                return Ok(true);
            }
            // Funnel through the shared zip-slip guard.
            let out_path = match safe_join(&opts.dest, &name) {
                Ok(p) => p,
                Err(e) => {
                    first_error = Some(e);
                    return Ok(false);
                }
            };
            if entry.is_directory() {
                if let Err(e) = std::fs::create_dir_all(&out_path) {
                    first_error = Some(Error::Io(e));
                    return Ok(false);
                }
                report.dirs_created += 1;
                return Ok(true);
            }
            if let Err(e) = ensure_parent(&out_path) {
                first_error = Some(e);
                return Ok(false);
            }
            if out_path.exists() && !opts.overwrite {
                first_error = Some(Error::other(format!(
                    "{} already exists (use overwrite)",
                    out_path.display()
                )));
                return Ok(false);
            }
            let mut out = match File::create(&out_path) {
                Ok(f) => f,
                Err(e) => {
                    first_error = Some(Error::Io(e));
                    return Ok(false);
                }
            };
            match io::copy(rd, &mut out) {
                Ok(n) => {
                    report.files_written += 1;
                    report.bytes_written += n;
                }
                Err(e) => {
                    first_error = Some(Error::Io(e));
                    return Ok(false);
                }
            }
            progress(Progress {
                current_path: name,
                entries_done: idx,
                entries_total: 0,
                bytes_done: report.bytes_written,
                bytes_total: 0,
            });
            Ok(true)
        })
        .map_err(map_err)?;

    if let Some(e) = first_error {
        return Err(e);
    }
    Ok(report)
}

pub fn test(path: &Path, opts: &ListOptions, progress: ProgressFn) -> Result<TestReport> {
    let mut reader = SevenZReader::open(path, password(&opts.password)).map_err(map_err)?;
    let mut tested = 0u64;
    let mut bad = Vec::new();
    reader
        .for_each_entries(|entry, rd| {
            if entry.is_directory() {
                return Ok(true);
            }
            tested += 1;
            let name = entry.name().to_string();
            let mut sink = io::sink();
            if let Err(e) = io::copy(rd, &mut sink) {
                bad.push(format!("{name}: {e}"));
            }
            progress(Progress {
                current_path: name,
                entries_done: tested,
                entries_total: 0,
                bytes_done: 0,
                bytes_total: 0,
            });
            Ok(true)
        })
        .map_err(map_err)?;
    Ok(TestReport {
        ok: bad.is_empty(),
        entries_tested: tested,
        bad_entries: bad,
    })
}

pub fn create(
    output: &Path,
    inputs: &[PathBuf],
    opts: &CreateOptions,
    progress: ProgressFn,
) -> Result<CreateReport> {
    let files = collect_inputs(inputs)?;
    let out = File::create(output)?;
    let mut writer = SevenZWriter::new(out).map_err(map_err)?;

    if let Some(pw) = &opts.password {
        // Match the crate's own encrypted-archive recipe: AES-256 over LZMA2.
        writer.set_content_methods(vec![
            AesEncoderOptions::new(Password::from(pw.as_str())).into(),
            SevenZMethod::LZMA2.into(),
        ]);
    }

    let total = files.len() as u64;
    let mut entries_added = 0u64;
    let mut bytes_in = 0u64;

    for (idx, (src, rel)) in files.iter().enumerate() {
        if rel.ends_with('/') {
            let entry = SevenZArchiveEntry::new_folder(rel.trim_end_matches('/'));
            writer
                .push_archive_entry::<&[u8]>(entry, None)
                .map_err(map_err)?;
        } else {
            let entry = SevenZArchiveEntry::from_path(src, rel.clone());
            let f = File::open(src)?;
            bytes_in += f.metadata()?.len();
            writer.push_archive_entry(entry, Some(f)).map_err(map_err)?;
            entries_added += 1;
        }
        progress(Progress {
            current_path: rel.clone(),
            entries_done: idx as u64 + 1,
            entries_total: total,
            bytes_done: bytes_in,
            bytes_total: 0,
        });
    }

    let mut finished = writer.finish().map_err(|e| Error::Io(e))?;
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

fn matches_filter(name: &str, include: &[String]) -> bool {
    include.is_empty() || include.iter().any(|p| name.contains(p.as_str()))
}
