//! Unix `ar` archives (`.ar`, `.a`) and Debian `.deb` packages.
//!
//! An `ar` archive begins with the 8-byte magic `!<arch>\n` followed by 2-byte
//! aligned members, each preceded by a 60-byte ASCII header. Both GNU/SysV
//! (`//` string table + `/offset` references) and BSD (`#1/<len>` inline names)
//! extended filenames are supported.
//!
//! A `.deb` (or `.ipk` / `.udeb`) package is an `ar` archive containing
//! `debian-binary`, `control.tar.*` and `data.tar.*`. Opening a `.deb` as
//! [`Format::Deb`] transparently traverses inside `data.tar.*` (and places
//! `control.tar.*` under `DEBIAN/`, matching `dpkg-deb -R`), so the user sees
//! and extracts the actual package contents rather than three inner tarballs.

use crate::encoding::NameDecoder;
use crate::error::{Error, Result};
use crate::formats::{
    copy_watched, create_file, ensure_parent, prepare_dir, prepare_leaf, report, unpack_link,
    DestGuard,
};
use crate::model::*;
use crate::{ExtractOptions, ListOptions, ProgressFn, ReadOptions};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

pub const AR_MAGIC: &[u8; 8] = b"!<arch>\n";

struct ArMember {
    name: String,
    mtime: Option<i64>,
    mode: Option<u32>,
    data_offset: u64,
    size: u64,
}

fn parse_ar_members(f: &mut File) -> Result<Vec<ArMember>> {
    let file_len = f.metadata()?.len();
    f.seek(SeekFrom::Start(0))?;
    let mut magic = [0u8; 8];
    f.read_exact(&mut magic)
        .map_err(|_| Error::corrupt("ar archive is truncated"))?;
    if &magic != AR_MAGIC {
        return Err(Error::corrupt("invalid ar magic"));
    }

    let mut gnu_names: Vec<u8> = Vec::new();
    let mut members = Vec::new();
    let mut pos = 8u64;

    while pos + 60 <= file_len {
        f.seek(SeekFrom::Start(pos))?;
        let mut hdr = [0u8; 60];
        if f.read_exact(&mut hdr).is_err() {
            break;
        }
        if &hdr[58..60] != b"`\n" {
            return Err(Error::corrupt("invalid ar member header magic"));
        }

        let raw_name = std::str::from_utf8(&hdr[0..16])
            .unwrap_or("")
            .trim_end_matches(' ');
        let mtime = std::str::from_utf8(&hdr[16..28])
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok());
        let mode = std::str::from_utf8(&hdr[40..48])
            .ok()
            .and_then(|s| u32::from_str_radix(s.trim(), 8).ok());
        let size = std::str::from_utf8(&hdr[48..58])
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .ok_or_else(|| Error::corrupt("invalid ar member size"))?;

        let mut data_offset = pos + 60;
        let mut data_size = size;
        pos = data_offset.saturating_add(size);
        if pos % 2 == 1 {
            pos += 1; // 2-byte alignment padding
        }
        if data_offset + data_size > file_len {
            return Err(Error::corrupt("ar member extends past end of file"));
        }

        // GNU extended filename table
        if raw_name == "//" {
            gnu_names.resize(data_size as usize, 0);
            f.seek(SeekFrom::Start(data_offset))?;
            f.read_exact(&mut gnu_names)?;
            continue;
        }
        // GNU / SysV symbol tables
        if raw_name == "/" || raw_name == "/SYM64/" {
            continue;
        }

        let name = if let Some(rest) = raw_name.strip_prefix("#1/") {
            let name_len = rest
                .trim()
                .parse::<usize>()
                .map_err(|_| Error::corrupt("invalid BSD ar extended name length"))?;
            if (name_len as u64) > data_size {
                return Err(Error::corrupt("BSD ar name length exceeds member size"));
            }
            let mut nbuf = vec![0u8; name_len];
            f.seek(SeekFrom::Start(data_offset))?;
            f.read_exact(&mut nbuf)?;
            data_offset += name_len as u64;
            data_size -= name_len as u64;
            let s = String::from_utf8_lossy(&nbuf)
                .trim_end_matches('\0')
                .to_string();
            s
        } else if let Some(off_str) = raw_name.strip_prefix('/') {
            if let Ok(off) = off_str.trim().parse::<usize>() {
                if off < gnu_names.len() {
                    let tail = &gnu_names[off..];
                    let end = tail
                        .iter()
                        .position(|&b| b == b'\n' || b == b'\0')
                        .unwrap_or(tail.len());
                    String::from_utf8_lossy(&tail[..end])
                        .trim_end_matches('\r')
                        .trim_end_matches('/')
                        .to_string()
                } else {
                    raw_name.to_string()
                }
            } else {
                raw_name.to_string()
            }
        } else {
            raw_name.trim_end_matches('/').to_string()
        };

        if name.is_empty() || name.starts_with("__.SYMDEF") {
            continue;
        }

        members.push(ArMember {
            name,
            mtime,
            mode,
            data_offset,
            size: data_size,
        });
    }

    Ok(members)
}

