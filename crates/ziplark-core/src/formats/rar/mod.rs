//! RAR / RAR5 — read-only, and the format Ziplark means to be best at.
//!
//! Three things make RAR different from every other format the engine reads,
//! and all three are handled here rather than pushed onto the caller:
//!
//! * **An archive is often more than one file.** `movie.part01.rar` +
//!   `part02` + `part03`, or the older `movie.rar` + `movie.r00` + `movie.r01`.
//!   Any one of those is a reasonable thing to double-click, so opening any
//!   part opens the whole set, the parts are reported, and a set with a hole in
//!   it names *which volume is missing* instead of failing obscurely.
//! * **Integrity is checked by unpacking.** libunrar verifies a file's
//!   checksum as it decompresses, so testing an archive means running the
//!   decompressor — but not, as this module used to, writing the whole archive
//!   to a temporary directory to do it.
//! * **Damage is normal.** RAR is what large downloads arrive in, and they
//!   arrive truncated, bit-rotted, or one volume short. So testing reports
//!   every bad entry rather than the first, and extraction can be asked to
//!   salvage what is readable instead of throwing away the 99% that was fine.
//!
//! [`raw`] is the libunrar binding underneath; it explains why this does not
//! use the `unrar` crate.

mod raw;
mod volumes;

use crate::error::{Error, Result};
use crate::formats::{ensure_parent, prepare_dir, prepare_leaf, unpack_link, DestGuard};
use crate::model::*;
use crate::{ExtractOptions, ListOptions, ProgressFn};
use raw::{Archive, LinkKind, Mode};
use std::path::Path;
pub use volumes::first_volume;

/// Record an entry that could not be dealt with, or fail outright.
///
/// Whether a bad entry is fatal is the caller's call: by default the first one
/// stops everything, which is what you want when extracting something you
/// expect to be intact. `keep_broken` turns it into a line in the report, which
/// is what you want when the archive is the only copy you have.
fn note(failed: &mut Vec<String>, keep_broken: bool, name: &str, e: Error) -> Result<()> {
    if keep_broken {
        failed.push(format!("{name}: {e}"));
        Ok(())
    } else {
        Err(e)
    }
}

fn entry_of(e: &raw::Entry) -> ArchiveEntry {
    ArchiveEntry {
        path: e.name.clone(),
        is_dir: e.is_dir,
        size: e.size,
        compressed_size: Some(e.packed_size),
        encrypted: e.encrypted,
        modified: e.modified,
        crc32: e.crc32,
        split: e.split(),
    }
}

// ───────────────────────────── listing ─────────────────────────────

pub fn list(path: &Path, fmt: Format, opts: &ListOptions) -> Result<ArchiveInfo> {
    let first = first_volume(path);
    let mut archive = Archive::open(&first, Mode::List, opts.password.as_deref(), false)?;

    let attributes = ArchiveAttributes {
        solid: archive.is_solid(),
        recovery_record: archive.has_recovery_record(),
        encrypted_headers: archive.has_encrypted_headers(),
        locked: archive.is_locked(),
    };
    let comment = archive.comment().map(str::to_owned);

    let mut entries = Vec::new();
    let mut total_size = 0u64;
    let mut any_encrypted = attributes.encrypted_headers;
    let mut missing_volume = None;

    loop {
        match archive.read_header() {
            Ok(Some(entry)) => {
                any_encrypted |= entry.encrypted;
                // A split entry's size is the whole file's, repeated in each
                // volume's header; count it once, where it starts.
                if !entry.split_before {
                    total_size += entry.size;
                }
                entries.push(entry_of(&entry));
                if let Err(e) = archive.skip() {
                    missing_volume = incomplete(&e)?;
                    break;
                }
            }
            Ok(None) => break,
            // A set with a hole in it lists what the volumes on hand contain
            // and says which one is missing — the same as WinRAR, and more use
            // than refusing to show anything.
            Err(e) => {
                missing_volume = incomplete(&e)?;
                break;
            }
        }
    }

    let parts = volumes::present(&first);
    let total_compressed = parts
        .iter()
        .map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .sum::<u64>()
        .max(std::fs::metadata(&first)?.len());

    Ok(ArchiveInfo {
        format: fmt,
        path: first,
        entries,
        encrypted: any_encrypted,
        total_size,
        total_compressed,
        volumes: parts,
        missing_volume,
        comment,
        attributes,
    })
}

