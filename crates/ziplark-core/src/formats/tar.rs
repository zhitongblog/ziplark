use crate::encoding::NameDecoder;
use crate::error::{Error, Result};
use crate::formats::{
    collect_inputs, create_symlink, ensure_parent, prepare_leaf, report, DestGuard, Input,
    InputKind,
};
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
        Format::TarLz4 => Box::new(lz4_flex::frame::FrameDecoder::new(f)),
        _ => return Err(Error::UnsupportedFormat(Some(path.to_path_buf()))),
    })
}

pub fn list(path: &Path, fmt: Format, _opts: &ListOptions) -> Result<ArchiveInfo> {
    let reader = open_reader(path, fmt)?;
    let mut archive = tar::Archive::new(reader);
    let mut entries = Vec::new();
    let mut total_size = 0u64;

    let mut names = NameDecoder::new();
    for entry in archive.entries()? {
        let entry = entry?;
        let is_dir = entry.header().entry_type().is_dir();
        let size = entry.size();
        total_size += size;
        let raw = entry.path_bytes().into_owned();
        names.sample(&raw);
        let name = names.decode(&raw);
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

    let mut guard = DestGuard::new(&opts.dest);
    let mut names = NameDecoder::new();
    for (i, entry) in archive.entries()?.enumerate() {
        let mut entry = entry?;
        let idx = i as u64 + 1;
        let raw = entry.path_bytes().into_owned();
        names.sample(&raw);
        let name = names.decode(&raw);
        if !matches_filter(&name, &opts.include) {
            continue;
        }
        // Validates the name *and* refuses to descend through a symlink — tar
        // is the one format of ours that stores links, so an earlier entry can
        // have planted one right in our path.
        let out_path = guard.join(&name)?;

        let kind = entry.header().entry_type();
        if kind.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            report.dirs_created += 1;
            continue;
        }
        ensure_parent(&out_path)?;

        if kind.is_symlink() || kind.is_hard_link() {
            let target_raw = entry
                .link_name_bytes()
                .ok_or_else(|| Error::corrupt(format!("{name}: link entry with no target")))?
                .into_owned();
            let target = names.decode(&target_raw);
            unpack_link(&mut guard, kind.is_symlink(), &target, &out_path, opts.overwrite)?;
            report.files_written += 1;
            crate::formats::report(
                progress,
                Progress {
                    current_path: name,
                    entries_done: idx,
                    entries_total: 0,
                    bytes_done: report.bytes_written,
                    bytes_total: 0,
                },
            )?;
            continue;
        }

        // Regular file. `unpack` writes it (and restores the mtime), but it
        // would happily write *through* a symlink already sitting at this path,
        // so clear the leaf first.
        prepare_leaf(&out_path, opts.overwrite)?;
        entry.unpack(&out_path)?;
        report.files_written += 1;
        report.bytes_written += entry.size();
        crate::formats::report(
            progress,
            Progress {
                current_path: name,
                entries_done: idx,
                entries_total: 0,
                bytes_done: report.bytes_written,
                bytes_total: 0,
            },
        )?;
    }
    Ok(report)
}

/// Restore a link entry.
///
/// A **hard** link's target names a file inside the archive, so it goes through
/// the guard: a tar claiming `link -> /etc/shadow` must not get one.
///
/// A **symlink**'s target is just a string that the OS resolves whenever the
/// link is used later. Absolute and `..` targets are legal and ordinary there —
/// packaging tarballs are full of them — so it is stored verbatim, the same as
/// GNU tar and bsdtar. That is safe because the link cannot be *used* to escape
/// during extraction: every later entry re-checks its ancestors through
/// `guard`, which is what closes the `evil -> /tmp` + `evil/owned.txt` attack.
fn unpack_link(
    guard: &mut DestGuard,
    is_symlink: bool,
    target: &str,
    out_path: &Path,
    overwrite: bool,
) -> Result<()> {
    if target.is_empty() {
        return Err(Error::corrupt(format!(
            "{}: link entry with an empty target",
            out_path.display()
        )));
    }
    prepare_leaf(out_path, overwrite)?;
    // prepare_leaf only clears a symlink; neither symlink() nor hard_link() can
    // replace an existing file, so with overwrite the leaf has to go entirely.
    if overwrite {
        let _ = std::fs::remove_file(out_path);
    }

    if is_symlink {
        create_symlink(target, out_path)?;
        // This path is a symlink now — it must never be remembered as a
        // directory that is safe to descend through.
        guard.forget(out_path);
    } else {
        let src = guard.join(target)?;
        std::fs::hard_link(&src, out_path)?;
    }
    Ok(())
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
        report(
            progress,
            Progress {
                current_path: name,
                entries_done: tested,
                entries_total: 0,
                bytes_done: 0,
                bytes_total: 0,
            },
        )?;
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
        Format::TarLz4 => {
            let enc = lz4_flex::frame::FrameEncoder::new(out);
            let mut b = tar::Builder::new(enc);
            let r = add_all(&mut b, &files, progress)?;
            b.into_inner()?
                .finish()
                .map_err(|e| Error::other(e.to_string()))?;
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
    files: &[Input],
    progress: ProgressFn,
) -> Result<(u64, u64)> {
    let total = files.len() as u64;
    let mut entries_added = 0u64;
    let mut bytes_in = 0u64;
    for (idx, input) in files.iter().enumerate() {
        match input.kind {
            InputKind::EmptyDir => {
                builder.append_dir(input.rel.trim_end_matches('/'), &input.path)?;
            }
            InputKind::Symlink => {
                // Store the link itself. `append_file` would have followed it
                // and archived a second copy of whatever it points at.
                let target = std::fs::read_link(&input.path)?;
                let meta = std::fs::symlink_metadata(&input.path)?;
                let mut header = tar::Header::new_gnu();
                header.set_metadata(&meta);
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                builder.append_link(&mut header, &input.rel, &target)?;
            }
            InputKind::File => {
                let mut f = File::open(&input.path)?;
                bytes_in += f.metadata()?.len();
                builder.append_file(&input.rel, &mut f)?;
            }
        }
        entries_added += 1;
        report(
            progress,
            Progress {
                current_path: input.rel.clone(),
                entries_done: idx as u64 + 1,
                entries_total: total,
                bytes_done: bytes_in,
                bytes_total: 0,
            },
        )?;
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