/// Check if an `ar` member inside a `.deb` is `data.tar*` or `control.tar*`.
/// Returns `Some(prefix)` where `prefix` is `""` for `data.tar*` and `"DEBIAN"`
/// for `control.tar*`.
fn deb_tar_prefix(member_name: &str) -> Option<&'static str> {
    if member_name == "data.tar" || member_name.starts_with("data.tar.") {
        Some("")
    } else if member_name == "control.tar" || member_name.starts_with("control.tar.") {
        Some("DEBIAN")
    } else {
        None
    }
}

fn open_tar_decoder<'a, R: Read + 'a>(member_name: &str, reader: R) -> Result<Box<dyn Read + 'a>> {
    Ok(if member_name.ends_with(".gz") {
        Box::new(flate2::read::GzDecoder::new(reader))
    } else if member_name.ends_with(".xz") || member_name.ends_with(".lzma") {
        Box::new(xz2::read::XzDecoder::new(reader))
    } else if member_name.ends_with(".zst") {
        Box::new(zstd::stream::read::Decoder::new(reader)?)
    } else if member_name.ends_with(".bz2") {
        Box::new(bzip2::read::BzDecoder::new(reader))
    } else if member_name.ends_with(".lz4") {
        Box::new(lz4_flex::frame::FrameDecoder::new(reader))
    } else {
        Box::new(reader)
    })
}

fn clean_deb_entry_path(prefix: &str, raw_path: &str) -> String {
    let trimmed = raw_path
        .trim_start_matches("./")
        .trim_start_matches('/')
        .trim_end_matches('/');
    if trimmed.is_empty() {
        return prefix.to_string();
    }
    if prefix.is_empty() {
        trimmed.to_string()
    } else {
        format!("{prefix}/{trimmed}")
    }
}

pub fn list(path: &Path, fmt: Format, _opts: &ListOptions) -> Result<ArchiveInfo> {
    let mut f = File::open(path)?;
    let total_compressed = f.metadata()?.len();
    let members = parse_ar_members(&mut f)?;

    let is_deb = fmt == Format::Deb
        && members
            .iter()
            .any(|m| deb_tar_prefix(&m.name).is_some());

    let mut entries = Vec::new();
    let mut total_size = 0u64;

    if is_deb {
        for m in &members {
            let Some(prefix) = deb_tar_prefix(&m.name) else {
                continue;
            };
            f.seek(SeekFrom::Start(m.data_offset))?;
            let slice = (&mut f).take(m.size);
            let dec = open_tar_decoder(&m.name, slice)?;
            let mut tar = tar::Archive::new(dec);
            let mut names = NameDecoder::new();
            for entry in tar.entries()? {
                let entry = entry?;
                let is_dir = entry.header().entry_type().is_dir();
                let size = entry.size();
                let raw = entry.path_bytes().into_owned();
                names.sample(&raw);
                let decoded = names.decode(&raw);
                let full_path = clean_deb_entry_path(prefix, &decoded);
                if full_path.is_empty() {
                    continue;
                }
                total_size += size;
                entries.push(ArchiveEntry {
                    path: full_path,
                    is_dir,
                    size,
                    compressed_size: None,
                    encrypted: false,
                    modified: entry.header().mtime().ok().map(|t| t as i64),
                    crc32: None,
                    split: false,
                });
            }
        }
    } else {
        for m in &members {
            total_size += m.size;
            entries.push(ArchiveEntry {
                path: m.name.clone(),
                is_dir: false,
                size: m.size,
                compressed_size: Some(m.size),
                encrypted: false,
                modified: m.mtime,
                crc32: None,
                split: false,
            });
        }
    }

    Ok(ArchiveInfo {
        format: fmt,
        path: path.to_path_buf(),
        entries,
        encrypted: false,
        total_size,
        total_compressed,
        volumes: Vec::new(),
        missing_volume: None,
        comment: None,
        attributes: ArchiveAttributes::default(),
    })
}