/// A missing volume is information; every other error is an error.
fn incomplete(e: &Error) -> Result<Option<std::path::PathBuf>> {
    match e {
        Error::MissingVolume(p) => Ok(Some(p.clone())),
        other => Err(Error::other(other.to_string())),
    }
}

// ───────────────────────────── extraction ─────────────────────────────

pub fn extract(path: &Path, opts: &ExtractOptions, progress: ProgressFn) -> Result<ExtractReport> {
    let first = first_volume(path);
    // One header pass first. It costs almost nothing and buys two things: real
    // totals for the progress bar, and knowing about a missing volume *before*
    // writing anything rather than part-way through.
    let info = list(
        &first,
        Format::Rar,
        &ListOptions {
            password: opts.password.clone(),
        },
    )?;
    if let (Some(missing), false) = (&info.missing_volume, opts.keep_broken) {
        return Err(Error::MissingVolume(missing.clone()));
    }

    let selector = opts.selector();
    let (entries_total, bytes_total) = info
        .entries
        .iter()
        .filter(|e| !e.is_dir && selector.matches(&e.path))
        .fold((0u64, 0u64), |(n, b), e| (n + 1, b + e.size));

    std::fs::create_dir_all(&opts.dest)?;
    let mut report = ExtractReport {
        files_written: 0,
        dirs_created: 0,
        bytes_written: 0,
        dest: opts.dest.clone(),
        failed: Vec::new(),
        partial: Vec::new(),
    };
    let mut guard = DestGuard::new(&opts.dest);
    let mut archive = Archive::open(
        &first,
        Mode::Process,
        opts.password.as_deref(),
        opts.keep_broken,
    )?;
    // Reported once at the end, however many entries ran into it.
    let mut missing_volume = info.missing_volume.clone();

    loop {
        let entry = match archive.read_header() {
            Ok(Some(e)) => e,
            Ok(None) => break,
            // Nothing readable follows a header we cannot parse, so this ends
            // the extraction either way — the only question is how loudly.
            Err(Error::MissingVolume(p)) => {
                missing_volume = Some(p);
                break;
            }
            Err(e) => {
                note(&mut report.failed, opts.keep_broken, "archive", e)?;
                break;
            }
        };

        if !selector.matches(&entry.name) {
            archive.skip()?;
            continue;
        }

        // Nothing the guard rejects is handed to libunrar at all, so a refused
        // entry writes nothing and the archive stays usable.
        let out_path = match guard.join(&entry.name) {
            Ok(p) => p,
            Err(e) => {
                note(&mut report.failed, opts.keep_broken, &entry.name, e)?;
                archive.skip()?;
                continue;
            }
        };

        if entry.is_dir {
            match prepare_dir(&out_path, &entry.name) {
                Ok(()) => report.dirs_created += 1,
                Err(e) => note(&mut report.failed, opts.keep_broken, &entry.name, e)?,
            }
            archive.skip()?;
            continue;
        }

        if let Err(e) = ensure_parent(&out_path) {
            note(&mut report.failed, opts.keep_broken, &entry.name, e)?;
            archive.skip()?;
            continue;
        }

        // A link entry carries a target rather than data. libunrar would create
        // it for us, but then the target would never pass through our guard, so
        // links are restored here — identically to how tar's are.
        if let Some((kind, target)) = &entry.link {
            let restored = match kind {
                LinkKind::Symlink => {
                    unpack_link(&mut guard, true, target, &out_path, opts.overwrite)
                }
                LinkKind::HardLink => {
                    unpack_link(&mut guard, false, target, &out_path, opts.overwrite)
                }
                // RAR5 deduplication: the entry *is* another entry's data, so
                // it can only be restored if that one was extracted too.
                LinkKind::Copy => guard.join(target).and_then(|src| {
                    prepare_leaf(&out_path, opts.overwrite)?;
                    std::fs::copy(&src, &out_path).map(|_| ()).map_err(|e| {
                        Error::other(format!(
                            "copies {} , which was not extracted: {e}",
                            src.display()
                        ))
                    })
                }),
            };
            match restored {
                Ok(()) => report.files_written += 1,
                Err(e) => note(&mut report.failed, opts.keep_broken, &entry.name, e)?,
            }
            archive.skip()?;
            continue;
        }

        // libunrar opens the destination itself, so a symlink sitting at the
        // leaf has to be cleared here rather than at the point of writing —
        // otherwise the write follows it out of the destination.
        if let Err(e) = prepare_leaf(&out_path, opts.overwrite) {
            note(&mut report.failed, opts.keep_broken, &entry.name, e)?;
            archive.skip()?;
            continue;
        }

        match archive.extract_to(&out_path) {
            Ok(()) => {
                guard.forget(&out_path);
                report.files_written += 1;
                report.bytes_written += entry.size;
                restore_metadata(&out_path, &entry);
                crate::formats::report(
                    progress,
                    Progress {
                        current_path: entry.name.clone(),
                        entries_done: report.files_written,
                        entries_total,
                        bytes_done: report.bytes_written,
                        bytes_total,
                    },
                )?;
            }
            // An entry that runs off the end of the volumes we have is the
            // same one fact as the set being incomplete; say it once.
            Err(Error::MissingVolume(p)) => {
                missing_volume = Some(p);
                report.partial.push(entry.name.clone());
            }
            // An entry that fails to decompress has already consumed its data,
            // so the next header can be read straight away — this is the whole
            // reason for talking to libunrar directly.
            Err(e) => note(&mut report.failed, opts.keep_broken, &entry.name, e)?,
        }
    }

    if let Some(missing) = missing_volume {
        report.failed.push(format!(
            "{} is missing, so the archive stops there",
            missing.display()
        ));
    }
    Ok(report)
}

