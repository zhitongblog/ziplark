//! ISO 9660 / Joliet disc-image extraction — a small, dependency-free reader.
//!
//! ISO 9660 stores files uncompressed and sector-addressed, so reading it is
//! just: parse the volume descriptor → walk the directory records → seek to
//! `LBA * 2048` and copy `size` bytes. We prefer the Joliet supplementary
//! descriptor when present (Unicode / long names) and fall back to the plain
//! primary descriptor (upper-case 8.3 names). Extraction funnels through
//! `safe_join`; `.`/`..` records are skipped and recursion depth is bounded.
//!
//! Disc images are containers, not a compression format, so there is no
//! `create` (`Format::Iso.can_create()` is false).

use crate::error::{Error, Result};
use crate::formats::{ensure_parent, safe_join};
use crate::model::*;
use crate::{ExtractOptions, ListOptions, ProgressFn};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

const SECTOR: u64 = 2048;
const MAX_DEPTH: u32 = 64;

/// A single directory record we care about.
struct Record {
    /// LBA where the entry's data starts (extent location + any EAR blocks).
    lba: u32,
    size: u32,
    is_dir: bool,
    name: String,
}

fn corrupt(msg: impl std::fmt::Display) -> Error {
    Error::corrupt(format!("ISO: {msg}"))
}

/// Scan the volume descriptors (sector 16+) and return the root directory's
/// (lba, size) and whether names are Joliet (UTF-16BE).
fn read_volume(f: &mut File) -> Result<(u32, u32, bool)> {
    let mut pvd = None;
    let mut joliet = None;
    let mut sec = [0u8; SECTOR as usize];
    for lba in 16u64..64 {
        f.seek(SeekFrom::Start(lba * SECTOR))?;
        if f.read_exact(&mut sec).is_err() {
            break;
        }
        if &sec[1..6] != b"CD001" {
            break;
        }
        match sec[0] {
            1 => pvd = Some(root_record(&sec[156..190])), // Primary
            2 => {
                // Supplementary descriptor; Joliet advertises UCS-2 via an
                // escape sequence in bytes 88..120 ("%/@", "%/C" or "%/E").
                let esc = &sec[88..120];
                let is_joliet = esc
                    .windows(3)
                    .any(|w| w == b"%/@" || w == b"%/C" || w == b"%/E");
                if is_joliet {
                    joliet = Some(root_record(&sec[156..190]));
                }
            }
            255 => break, // terminator
            _ => {}
        }
    }
    if let Some((l, s)) = joliet {
        Ok((l, s, true))
    } else if let Some((l, s)) = pvd {
        Ok((l, s, false))
    } else {
        Err(corrupt("no primary volume descriptor"))
    }
}

/// Parse a 34-byte directory record header into (data_lba, data_len).
fn root_record(r: &[u8]) -> (u32, u32) {
    let ext_attr = r[1] as u32;
    let loc = u32::from_le_bytes([r[2], r[3], r[4], r[5]]);
    let len = u32::from_le_bytes([r[10], r[11], r[12], r[13]]);
    (loc + ext_attr, len)
}

/// Read and parse one directory extent into its child records (skipping the
/// `.` and `..` self/parent entries).
fn read_dir(f: &mut File, lba: u32, size: u32, joliet: bool) -> Result<Vec<Record>> {
    if size == 0 {
        return Ok(Vec::new());
    }
    let mut buf = vec![0u8; size as usize];
    f.seek(SeekFrom::Start(lba as u64 * SECTOR))?;
    f.read_exact(&mut buf).map_err(corrupt)?;

    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < buf.len() {
        let len_dr = buf[pos] as usize;
        if len_dr == 0 {
            // Records never cross a sector boundary; jump to the next sector.
            let next = (pos / SECTOR as usize + 1) * SECTOR as usize;
            if next >= buf.len() {
                break;
            }
            pos = next;
            continue;
        }
        if len_dr < 34 || pos + len_dr > buf.len() {
            break; // malformed
        }
        let rec = &buf[pos..pos + len_dr];
        let ext_attr = rec[1] as u32;
        let loc = u32::from_le_bytes([rec[2], rec[3], rec[4], rec[5]]);
        let dlen = u32::from_le_bytes([rec[10], rec[11], rec[12], rec[13]]);
        let is_dir = rec[25] & 0x02 != 0;
        let len_fi = rec[32] as usize;
        if 33 + len_fi <= len_dr {
            let id = &rec[33..33 + len_fi];
            // Skip "." (0x00) and ".." (0x01).
            let is_special = len_fi == 1 && (id[0] == 0 || id[0] == 1);
            if !is_special {
                let name = decode_name(id, joliet);
                if !name.is_empty() {
                    out.push(Record {
                        lba: loc + ext_attr,
                        size: dlen,
                        is_dir,
                        name,
                    });
                }
            }
        }
        pos += len_dr;
    }
    Ok(out)
}

/// Decode a file identifier and strip the trailing `;version` (and any dangling
/// `.` left on extension-less names).
fn decode_name(id: &[u8], joliet: bool) -> String {
    let raw = if joliet {
        let mut s = String::with_capacity(id.len() / 2);
        let mut i = 0;
        while i + 1 < id.len() {
            let u = u16::from_be_bytes([id[i], id[i + 1]]);
            s.push(char::from_u32(u as u32).unwrap_or('\u{FFFD}'));
            i += 2;
        }
        s
    } else {
        String::from_utf8_lossy(id).into_owned()
    };
    let base = match raw.rsplit_once(';') {
        Some((b, v)) if !b.is_empty() && v.chars().all(|c| c.is_ascii_digit()) => b,
        _ => &raw,
    };
    base.trim_end_matches('.').to_string()
}