pub fn extract(
    path: &Path,
    fmt: Format,
    opts: &ExtractOptions,
    progress: ProgressFn,
) -> Result<ExtractReport> {
    let mut f = File::open(path)?;
    let members = parse_ar_members(&mut f)?;
    std::fs::create_dir_all(&opts.dest)?;
    let selector = opts.selector();
    let mut guard = DestGuard::new(&opts.dest);
    let mut report_out = ExtractReport {
        files_written: 0,
        dirs_created: 0,
        bytes_written: 0,
        dest: opts.dest.clone(),
        failed: Vec::new(),
        partial: Vec::new(),
    };

    let is_deb = fmt == Format::Deb
        && members
            .iter()
            .any(|m| deb_tar_prefix(&m.name).is_some());

    if is_deb {
        let mut idx = 0u64;
        for m in &members {
            let Some(prefix) = deb_tar_prefix(&m.name) else {
                continue;
            };
            f.seek(SeekFrom::Start(m.data_offset))?;
            let slice = (&mut f).take(m.size);
            let dec = open_tar_decoder(&m.name, slice)?;
            let mut tar = tar::Archive::new(dec);
            let mut names = NameDecoder::new();
            for entry in tar.entries()? {
                let mut entry = entry?;
                let raw = entry.path_bytes().into_owned();
                names.sample(&raw);
                let decoded = names.decode(&raw);
                let name = clean_deb_entry_path(prefix, &decoded);
                if name.is_empty() || !selector.matches(&name) {
                    continue;
                }
                idx += 1;
                let out_path = guard.join(&name)?;
                let kind = entry.header().entry_type();
                if kind.is_dir() {
                    prepare_dir(&out_path, &name)?;
                    report_out.dirs_created += 1;
                    continue;
                }
                ensure_parent(&out_path)?;
                if kind.is_symlink() || kind.is_hard_link() {
                    let target_raw = entry
                        .link_name_bytes()
                        .ok_or_else(|| Error::corrupt(format!("{name}: link with no target")))?
                        .into_owned();
                    let target = names.decode(&target_raw);
                    let target_clean = if kind.is_hard_link() {
                        clean_deb_entry_path(prefix, &target)
                    } else {
                        target
                    };
                    unpack_link(
                        &mut guard,
                        kind.is_symlink(),
                        &target_clean,
                        &out_path,
                        opts.overwrite,
                    )?;
                    report_out.files_written += 1;
                    continue;
                }
                prepare_leaf(&out_path, opts.overwrite)?;
                entry.unpack(&out_path)?;
                report_out.files_written += 1;
                report_out.bytes_written += entry.size();
                report(
                    progress,
                    Progress {
                        current_path: name,
                        entries_done: idx,
                        entries_total: 0,
                        bytes_done: report_out.bytes_written,
                        bytes_total: 0,
                    },
                )?;
            }
        }
    } else {
        let total = members.len() as u64;
        for (i, m) in members.iter().enumerate() {
            if !selector.matches(&m.name) {
                continue;
            }
            let out_path = guard.join(&m.name)?;
            ensure_parent(&out_path)?;
            let mut out = create_file(&out_path, opts.overwrite)?;
            f.seek(SeekFrom::Start(m.data_offset))?;
            let mut reader = (&mut f).take(m.size);
            let done_before = report_out.bytes_written;
            let n = copy_watched(&mut reader, &mut out, |so_far| {
                progress(Progress {
                    current_path: m.name.clone(),
                    entries_done: i as u64 + 1,
                    entries_total: total,
                    bytes_done: done_before + so_far,
                    bytes_total: 0,
                })
            })?;
            drop(out);
            restore_metadata(&out_path, m.mode, m.mtime);
            report_out.files_written += 1;
            report_out.bytes_written += n;
        }
    }

    Ok(report_out)
}

