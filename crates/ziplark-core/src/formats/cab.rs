//! Microsoft Cabinet (`.cab`) archive reader — dependency-free.
//!
//! Parses `CFHEADER`, `CFFOLDER`, `CFFILE` and `CFDATA` structures and unpacks
//! `TYPE_NONE` (uncompressed) and `TYPE_MSZIP` (`CK` + raw Deflate) folders
//! using `flate2`, which is already in the crate graph.

use crate::encoding::NameDecoder;
use crate::error::{Error, Result};
use crate::formats::{copy_watched, create_file, ensure_parent, report, DestGuard};
use crate::model::*;
use crate::{ExtractOptions, ListOptions, ProgressFn, ReadOptions};
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;

pub const CAB_MAGIC: &[u8; 4] = b"MSCF";

const FLAG_PREV_CABINET: u16 = 0x0001;
const FLAG_NEXT_CABINET: u16 = 0x0002;
const FLAG_RESERVE_PRESENT: u16 = 0x0004;

const COMP_MASK: u16 = 0x000F;
const COMP_NONE: u16 = 0;
const COMP_MSZIP: u16 = 1;

#[derive(Clone)]
struct CabFolder {
    data_offset: u32,
    num_blocks: u16,
    comp_type: u16,
}

#[derive(Clone)]
struct CabFile {
    name: String,
    size: u32,
    folder_offset: u32,
    folder_index: u16,
    mtime: Option<i64>,
    readonly: bool,
}

struct CabArchive {
    folders: Vec<CabFolder>,
    files: Vec<CabFile>,
    data_reserve: usize,
}

fn read_cstring<R: Read>(r: &mut R) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    loop {
        r.read_exact(&mut b)
            .map_err(|_| Error::corrupt("CAB: unexpected EOF reading string"))?;
        if b[0] == 0 {
            break;
        }
        out.push(b[0]);
        if out.len() > 4096 {
            return Err(Error::corrupt("CAB: string exceeds maximum length"));
        }
    }
    Ok(out)
}

fn dos_to_unix(date: u16, time: u16) -> Option<i64> {
    if date == 0 {
        return None;
    }
    let year = ((date >> 9) & 0x7F) as i32 + 1980;
    let month = ((date >> 5) & 0x0F) as u8;
    let day = (date & 0x1F) as u8;
    let hour = ((time >> 11) & 0x1F) as u8;
    let min = ((time >> 5) & 0x3F) as u8;
    let sec = ((time & 0x1F) * 2) as u8;
    let m = time::Month::try_from(month).ok()?;
    let d = time::Date::from_calendar_date(year, m, day).ok()?;
    let t = time::Time::from_hms(hour, min, sec).ok()?;
    Some(time::PrimitiveDateTime::new(d, t).assume_utc().unix_timestamp())
}

