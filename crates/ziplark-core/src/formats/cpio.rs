//! CPIO archives (`.cpio`) and Red Hat `.rpm` packages.
//!
//! Supports SVR4 ASCII `newc` (`070701`), `crc` (`070702`) and POSIX `odc`
//! (`070707`) CPIO streams, whether raw or compressed with gzip, xz, zstd,
//! bzip2 or lz4.
//!
//! An `.rpm` package begins with a 96-byte Lead, a Signature Header (padded to
//! 8 bytes) and a Main Header, followed immediately by a compressed CPIO
//! payload. [`open_payload`] skips the RPM headers and wraps the payload in the
//! matching decompressor so `.rpm` packages can be listed, verified, previewed
//! and extracted without external tools.

use crate::error::{Error, Result};
use crate::formats::{
    copy_watched, create_file, create_symlink, ensure_parent, prepare_dir, prepare_leaf, report,
    DestGuard,
};
use crate::model::*;
use crate::{ExtractOptions, ListOptions, ProgressFn, ReadOptions};
use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;

pub const RPM_MAGIC: &[u8; 4] = b"\xED\xAB\xEE\xDB";
const RPM_HEADER_MAGIC: &[u8; 4] = b"\x8E\xAD\xE8\x01";

const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;

pub(crate) struct CpioHeader {
    pub(crate) name: String,
    pub(crate) mode: u32,
    pub(crate) mtime: i64,
    pub(crate) size: u64,
    pub(crate) data_pad: usize,
}

impl CpioHeader {
    pub(crate) fn is_dir(&self) -> bool {
        (self.mode & S_IFMT) == S_IFDIR
    }
    pub(crate) fn is_symlink(&self) -> bool {
        (self.mode & S_IFMT) == S_IFLNK
    }
}

/// Skip the 96-byte RPM lead + signature header + main header, positioning `f`
/// at the start of the compressed CPIO payload.
fn seek_rpm_payload(f: &mut File) -> Result<()> {
    f.seek(SeekFrom::Start(0))?;
    let mut lead = [0u8; 96];
    f.read_exact(&mut lead)
        .map_err(|_| Error::corrupt("RPM file is truncated (missing 96-byte lead)"))?;
    if &lead[0..4] != RPM_MAGIC {
        return Err(Error::corrupt("invalid RPM lead magic"));
    }

    // 1. Signature Header (padded to 8-byte alignment).
    let sig_len = read_rpm_header_size(f)?;
    let sig_padded = (sig_len + 7) & !7;
    f.seek(SeekFrom::Current(sig_padded as i64))?;

    // 2. Main Header (not padded).
    let main_len = read_rpm_header_size(f)?;
    f.seek(SeekFrom::Current(main_len as i64))?;
    Ok(())
}

fn read_rpm_header_size(f: &mut File) -> Result<u64> {
    let mut intro = [0u8; 16];
    f.read_exact(&mut intro)
        .map_err(|_| Error::corrupt("RPM header is truncated"))?;
    if &intro[0..4] != RPM_HEADER_MAGIC {
        return Err(Error::corrupt("invalid RPM header magic"));
    }
    let nindex = u32::from_be_bytes([intro[8], intro[9], intro[10], intro[11]]) as u64;
    let hsize = u32::from_be_bytes([intro[12], intro[13], intro[14], intro[15]]) as u64;
    Ok(nindex * 16 + hsize)
}

/// Open a CPIO stream from a `.cpio` or `.rpm` file, automatically detecting
/// any stream compression on the payload.
fn open_payload(path: &Path, fmt: Format) -> Result<Box<dyn Read>> {
    let mut f = File::open(path)?;
    if fmt == Format::Rpm {
        seek_rpm_payload(&mut f)?;
    }
    let mut peek = [0u8; 6];
    let n = f.read(&mut peek)?;
    let head = &peek[..n];
    let chained = Cursor::new(head.to_vec()).chain(f);
    wrap_compressed_stream(chained)
}

/// Wrap any reader in the matching stream decompressor (`gzip`, `xz`, `zstd`,
/// `bzip2`, `lz4`) based on its leading magic bytes, or pass it through raw.
pub(crate) fn wrap_compressed_stream<'a, R: Read + 'a>(mut r: R) -> Result<Box<dyn Read + 'a>> {
    let mut peek = [0u8; 6];
    let n = r.read(&mut peek)?;
    let head = &peek[..n];
    let chained = Cursor::new(head.to_vec()).chain(r);

    Ok(if head.starts_with(b"\x1F\x8B") {
        Box::new(flate2::read::GzDecoder::new(chained))
    } else if head.starts_with(b"\xFD7zXZ") {
        Box::new(xz2::read::XzDecoder::new(chained))
    } else if head.starts_with(b"\x28\xB5\x2F\xFD") {
        Box::new(zstd::stream::read::Decoder::new(BufReader::new(chained))?)
    } else if head.starts_with(b"BZh") {
        Box::new(bzip2::read::BzDecoder::new(chained))
    } else if head.starts_with(b"\x04\x22\x4D\x18") {
        Box::new(lz4_flex::frame::FrameDecoder::new(chained))
    } else {
        Box::new(chained)
    })
}