pub fn test(
    path: &Path,
    fmt: Format,
    _opts: &ListOptions,
    progress: ProgressFn,
) -> Result<TestReport> {
    let mut f = File::open(path)?;
    let members = parse_ar_members(&mut f)?;
    let mut tested = 0u64;
    let mut bad = Vec::new();
    let mut sink = io::sink();

    let is_deb = fmt == Format::Deb
        && members
            .iter()
            .any(|m| deb_tar_prefix(&m.name).is_some());

    if is_deb {
        for m in &members {
            let Some(prefix) = deb_tar_prefix(&m.name) else {
                continue;
            };
            f.seek(SeekFrom::Start(m.data_offset))?;
            let slice = (&mut f).take(m.size);
            let dec = match open_tar_decoder(&m.name, slice) {
                Ok(d) => d,
                Err(e) => {
                    bad.push(format!("{}: {e}", m.name));
                    continue;
                }
            };
            let mut tar = tar::Archive::new(dec);
            match tar.entries() {
                Ok(entries) => {
                    for entry in entries {
                        match entry {
                            Ok(mut e) => {
                                if e.header().entry_type().is_dir() {
                                    continue;
                                }
                                tested += 1;
                                let raw = e.path().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                                let name = clean_deb_entry_path(prefix, &raw);
                                if let Err(err) = io::copy(&mut e, &mut sink) {
                                    bad.push(format!("{name}: {err}"));
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
                            Err(err) => bad.push(format!("{}: {err}", m.name)),
                        }
                    }
                }
                Err(err) => bad.push(format!("{}: {err}", m.name)),
            }
        }
    } else {
        let total = members.len() as u64;
        for (i, m) in members.iter().enumerate() {
            tested += 1;
            f.seek(SeekFrom::Start(m.data_offset))?;
            let mut slice = (&mut f).take(m.size);
            if let Err(e) = io::copy(&mut slice, &mut sink) {
                bad.push(format!("{}: {e}", m.name));
            }
            report(
                progress,
                Progress {
                    current_path: m.name.clone(),
                    entries_done: i as u64 + 1,
                    entries_total: total,
                    bytes_done: 0,
                    bytes_total: 0,
                },
            )?;
        }
    }

    Ok(TestReport {
        ok: bad.is_empty(),
        entries_tested: tested,
        bad_entries: bad,
    })
}

pub fn read_entry(
    path: &Path,
    fmt: Format,
    entry_path: &str,
    opts: &ReadOptions,
) -> Result<EntryContent> {
    let mut f = File::open(path)?;
    let members = parse_ar_members(&mut f)?;
    let target = entry_path.trim_start_matches("./").trim_matches('/');

    let is_deb = fmt == Format::Deb
        && members
            .iter()
            .any(|m| deb_tar_prefix(&m.name).is_some());

    if is_deb {
        for m in &members {
            let Some(prefix) = deb_tar_prefix(&m.name) else {
                continue;
            };
            f.seek(SeekFrom::Start(m.data_offset))?;
            let slice = (&mut f).take(m.size);
            let dec = open_tar_decoder(&m.name, slice)?;
            let mut tar = tar::Archive::new(dec);
            let mut names = NameDecoder::new();
            for entry in tar.entries()? {
                let mut entry = entry?;
                let raw = entry.path_bytes().into_owned();
                names.sample(&raw);
                let decoded = names.decode(&raw);
                let name = clean_deb_entry_path(prefix, &decoded);
                if name == target {
                    let size = entry.size();
                    return crate::formats::read_bounded(&mut entry, name, size, opts);
                }
            }
        }
    } else {
        for m in &members {
            if m.name.trim_matches('/') == target {
                f.seek(SeekFrom::Start(m.data_offset))?;
                let mut slice = (&mut f).take(m.size);
                return crate::formats::read_bounded(&mut slice, m.name.clone(), m.size, opts);
            }
        }
    }

    Err(Error::other(format!("entry not found in archive: {entry_path}")))
}

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