fn parse_cab(f: &mut File) -> Result<CabArchive> {
    f.seek(SeekFrom::Start(0))?;
    let mut hdr = [0u8; 36];
    f.read_exact(&mut hdr)
        .map_err(|_| Error::corrupt("CAB: truncated header"))?;
    if &hdr[0..4] != CAB_MAGIC {
        return Err(Error::corrupt("CAB: invalid MSCF signature"));
    }

    let coff_files = u32::from_le_bytes([hdr[16], hdr[17], hdr[18], hdr[19]]);
    let num_folders = u16::from_le_bytes([hdr[26], hdr[27]]) as usize;
    let num_files = u16::from_le_bytes([hdr[28], hdr[29]]) as usize;
    let flags = u16::from_le_bytes([hdr[30], hdr[31]]);

    let mut folder_reserve = 0usize;
    let mut data_reserve = 0usize;
    if flags & FLAG_RESERVE_PRESENT != 0 {
        let mut res = [0u8; 4];
        f.read_exact(&mut res)
            .map_err(|_| Error::corrupt("CAB: truncated reserve header"))?;
        let header_reserve = u16::from_le_bytes([res[0], res[1]]) as i64;
        folder_reserve = res[2] as usize;
        data_reserve = res[3] as usize;
        if header_reserve > 0 {
            f.seek(SeekFrom::Current(header_reserve))?;
        }
    }
    if flags & FLAG_PREV_CABINET != 0 {
        let _ = read_cstring(f)?;
        let _ = read_cstring(f)?;
    }
    if flags & FLAG_NEXT_CABINET != 0 {
        let _ = read_cstring(f)?;
        let _ = read_cstring(f)?;
    }

    let mut folders = Vec::with_capacity(num_folders);
    for _ in 0..num_folders {
        let mut fhdr = [0u8; 8];
        f.read_exact(&mut fhdr)
            .map_err(|_| Error::corrupt("CAB: truncated folder header"))?;
        if folder_reserve > 0 {
            f.seek(SeekFrom::Current(folder_reserve as i64))?;
        }
        folders.push(CabFolder {
            data_offset: u32::from_le_bytes([fhdr[0], fhdr[1], fhdr[2], fhdr[3]]),
            num_blocks: u16::from_le_bytes([fhdr[4], fhdr[5]]),
            comp_type: u16::from_le_bytes([fhdr[6], fhdr[7]]),
        });
    }

    f.seek(SeekFrom::Start(coff_files as u64))?;
    let mut raw_files = Vec::with_capacity(num_files);
    let mut decoder = NameDecoder::new();
    for _ in 0..num_files {
        let mut ff = [0u8; 16];
        f.read_exact(&mut ff)
            .map_err(|_| Error::corrupt("CAB: truncated file entry"))?;
        let size = u32::from_le_bytes([ff[0], ff[1], ff[2], ff[3]]);
        let folder_offset = u32::from_le_bytes([ff[4], ff[5], ff[6], ff[7]]);
        let folder_index = u16::from_le_bytes([ff[8], ff[9]]);
        let date = u16::from_le_bytes([ff[10], ff[11]]);
        let time = u16::from_le_bytes([ff[12], ff[13]]);
        let attribs = u16::from_le_bytes([ff[14], ff[15]]);
        let name_bytes = read_cstring(f)?;
        if attribs & 0x80 == 0 {
            decoder.sample(&name_bytes);
        }
        raw_files.push((size, folder_offset, folder_index, date, time, attribs, name_bytes));
    }

    let mut files = Vec::with_capacity(num_files);
    for (size, folder_offset, folder_index, date, time, attribs, name_bytes) in raw_files {
        let name = if attribs & 0x80 != 0 {
            String::from_utf8_lossy(&name_bytes).into_owned()
        } else {
            decoder.decode(&name_bytes)
        }
        .replace('\\', "/");
        files.push(CabFile {
            name,
            size,
            folder_offset,
            folder_index,
            mtime: dos_to_unix(date, time),
            readonly: (attribs & 0x01) != 0,
        });
    }

    Ok(CabArchive {
        folders,
        files,
        data_reserve,
    })
}