fn child_rel(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

fn matches_filter(name: &str, include: &[String]) -> bool {
    include.is_empty() || include.iter().any(|p| name.contains(p.as_str()))
}

// ───────────────────────────── list ─────────────────────────────

pub fn list(path: &Path, fmt: Format, _opts: &ListOptions) -> Result<ArchiveInfo> {
    let mut f = File::open(path)?;
    let (lba, size, joliet) = read_volume(&mut f)?;
    let mut entries = Vec::new();
    let mut total = 0u64;
    list_dir(&mut f, lba, size, joliet, "", &mut entries, &mut total, 0)?;
    let total_compressed = std::fs::metadata(path)?.len();
    Ok(ArchiveInfo {
        format: fmt,
        path: path.to_path_buf(),
        entries,
        encrypted: false,
        total_size: total,
        total_compressed,
    })
}

#[allow(clippy::too_many_arguments)]
fn list_dir(
    f: &mut File,
    lba: u32,
    size: u32,
    joliet: bool,
    prefix: &str,
    out: &mut Vec<ArchiveEntry>,
    total: &mut u64,
    depth: u32,
) -> Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    for rec in read_dir(f, lba, size, joliet)? {
        let rel = child_rel(prefix, &rec.name);
        if rec.is_dir {
            out.push(entry(rel.clone(), true, 0));
            list_dir(f, rec.lba, rec.size, joliet, &rel, out, total, depth + 1)?;
        } else {
            *total += rec.size as u64;
            out.push(entry(rel, false, rec.size as u64));
        }
    }
    Ok(())
}

fn entry(path: String, is_dir: bool, size: u64) -> ArchiveEntry {
    ArchiveEntry {
        path,
        is_dir,
        size,
        compressed_size: None,
        encrypted: false,
        modified: None,
        crc32: None,
    }
}

// ───────────────────────────── extract ─────────────────────────────

pub fn extract(path: &Path, opts: &ExtractOptions, progress: ProgressFn) -> Result<ExtractReport> {
    let mut f = File::open(path)?;
    let (lba, size, joliet) = read_volume(&mut f)?;
    std::fs::create_dir_all(&opts.dest)?;
    let mut report = ExtractReport {
        files_written: 0,
        dirs_created: 0,
        bytes_written: 0,
        dest: opts.dest.clone(),
    };
    let mut idx = 0u64;
    extract_dir(&mut f, lba, size, joliet, "", opts, &mut report, &mut idx, progress, 0)?;
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn extract_dir(
    f: &mut File,
    lba: u32,
    size: u32,
    joliet: bool,
    prefix: &str,
    opts: &ExtractOptions,
    report: &mut ExtractReport,
    idx: &mut u64,
    progress: ProgressFn,
    depth: u32,
) -> Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    for rec in read_dir(f, lba, size, joliet)? {
        let rel = child_rel(prefix, &rec.name);
        if rec.is_dir {
            let out = safe_join(&opts.dest, &rel)?;
            std::fs::create_dir_all(&out)?;
            report.dirs_created += 1;
            extract_dir(f, rec.lba, rec.size, joliet, &rel, opts, report, idx, progress, depth + 1)?;
            continue;
        }
        if !matches_filter(&rel, &opts.include) {
            continue;
        }
        let out = safe_join(&opts.dest, &rel)?;
        ensure_parent(&out)?;
        if out.exists() && !opts.overwrite {
            return Err(Error::other(format!(
                "{} already exists (use overwrite)",
                out.display()
            )));
        }
        f.seek(SeekFrom::Start(rec.lba as u64 * SECTOR))?;
        let mut w = File::create(&out)?;
        let n = io::copy(&mut f.take(rec.size as u64), &mut w).map_err(corrupt)?;
        report.files_written += 1;
        report.bytes_written += n;
        *idx += 1;
        progress(Progress {
            current_path: rel,
            entries_done: *idx,
            entries_total: 0,
            bytes_done: report.bytes_written,
            bytes_total: 0,
        });
    }
    Ok(())
}

// ───────────────────────────── test ─────────────────────────────

pub fn test(path: &Path, _opts: &ListOptions, progress: ProgressFn) -> Result<TestReport> {
    let mut f = File::open(path)?;
    let (lba, size, joliet) = read_volume(&mut f)?;
    let mut tested = 0u64;
    let mut bad = Vec::new();
    test_dir(&mut f, lba, size, joliet, "", &mut tested, &mut bad, progress, 0)?;
    Ok(TestReport {
        ok: bad.is_empty(),
        entries_tested: tested,
        bad_entries: bad,
    })
}

#[allow(clippy::too_many_arguments)]
fn test_dir(
    f: &mut File,
    lba: u32,
    size: u32,
    joliet: bool,
    prefix: &str,
    tested: &mut u64,
    bad: &mut Vec<String>,
    progress: ProgressFn,
    depth: u32,
) -> Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    for rec in read_dir(f, lba, size, joliet)? {
        let rel = child_rel(prefix, &rec.name);
        if rec.is_dir {
            test_dir(f, rec.lba, rec.size, joliet, &rel, tested, bad, progress, depth + 1)?;
            continue;
        }
        *tested += 1;
        if let Err(e) = f.seek(SeekFrom::Start(rec.lba as u64 * SECTOR)) {
            bad.push(format!("{rel}: {e}"));
            continue;
        }
        if let Err(e) = io::copy(&mut f.take(rec.size as u64), &mut io::sink()) {
            bad.push(format!("{rel}: {e}"));
        }
        progress(Progress {
            current_path: rel,
            entries_done: *tested,
            entries_total: 0,
            bytes_done: 0,
            bytes_total: 0,
        });
    }
    Ok(())
}