/// Read the next CPIO entry header from `r`, returning `Ok(None)` at EOF or
/// `"TRAILER!!!"`.
pub(crate) fn next_header<R: Read + ?Sized>(r: &mut R) -> Result<Option<CpioHeader>> {
    let mut magic = [0u8; 6];
    if !read_exact_or_eof(r, &mut magic)? {
        return Ok(None);
    }

    let (mode, mtime, size, namesize, name_pad, data_pad) =
        if &magic == b"070701" || &magic == b"070702" {
            let mut rest = [0u8; 104];
            r.read_exact(&mut rest)
                .map_err(|_| Error::corrupt("truncated cpio newc header"))?;
            let mode = hex_u32(&rest[8..16])?;
            let mtime = hex_u32(&rest[40..48])? as i64;
            let filesize = hex_u32(&rest[48..56])? as u64;
            let namesize = hex_u32(&rest[88..96])? as usize;
            let name_pad = (4 - ((110 + namesize) % 4)) % 4;
            let data_pad = ((4 - (filesize % 4)) % 4) as usize;
            (mode, mtime, filesize, namesize, name_pad, data_pad)
        } else if &magic == b"070707" {
            let mut rest = [0u8; 70];
            r.read_exact(&mut rest)
                .map_err(|_| Error::corrupt("truncated cpio odc header"))?;
            let mode = oct_u64(&rest[12..18])? as u32;
            let mtime = oct_u64(&rest[42..53])? as i64;
            let namesize = oct_u64(&rest[53..59])? as usize;
            let filesize = oct_u64(&rest[59..70])?;
            (mode, mtime, filesize, namesize, 0usize, 0usize)
        } else {
            return Err(Error::corrupt("unrecognized cpio header magic"));
        };

    if namesize == 0 || namesize > 65_536 {
        return Err(Error::corrupt("invalid cpio entry name length"));
    }
    let mut name_buf = vec![0u8; namesize];
    r.read_exact(&mut name_buf)
        .map_err(|_| Error::corrupt("truncated cpio entry name"))?;
    if name_pad > 0 {
        let mut pad = [0u8; 4];
        r.read_exact(&mut pad[..name_pad])
            .map_err(|_| Error::corrupt("truncated cpio name padding"))?;
    }

    let raw_name = String::from_utf8_lossy(&name_buf)
        .trim_end_matches('\0')
        .to_string();
    if raw_name == "TRAILER!!!" {
        return Ok(None);
    }

    let clean_name = raw_name
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_string();

    Ok(Some(CpioHeader {
        name: clean_name,
        mode,
        mtime,
        size,
        data_pad,
    }))
}

pub(crate) fn skip_bytes<R: Read + ?Sized>(r: &mut R, n: u64) -> Result<()> {
    let mut take = r.take(n);
    io::copy(&mut take, &mut io::sink())?;
    Ok(())
}

fn read_exact_or_eof<R: Read + ?Sized>(r: &mut R, buf: &mut [u8]) -> Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = r.read(&mut buf[filled..])?;
        if n == 0 {
            if filled == 0 {
                return Ok(false);
            }
            return Err(Error::corrupt("unexpected EOF in cpio header"));
        }
        filled += n;
    }
    Ok(true)
}

fn hex_u32(bytes: &[u8]) -> Result<u32> {
    let s = std::str::from_utf8(bytes).map_err(|_| Error::corrupt("non-ASCII cpio field"))?;
    u32::from_str_radix(s, 16).map_err(|_| Error::corrupt("invalid hex in cpio header"))
}

fn oct_u64(bytes: &[u8]) -> Result<u64> {
    let s = std::str::from_utf8(bytes).map_err(|_| Error::corrupt("non-ASCII cpio field"))?;
    u64::from_str_radix(s, 8).map_err(|_| Error::corrupt("invalid octal in cpio header"))
}