/// Decompress an entire CAB folder into memory. CAB folders are typically small
/// solid blocks (several megabytes), and multiple `CFFILE` entries slice into
/// the resulting uncompressed buffer by `uoffFolderStart..uoffFolderStart+cbFile`.
fn unpack_folder(f: &mut File, folder: &CabFolder, data_reserve: usize) -> Result<Vec<u8>> {
    f.seek(SeekFrom::Start(folder.data_offset as u64))?;
    let method = folder.comp_type & COMP_MASK;
    let mut out = Vec::new();

    for _ in 0..folder.num_blocks {
        let mut dhdr = [0u8; 8];
        f.read_exact(&mut dhdr)
            .map_err(|_| Error::corrupt("CAB: truncated CFDATA block header"))?;
        let cb_data = u16::from_le_bytes([dhdr[4], dhdr[5]]) as usize;
        let cb_uncomp = u16::from_le_bytes([dhdr[6], dhdr[7]]) as usize;
        if data_reserve > 0 {
            f.seek(SeekFrom::Current(data_reserve as i64))?;
        }
        let mut payload = vec![0u8; cb_data];
        f.read_exact(&mut payload)
            .map_err(|_| Error::corrupt("CAB: truncated CFDATA payload"))?;

        match method {
            COMP_NONE => {
                out.extend_from_slice(&payload);
            }
            COMP_MSZIP => {
                if payload.len() < 2 || &payload[0..2] != b"CK" {
                    return Err(Error::corrupt("CAB: missing MSZIP 'CK' signature"));
                }
                let start_len = out.len();
                let mut dec = flate2::Decompress::new(false);

                // In MSZIP, each CFDATA block is a separate RFC 1951 Deflate stream
                // (`BFINAL=1`), but the 32 KiB LZ77 sliding dictionary window is
                // preserved from the preceding CFDATA block in the same folder.
                // Prime `dec`'s 32 KiB window by feeding the last <= 32 KiB of
                // `out` wrapped in a non-final RFC 1951 stored block (`BTYPE=00`,
                // `BFINAL=0`), then immediately decode `payload[2..]`.
                let dict_len = start_len.min(32_768);
                if dict_len > 0 {
                    let dict = &out[start_len - dict_len..start_len];
                    let len_u16 = dict_len as u16;
                    let nlen_u16 = !len_u16;
                    let mut dict_stream = Vec::with_capacity(5 + dict_len);
                    dict_stream.push(0x00); // BFINAL=0, BTYPE=00 (stored)
                    dict_stream.extend_from_slice(&len_u16.to_le_bytes());
                    dict_stream.extend_from_slice(&nlen_u16.to_le_bytes());
                    dict_stream.extend_from_slice(dict);
                    let mut sink = vec![0u8; dict_len];
                    dec.decompress(&dict_stream, &mut sink, flate2::FlushDecompress::Sync)
                        .map_err(|e| Error::corrupt(format!("CAB MSZIP dictionary prime error: {e}")))?;
                }

                out.resize(start_len + cb_uncomp, 0);
                let before_out = dec.total_out();
                dec.decompress(
                    &payload[2..],
                    &mut out[start_len..start_len + cb_uncomp],
                    flate2::FlushDecompress::Finish,
                )
                .map_err(|e| Error::corrupt(format!("CAB MSZIP error: {e}")))?;
                let written = (dec.total_out() - before_out) as usize;
                out.truncate(start_len + written);
            }
            other => {
                return Err(Error::other(format!(
                    "CAB compression method {other} (LZX/Quantum) is not supported"
                )));
            }
        }
    }

    Ok(out)
}

pub fn list(path: &Path, fmt: Format, _opts: &ListOptions) -> Result<ArchiveInfo> {
    let mut f = File::open(path)?;
    let total_compressed = f.metadata()?.len();
    let cab = parse_cab(&mut f)?;
    let mut entries = Vec::with_capacity(cab.files.len());
    let mut total_size = 0u64;

    for cf in &cab.files {
        total_size += cf.size as u64;
        entries.push(ArchiveEntry {
            path: cf.name.clone(),
            is_dir: false,
            size: cf.size as u64,
            compressed_size: None,
            encrypted: false,
            modified: cf.mtime,
            crc32: None,
            split: cf.folder_index >= 0xFFFD,
        });
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
        attributes: ArchiveAttributes {
            solid: cab.files.len() > cab.folders.len(),
            ..ArchiveAttributes::default()
        },
    })
}