/// Put back what the archive recorded about the file itself.
///
/// Best-effort, like the other formats: a mode this filesystem cannot express
/// is not a reason to fail an extraction that otherwise worked.
fn restore_metadata(path: &Path, entry: &raw::Entry) {
    #[cfg(unix)]
    if let Some(mode) = entry.unix_mode() {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    if let Some(secs) = entry.modified {
        let t = filetime::FileTime::from_unix_time(secs, 0);
        let _ = filetime::set_file_times(path, t, t);
    }
}

// ───────────────────────────── testing ─────────────────────────────

pub fn test(path: &Path, opts: &ListOptions, progress: ProgressFn) -> Result<TestReport> {
    let first = first_volume(path);
    let info = list(&first, Format::Rar, opts)?;
    let entries_total = info.entries.iter().filter(|e| !e.is_dir).count() as u64;
    let bytes_total = info.entries.iter().map(|e| e.size).sum();

    let mut archive = Archive::open(&first, Mode::Process, opts.password.as_deref(), false)?;
    let mut bad_entries = Vec::new();
    let mut tested = 0u64;
    let mut bytes = 0u64;

    loop {
        let entry = match archive.read_header() {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(e) => {
                bad_entries.push(format!("archive: {e}"));
                break;
            }
        };
        if entry.is_dir || entry.link.is_some() {
            archive.skip()?;
            continue;
        }

        // Decompressing and discarding the bytes is what checks the checksum.
        // No temporary directory, which is what this used to need — testing a
        // 40 GB archive wrote 40 GB to /tmp.
        match archive.test() {
            Ok(()) => {
                tested += 1;
                bytes += entry.size;
                crate::formats::report(
                    progress,
                    Progress {
                        current_path: entry.name.clone(),
                        entries_done: tested,
                        entries_total,
                        bytes_done: bytes,
                        bytes_total,
                    },
                )?;
            }
            Err(e @ (Error::BadPassword | Error::PasswordRequired | Error::Cancelled)) => {
                return Err(e)
            }
            // Report every bad entry, not just the first: "which files survived"
            // is the question being asked of a damaged archive.
            Err(e) => bad_entries.push(format!("{}: {e}", entry.name)),
        }
    }

    if let Some(missing) = &info.missing_volume {
        bad_entries.push(format!("{}: volume missing", missing.display()));
    }
    Ok(TestReport {
        ok: bad_entries.is_empty(),
        entries_tested: tested,
        bad_entries,
    })
}