pub fn list(path: &Path, fmt: Format, _opts: &ListOptions) -> Result<ArchiveInfo> {
    let total_compressed = std::fs::metadata(path)?.len();
    let mut r = open_payload(path, fmt)?;
    let mut entries = Vec::new();
    let mut total_size = 0u64;

    while let Some(hdr) = next_header(&mut r)? {
        let skip = hdr.size + hdr.data_pad as u64;
        skip_bytes(&mut r, skip)?;
        if hdr.name.is_empty() || hdr.name == "." {
            continue;
        }
        let is_dir = hdr.is_dir();
        if !is_dir {
            total_size += hdr.size;
        }
        entries.push(ArchiveEntry {
            path: hdr.name,
            is_dir,
            size: if is_dir { 0 } else { hdr.size },
            compressed_size: None,
            encrypted: false,
            modified: (hdr.mtime > 0).then_some(hdr.mtime),
            crc32: None,
            split: false,
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
        attributes: ArchiveAttributes::default(),
    })
}

pub fn extract(
    path: &Path,
    fmt: Format,
    opts: &ExtractOptions,
    progress: ProgressFn,
) -> Result<ExtractReport> {
    let mut r = open_payload(path, fmt)?;
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
    let mut idx = 0u64;

    while let Some(hdr) = next_header(&mut r)? {
        if hdr.name.is_empty() || hdr.name == "." || !selector.matches(&hdr.name) {
            skip_bytes(&mut r, hdr.size + hdr.data_pad as u64)?;
            continue;
        }
        idx += 1;
        let out_path = guard.join(&hdr.name)?;

        if hdr.is_dir() {
            skip_bytes(&mut r, hdr.size + hdr.data_pad as u64)?;
            prepare_dir(&out_path, &hdr.name)?;
            report_out.dirs_created += 1;
            continue;
        }

        ensure_parent(&out_path)?;

        if hdr.is_symlink() {
            let mut target_bytes = vec![0u8; hdr.size as usize];
            r.read_exact(&mut target_bytes)?;
            skip_bytes(&mut r, hdr.data_pad as u64)?;
            let target = String::from_utf8_lossy(&target_bytes)
                .trim_end_matches('\0')
                .to_string();
            prepare_leaf(&out_path, opts.overwrite)?;
            if opts.overwrite {
                let _ = std::fs::remove_file(&out_path);
            }
            create_symlink(&target, &out_path)?;
            guard.forget(&out_path);
            report_out.files_written += 1;
            continue;
        }

        let mut out = create_file(&out_path, opts.overwrite)?;
        let done_before = report_out.bytes_written;
        let mut entry_reader = (&mut r).take(hdr.size);
        let n = copy_watched(&mut entry_reader, &mut out, |so_far| {
            progress(Progress {
                current_path: hdr.name.clone(),
                entries_done: idx,
                entries_total: 0,
                bytes_done: done_before + so_far,
                bytes_total: 0,
            })
        })?;
        drop(out);
        skip_bytes(&mut r, hdr.data_pad as u64)?;
        restore_metadata(&out_path, Some(hdr.mode), Some(hdr.mtime));
        report_out.files_written += 1;
        report_out.bytes_written += n;
    }

    Ok(report_out)
}

pub fn test(
    path: &Path,
    fmt: Format,
    _opts: &ListOptions,
    progress: ProgressFn,
) -> Result<TestReport> {
    let mut r = open_payload(path, fmt)?;
    let mut tested = 0u64;
    let mut bad = Vec::new();
    let mut sink = io::sink();

    loop {
        let hdr = match next_header(&mut r) {
            Ok(Some(h)) => h,
            Ok(None) => break,
            Err(e) => {
                bad.push(e.to_string());
                break;
            }
        };
        if hdr.is_dir() {
            let _ = skip_bytes(&mut r, hdr.size + hdr.data_pad as u64);
            continue;
        }
        tested += 1;
        let mut entry_reader = (&mut r).take(hdr.size);
        if let Err(e) = io::copy(&mut entry_reader, &mut sink) {
            bad.push(format!("{}: {e}", hdr.name));
            break;
        }
        if let Err(e) = skip_bytes(&mut r, hdr.data_pad as u64) {
            bad.push(format!("{}: {e}", hdr.name));
            break;
        }
        report(
            progress,
            Progress {
                current_path: hdr.name,
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

pub fn read_entry(
    path: &Path,
    fmt: Format,
    entry_path: &str,
    opts: &ReadOptions,
) -> Result<EntryContent> {
    let mut r = open_payload(path, fmt)?;
    let target = entry_path.trim_start_matches("./").trim_matches('/');

    while let Some(hdr) = next_header(&mut r)? {
        if hdr.name.trim_matches('/') == target {
            let mut slice = (&mut r).take(hdr.size);
            return crate::formats::read_bounded(&mut slice, hdr.name, hdr.size, opts);
        }
        skip_bytes(&mut r, hdr.size + hdr.data_pad as u64)?;
    }
    Err(Error::other(format!("entry not found in archive: {entry_path}")))
}

pub(crate) fn restore_metadata(path: &Path, mode: Option<u32>, mtime: Option<i64>) {
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777));
    }
    #[cfg(not(unix))]
    let _ = mode;

    if let Some(secs) = mtime.filter(|&s| s > 0) {
        let t = filetime::FileTime::from_unix_time(secs, 0);
        let _ = filetime::set_file_times(path, t, t);
    }
}