pub fn extract(path: &Path, opts: &ExtractOptions, progress: ProgressFn) -> Result<ExtractReport> {
    let mut f = File::open(path)?;
    let cab = parse_cab(&mut f)?;
    std::fs::create_dir_all(&opts.dest)?;
    let selector = opts.selector();
    let mut guard = DestGuard::new(&opts.dest);

    let total = cab.files.len() as u64;
    let bytes_total: u64 = cab.files.iter().map(|c| c.size as u64).sum();
    let mut report_out = ExtractReport {
        files_written: 0,
        dirs_created: 0,
        bytes_written: 0,
        dest: opts.dest.clone(),
        failed: Vec::new(),
        partial: Vec::new(),
    };

    let mut cached_folder: Option<(usize, Vec<u8>)> = None;

    for (i, cf) in cab.files.iter().enumerate() {
        if !selector.matches(&cf.name) {
            continue;
        }
        let folder_idx = cf.folder_index as usize;
        if folder_idx >= cab.folders.len() {
            return Err(Error::corrupt(format!(
                "CAB entry '{}' spans another cabinet or references invalid folder",
                cf.name
            )));
        }
        let need_unpack = match &cached_folder {
            Some((idx, _)) => *idx != folder_idx,
            None => true,
        };
        if need_unpack {
            let data = unpack_folder(&mut f, &cab.folders[folder_idx], cab.data_reserve)?;
            cached_folder = Some((folder_idx, data));
        }
        let (_, folder_buf) = cached_folder.as_ref().unwrap();
        let start = cf.folder_offset as usize;
        let end = start.saturating_add(cf.size as usize);
        if end > folder_buf.len() {
            return Err(Error::corrupt(format!(
                "CAB entry '{}' exceeds unpacked folder size",
                cf.name
            )));
        }

        let out_path = guard.join(&cf.name)?;
        ensure_parent(&out_path)?;
        let mut out = create_file(&out_path, opts.overwrite)?;
        let mut slice = Cursor::new(&folder_buf[start..end]);
        let done_before = report_out.bytes_written;
        let n = copy_watched(&mut slice, &mut out, |so_far| {
            progress(Progress {
                current_path: cf.name.clone(),
                entries_done: i as u64 + 1,
                entries_total: total,
                bytes_done: done_before + so_far,
                bytes_total,
            })
        })?;
        drop(out);
        restore_metadata(&out_path, cf.readonly, cf.mtime);
        report_out.files_written += 1;
        report_out.bytes_written += n;
    }

    Ok(report_out)
}

pub fn test(path: &Path, _opts: &ListOptions, progress: ProgressFn) -> Result<TestReport> {
    let mut f = File::open(path)?;
    let cab = parse_cab(&mut f)?;
    let mut bad = Vec::new();
    let mut tested = 0u64;
    let total = cab.files.len() as u64;
    let mut cached_folder: Option<(usize, Result<Vec<u8>>)> = None;

    for (i, cf) in cab.files.iter().enumerate() {
        tested += 1;
        let folder_idx = cf.folder_index as usize;
        if folder_idx >= cab.folders.len() {
            bad.push(format!("{}: invalid folder index", cf.name));
            continue;
        }
        let need_unpack = match &cached_folder {
            Some((idx, _)) => *idx != folder_idx,
            None => true,
        };
        if need_unpack {
            let res = unpack_folder(&mut f, &cab.folders[folder_idx], cab.data_reserve);
            cached_folder = Some((folder_idx, res));
        }
        match &cached_folder.as_ref().unwrap().1 {
            Ok(buf) => {
                let start = cf.folder_offset as usize;
                let end = start.saturating_add(cf.size as usize);
                if end > buf.len() {
                    bad.push(format!("{}: truncated folder data", cf.name));
                }
            }
            Err(e) => bad.push(format!("{}: {e}", cf.name)),
        }
        report(
            progress,
            Progress {
                current_path: cf.name.clone(),
                entries_done: i as u64 + 1,
                entries_total: total,
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

pub fn read_entry(path: &Path, entry_path: &str, opts: &ReadOptions) -> Result<EntryContent> {
    let mut f = File::open(path)?;
    let cab = parse_cab(&mut f)?;
    let target = entry_path.replace('\\', "/");
    let target = target.trim_start_matches("./").trim_matches('/');

    for cf in &cab.files {
        if cf.name.trim_matches('/') == target {
            let folder_idx = cf.folder_index as usize;
            if folder_idx >= cab.folders.len() {
                return Err(Error::corrupt("CAB entry references invalid folder"));
            }
            let buf = unpack_folder(&mut f, &cab.folders[folder_idx], cab.data_reserve)?;
            let start = cf.folder_offset as usize;
            let end = start.saturating_add(cf.size as usize);
            if end > buf.len() {
                return Err(Error::corrupt("CAB entry exceeds folder size"));
            }
            let mut cursor = Cursor::new(&buf[start..end]);
            return crate::formats::read_bounded(&mut cursor, cf.name.clone(), cf.size as u64, opts);
        }
    }
    Err(Error::other(format!("entry not found in archive: {entry_path}")))
}

fn restore_metadata(path: &Path, readonly: bool, mtime: Option<i64>) {
    if readonly {
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_readonly(true);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
    if let Some(secs) = mtime {
        let t = filetime::FileTime::from_unix_time(secs, 0);
        let _ = filetime::set_file_times(path, t, t);
    }
}
