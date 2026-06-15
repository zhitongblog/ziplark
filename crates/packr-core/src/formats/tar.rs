use crate::error::{Error, Result};
use crate::formats::{collect_inputs, ensure_parent, safe_join};
use crate::model::*;
use crate::{CreateOptions, ExtractOptions, Level, ListOptions, ProgressFn};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// Build a decompressing reader for any tar variant.
fn open_reader(path: &Path, fmt: Format) -> Result<Box<dyn Read>> {
    let f = File::open(path)?;
    Ok(match fmt {
        Format::Tar => Box::new(f),
        Format::TarGz => Box::new(flate2::read::GzDecoder::new(f)),
        Format::TarBz2 => Box::new(bzip2::read::BzDecoder::new(f)),
        Format::TarXz => Box::new(xz2::read::XzDecoder::new(f)),
        Format::TarZst => Box::new(zstd::stream::read::Decoder::new(f)?),
        _ => return Err(Error::UnsupportedFormat(Some(path.to_path_buf()))),
    })
}

pub fn list(path: &Path, fmt: Format, _opts: &ListOptions) -> Result<ArchiveInfo> {
    let reader = open_reader(path, fmt)?;
    let mut archive = tar::Archive::new(reader);
    let mut entries = Vec::new();
    let mut total_size = 0u64;

    for entry in archive.entries()? {
        let entry = entry?;
        let is_dir = entry.header().entry_type().is_dir();
        let size = entry.size();
        total_size += size;
        let name = entry.path()?.to_string_lossy().to_string();
        entries.push(ArchiveEntry {
            path: name,
            is_dir,
            size,
            compressed_size: None,
            encrypted: false,
            modified: entry.header().mtime().ok().map(|m| m as i64),
            crc32: None,
        });
    }

    let total_compressed = std::fs::metadata(path)?.len();
    Ok(ArchiveInfo {
        format: fmt,
        path: path.to_path_buf(),
        entries,
        encrypted: false,
        total_size,
        total_compressed,
    })
}

pub fn extract(
    path: &Path,
    fmt: Format,
    opts: &ExtractOptions,
    progress: ProgressFn,
) -> Result<ExtractReport> {
    let reader = open_reader(path, fmt)?;
    let mut archive = tar::Archive::new(reader);
    std::fs::create_dir_all(&opts.dest)?;
    let mut report = ExtractReport {
        files_written: 0,
        dirs_created: 0,
        bytes_written: 0,
        dest: opts.dest.clone(),
    };

    let mut idx = 0u64;
    for entry in archive.entries()? {
        let mut entry = entry?;
        idx += 1;
        let name = entry.path()?.to_string_lossy().to_string();
        if !matches_filter(&name, &opts.include) {
            continue;
        }
        let out_path = safe_join(&opts.dest, &name)?;

        if entry.header().entry_type().is_dir() {
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
        // entry.unpack handles regular files, symlinks and hardlinks; the path
        // has already been validated by safe_join.
        entry.unpack(&out_path)?;
        report.files_written += 1;
        report.bytes_written += entry.size();
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

pub fn test(path: &Path, fmt: Format, _opts: &ListOptions, progress: ProgressFn) -> Result<TestReport> {
    let reader = open_reader(path, fmt)?;
    let mut archive = tar::Archive::new(reader);
    let mut tested = 0u64;
    let mut bad = Vec::new();
    let mut sink = io::sink();

    for entry in archive.entries()? {
        let mut entry = match entry {
            Ok(e) => e,
            Err(e) => {
                bad.push(e.to_string());
                continue;
            }
        };
        if entry.header().entry_type().is_dir() {
            continue;
        }
        tested += 1;
        let name = entry.path().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        if let Err(e) = io::copy(&mut entry, &mut sink) {
            bad.push(format!("{name}: {e}"));
        }
        progress(Progress {
            current_path: name,
            entries_done: tested,
            entries_total: 0,
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
    inputs: &[PathBuf],
    opts: &CreateOptions,
    progress: ProgressFn,
) -> Result<CreateReport> {
    let files = collect_inputs(inputs)?;
    let out = File::create(output)?;

    let (entries_added, bytes_in) = match opts.format {
        Format::Tar => {
            let mut b = tar::Builder::new(out);
            let r = add_all(&mut b, &files, progress)?;
            b.into_inner()?.flush()?;
            r
        }
        Format::TarGz => {
            let enc = flate2::write::GzEncoder::new(out, gz_level(opts.level));
            let mut b = tar::Builder::new(enc);
            let r = add_all(&mut b, &files, progress)?;
            b.into_inner()?.finish()?;
            r
        }
        Format::TarBz2 => {
            let enc = bzip2::write::BzEncoder::new(out, bz_level(opts.level));
            let mut b = tar::Builder::new(enc);
            let r = add_all(&mut b, &files, progress)?;
            b.into_inner()?.finish()?;
            r
        }
        Format::TarXz => {
            let enc = xz2::write::XzEncoder::new(out, xz_level(opts.level));
            let mut b = tar::Builder::new(enc);
            let r = add_all(&mut b, &files, progress)?;
            b.into_inner()?.finish()?;
            r
        }
        Format::TarZst => {
            let enc = zstd::stream::write::Encoder::new(out, zst_level(opts.level))?;
            let mut b = tar::Builder::new(enc.auto_finish());
            let r = add_all(&mut b, &files, progress)?;
            b.into_inner()?;
            r
        }
        _ => return Err(Error::UnsupportedFormat(Some(output.to_path_buf()))),
    };

    let bytes_out = std::fs::metadata(output)?.len();
    Ok(CreateReport {
        output: output.to_path_buf(),
        format: opts.format,
        entries_added,
        bytes_in,
        bytes_out,
    })
}

fn add_all<W: Write>(
    builder: &mut tar::Builder<W>,
    files: &[(PathBuf, String)],
    progress: ProgressFn,
) -> Result<(u64, u64)> {
    let total = files.len() as u64;
    let mut entries_added = 0u64;
    let mut bytes_in = 0u64;
    for (idx, (src, rel)) in files.iter().enumerate() {
        if rel.ends_with('/') {
            builder.append_dir(rel.trim_end_matches('/'), src)?;
        } else {
            let mut f = File::open(src)?;
            bytes_in += f.metadata()?.len();
            builder.append_file(rel, &mut f)?;
        }
        entries_added += 1;
        progress(Progress {
            current_path: rel.clone(),
            entries_done: idx as u64 + 1,
            entries_total: total,
            bytes_done: bytes_in,
            bytes_total: 0,
        });
    }
    Ok((entries_added, bytes_in))
}

fn gz_level(l: Level) -> flate2::Compression {
    match l {
        Level::Store => flate2::Compression::none(),
        Level::Fast => flate2::Compression::fast(),
        Level::Default => flate2::Compression::default(),
        Level::Best => flate2::Compression::best(),
    }
}
fn bz_level(l: Level) -> bzip2::Compression {
    match l {
        Level::Store | Level::Fast => bzip2::Compression::fast(),
        Level::Default => bzip2::Compression::default(),
        Level::Best => bzip2::Compression::best(),
    }
}
fn xz_level(l: Level) -> u32 {
    match l {
        Level::Store => 0,
        Level::Fast => 1,
        Level::Default => 6,
        Level::Best => 9,
    }
}
fn zst_level(l: Level) -> i32 {
    match l {
        Level::Store => 1,
        Level::Fast => 3,
        Level::Default => 9,
        Level::Best => 19,
    }
}

fn matches_filter(name: &str, include: &[String]) -> bool {
    include.is_empty() || include.iter().any(|p| name.contains(p.as_str()))
}
