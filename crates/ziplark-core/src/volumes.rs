//! Multi-volume and offset-sliced archive readers for 7z and ZIP.
//!
//! Two kinds of multi-file archives show up outside RAR:
//!
//! 1. **Byte-split sets** (`archive.7z.001`, `archive.7z.002`, … or
//!    `archive.zip.001`, `archive.tar.gz.001`, …) — a single archive stream
//!    sliced into fixed-size files by 7-Zip or `split`.
//! 2. **PKWARE split ZIP sets** (`archive.z01`, `archive.z02`, …,
//!    `archive.zip`) — `.z01` is disk 0 (prefixed with `PK\x07\x08`), `.z02` is
//!    disk 1, and `.zip` is the final disk carrying the central directory,
//!    whose entries store disk-local offsets.
//!
//! [`VolumeReader`] presents any of these (plus offset-prefixed containers like
//! `.crx`) as a single seekable stream without copying the volumes to a
//! temporary file.

use crate::error::{Error, Result};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Naming scheme of a non-RAR split archive.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SplitScheme {
    /// `<base>.001`, `<base>.002`, … where `width` is the zero-padded digit count.
    Numbered { base: String, width: usize },
    /// `<stem>.z01`, `<stem>.z02`, …, `<stem>.zip`.
    SplitZip { stem: String },
    /// A single standalone file.
    Single,
}

/// Summary of the volume files belonging to an archive.
#[derive(Debug, Clone)]
pub struct VolumeSet {
    /// The canonical path to report as the archive path (`first` for `.001`
    /// sets, `.zip` or `.z01` for split ZIPs).
    pub primary: PathBuf,
    /// Ordered list of volume files present on disk (empty for single-file archives).
    pub volumes: Vec<PathBuf>,
    /// The first missing volume if a gap or missing tail/head is detected.
    pub missing_volume: Option<PathBuf>,
    scheme: SplitScheme,
}

impl VolumeSet {
    /// Inspect `path` and discover all volumes of its set on disk.
    pub fn inspect(path: &Path) -> Self {
        let scheme = detect_scheme(path);
        match &scheme {
            SplitScheme::Single => Self {
                primary: path.to_path_buf(),
                volumes: Vec::new(),
                missing_volume: None,
                scheme,
            },
            SplitScheme::Numbered { base, width } => {
                let first = path.with_file_name(format!("{base}.{:0width$}", 1, width = *width));
                let primary = if first.exists() {
                    first.clone()
                } else {
                    path.to_path_buf()
                };
                if !first.exists() {
                    return Self {
                        primary,
                        volumes: vec![path.to_path_buf()],
                        missing_volume: Some(first),
                        scheme,
                    };
                }
                let mut volumes = Vec::new();
                let mut missing_volume = None;
                for n in 1..=9999usize {
                    let cand = path.with_file_name(format!("{base}.{:0width$}", n, width = *width));
                    if cand.exists() {
                        volumes.push(cand);
                    } else {
                        // Check if a later volume exists on disk (meaning `cand` is a hole in the middle).
                        let next_exists = (n + 1..=n + 3).any(|k| {
                            path.with_file_name(format!("{base}.{:0width$}", k, width = *width))
                                .exists()
                        });
                        if next_exists {
                            missing_volume = Some(cand);
                        }
                        break;
                    }
                }
                if missing_volume.is_none() {
                    let present_bytes: u64 = volumes
                        .iter()
                        .filter_map(|p| std::fs::metadata(p).ok().map(|m| m.len()))
                        .sum();
                    if let Some(expected_total) = sevenz_expected_len(&first) {
                        if present_bytes < expected_total {
                            let next_n = volumes.len() + 1;
                            missing_volume = Some(
                                path.with_file_name(format!("{base}.{:0width$}", next_n, width = *width)),
                            );
                        }
                    }
                }
                Self {
                    primary,
                    volumes,
                    missing_volume,
                    scheme,
                }
            }
            SplitScheme::SplitZip { stem } => {
                let zip_path = path.with_file_name(format!("{stem}.zip"));
                let mut volumes = Vec::new();
                let mut missing_volume = None;
                for n in 1..=999usize {
                    let z_part = path.with_file_name(format!("{stem}.z{n:02}"));
                    if z_part.exists() {
                        volumes.push(z_part);
                    } else {
                        let later_z = (n + 1..=n + 3).any(|k| {
                            path.with_file_name(format!("{stem}.z{k:02}")).exists()
                        });
                        if later_z {
                            missing_volume = Some(z_part);
                        }
                        break;
                    }
                }
                if zip_path.exists() {
                    volumes.push(zip_path.clone());
                } else if missing_volume.is_none() {
                    missing_volume = Some(zip_path.clone());
                }
                let primary = if zip_path.exists() {
                    zip_path.clone()
                } else {
                    path.to_path_buf()
                };
                if missing_volume.is_none() && zip_path.exists() {
                    if let Some(expected_disks) = split_zip_expected_disks(&zip_path) {
                        if volumes.len() < expected_disks {
                            let missing_idx = volumes.len();
                            missing_volume = Some(
                                path.with_file_name(format!("{stem}.z{missing_idx:02}")),
                            );
                        }
                    }
                }
                Self {
                    primary,
                    volumes,
                    missing_volume,
                    scheme,
                }
            }
        }
    }

    /// Expected path of the next numbered volume after those currently on disk.
    pub fn next_expected_volume(&self) -> Option<PathBuf> {
        if let Some(m) = &self.missing_volume {
            return Some(m.clone());
        }
        match &self.scheme {
            SplitScheme::Numbered { base, width } => {
                let next_n = self.volumes.len() + 1;
                Some(self.primary.with_file_name(format!(
                    "{base}.{:0width$}",
                    next_n,
                    width = *width
                )))
            }
            SplitScheme::SplitZip { stem } => {
                Some(self.primary.with_file_name(format!("{stem}.zip")))
            }
            SplitScheme::Single => None,
        }
    }

    /// Total compressed bytes across all present volumes (or the single file).
    pub fn total_compressed_size(&self) -> Result<u64> {
        if self.volumes.is_empty() {
            Ok(std::fs::metadata(&self.primary)?.len())
        } else {
            Ok(self
                .volumes
                .iter()
                .filter_map(|p| std::fs::metadata(p).ok().map(|m| m.len()))
                .sum())
        }
    }
}

fn detect_scheme(path: &Path) -> SplitScheme {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return SplitScheme::Single;
    };
    let lower = name.to_ascii_lowercase();

    // `<base>.001` .. `<base>.999` where `<base>` has an archive extension.
    if let Some(dot) = lower.rfind('.') {
        let ext = &lower[dot + 1..];
        let base_lower = &lower[..dot];
        if (2..=4).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_digit())
            && is_splittable_base(base_lower)
        {
            return SplitScheme::Numbered {
                base: name[..dot].to_string(),
                width: ext.len(),
            };
        }

        // `<stem>.z01` .. `<stem>.z99`
        if ext.len() >= 3
            && ext.starts_with('z')
            && ext[1..].chars().all(|c| c.is_ascii_digit())
        {
            return SplitScheme::SplitZip {
                stem: name[..dot].to_string(),
            };
        }

        // `<stem>.zip` when `<stem>.z01` sits right next to it.
        if ext == "zip" {
            let stem = &name[..dot];
            if path.with_file_name(format!("{stem}.z01")).exists() {
                return SplitScheme::SplitZip {
                    stem: stem.to_string(),
                };
            }
        }
    }
    SplitScheme::Single
}

fn is_splittable_base(base_lower: &str) -> bool {
    [
        ".7z", ".zip", ".tar", ".tgz", ".tbz2", ".txz", ".tzst", ".tlz4", ".gz", ".bz2", ".xz",
        ".zst", ".lz4", ".iso", ".cb7", ".cbz",
    ]
    .iter()
    .any(|ext| base_lower.ends_with(ext))
}

struct Part {
    path: PathBuf,
    start: u64,
    len: u64,
}

/// A seekable reader across one or more volume files, with optional leading
/// byte-offset slicing (for `.crx` headers) and tail patching (for multi-disk
/// split `.z01` + `.zip` central directories).
pub struct VolumeReader {
    parts: Vec<Part>,
    base_offset: u64,
    total_len: u64,
    pos: u64,
    current: Option<(usize, File)>,
    /// Optional patched tail overlay `(global_start_offset, bytes)`.
    tail_patch: Option<(u64, Vec<u8>)>,
}

impl VolumeReader {
    /// Open a volume set (or single file) for raw sequential/random reading.
    pub fn open(set: &VolumeSet) -> Result<Self> {
        if let Some(missing) = &set.missing_volume {
            return Err(Error::MissingVolume(missing.clone()));
        }
        let files = if set.volumes.is_empty() {
            vec![set.primary.clone()]
        } else {
            set.volumes.clone()
        };

        let mut parts = Vec::with_capacity(files.len());
        let mut offset = 0u64;
        for p in files {
            let len = std::fs::metadata(&p)?.len();
            parts.push(Part {
                path: p,
                start: offset,
                len,
            });
            offset += len;
        }
        Ok(Self {
            parts,
            base_offset: 0,
            total_len: offset,
            pos: 0,
            current: None,
            tail_patch: None,
        })
    }

    /// Open a ZIP archive (single file, `.crx` container, `.zip.001` split, or
    /// `.z01` + `.zip` multi-disk split).
    pub fn open_zip(set: &VolumeSet) -> Result<Self> {
        let mut reader = Self::open(set)?;
        if reader.parts.len() == 1 {
            if let Some(crx_off) = detect_crx_offset(&reader.parts[0].path)? {
                if crx_off < reader.total_len {
                    reader.base_offset = crx_off;
                    reader.total_len -= crx_off;
                }
            }
        } else if matches!(set.scheme, SplitScheme::SplitZip { .. }) {
            reader.patch_split_zip_central_directory()?;
        }
        Ok(reader)
    }

    /// Translate multi-disk numbers and per-disk local header offsets in a
    /// PKWARE split ZIP's Central Directory into global stream offsets so
    /// `zip::ZipArchive` can read the set seamlessly.
    fn patch_split_zip_central_directory(&mut self) -> Result<()> {
        let window = self.total_len.min(65_557) as usize;
        let tail_start = self.total_len - window as u64;
        let mut tail = vec![0u8; window];
        self.seek(SeekFrom::Start(tail_start))?;
        self.read_exact(&mut tail)?;
        self.pos = 0;

        // Locate End of Central Directory (`PK\x05\x06`) scanning backwards.
        let Some(eocd_rel) = (0..=tail.len().saturating_sub(22))
            .rev()
            .find(|&i| &tail[i..i + 4] == b"PK\x05\x06")
        else {
            return Err(Error::corrupt("split ZIP is missing End of Central Directory"));
        };

        let eocd = &tail[eocd_rel..];
        let this_disk = u16::from_le_bytes([eocd[4], eocd[5]]) as usize;
        let cd_disk = u16::from_le_bytes([eocd[6], eocd[7]]) as usize;
        let cd_entries_total = u16::from_le_bytes([eocd[10], eocd[11]]);
        let cd_offset_in_disk = u32::from_le_bytes([eocd[16], eocd[17], eocd[18], eocd[19]]) as u64;

        if this_disk == 0 && cd_disk == 0 {
            return Ok(());
        }
        // Check that all disks referenced by the EOCD are present.
        if this_disk + 1 > self.parts.len() || cd_disk >= self.parts.len() {
            let stem = self.parts[0]
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("archive");
            let missing_idx = self.parts.len();
            let missing = self.parts[0]
                .path
                .with_file_name(format!("{stem}.z{missing_idx:02}"));
            return Err(Error::MissingVolume(missing));
        }

        let cd_global = self.parts[cd_disk].start + cd_offset_in_disk;
        if cd_global > self.total_len {
            return Err(Error::corrupt("split ZIP central directory offset out of bounds"));
        }

        let patch_len = (self.total_len - cd_global) as usize;
        let mut buf = vec![0u8; patch_len];
        self.seek(SeekFrom::Start(cd_global))?;
        self.read_exact(&mut buf)?;
        self.pos = 0;

        let mut cursor = 0usize;
        for _ in 0..cd_entries_total {
            if cursor + 46 > buf.len() || &buf[cursor..cursor + 4] != b"PK\x01\x02" {
                break;
            }
            let name_len = u16::from_le_bytes([buf[cursor + 28], buf[cursor + 29]]) as usize;
            let extra_len = u16::from_le_bytes([buf[cursor + 30], buf[cursor + 31]]) as usize;
            let comment_len = u16::from_le_bytes([buf[cursor + 32], buf[cursor + 33]]) as usize;
            let disk_num = u16::from_le_bytes([buf[cursor + 34], buf[cursor + 35]]) as usize;
            let rel_off = u32::from_le_bytes([
                buf[cursor + 42],
                buf[cursor + 43],
                buf[cursor + 44],
                buf[cursor + 45],
            ]) as u64;

            if disk_num >= self.parts.len() {
                return Err(Error::corrupt("split ZIP entry references missing disk"));
            }
            let global_off = self.parts[disk_num].start + rel_off;
            buf[cursor + 34..cursor + 36].copy_from_slice(&0u16.to_le_bytes());
            buf[cursor + 42..cursor + 46].copy_from_slice(&(global_off as u32).to_le_bytes());

            cursor += 46 + name_len + extra_len + comment_len;
        }

        // Patch the EOCD at the end of `buf`.
        let eocd_in_buf = (tail_start + eocd_rel as u64 - cd_global) as usize;
        if eocd_in_buf + 22 <= buf.len() && &buf[eocd_in_buf..eocd_in_buf + 4] == b"PK\x05\x06" {
            buf[eocd_in_buf + 4..eocd_in_buf + 6].copy_from_slice(&0u16.to_le_bytes());
            buf[eocd_in_buf + 6..eocd_in_buf + 8].copy_from_slice(&0u16.to_le_bytes());
            buf[eocd_in_buf + 8..eocd_in_buf + 10]
                .copy_from_slice(&cd_entries_total.to_le_bytes());
            buf[eocd_in_buf + 16..eocd_in_buf + 20]
                .copy_from_slice(&(cd_global as u32).to_le_bytes());
        }

        self.tail_patch = Some((cd_global, buf));
        Ok(())
    }
}

impl Read for VolumeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.pos >= self.total_len {
            return Ok(0);
        }
        let abs_pos = self.base_offset + self.pos;

        // If reading within the patched central-directory overlay, serve from memory.
        if let Some((patch_start, ref patch_bytes)) = self.tail_patch {
            if abs_pos >= patch_start {
                let off = (abs_pos - patch_start) as usize;
                if off >= patch_bytes.len() {
                    return Ok(0);
                }
                let avail = (patch_bytes.len() - off).min(out.len());
                out[..avail].copy_from_slice(&patch_bytes[off..off + avail]);
                self.pos += avail as u64;
                return Ok(avail);
            }
        }

        // Clamp read if it would cross into `tail_patch`.
        let max_len = if let Some((patch_start, _)) = self.tail_patch {
            if abs_pos < patch_start {
                (patch_start - abs_pos).min(out.len() as u64) as usize
            } else {
                out.len()
            }
        } else {
            out.len()
        };

        let Some(part_idx) = self
            .parts
            .iter()
            .position(|p| abs_pos >= p.start && abs_pos < p.start + p.len)
        else {
            return Ok(0);
        };

        let part_offset = abs_pos - self.parts[part_idx].start;
        let part_rem = (self.parts[part_idx].len - part_offset).min(max_len as u64) as usize;

        let need_open = match &self.current {
            Some((idx, _)) => *idx != part_idx,
            None => true,
        };
        if need_open {
            let f = File::open(&self.parts[part_idx].path)?;
            self.current = Some((part_idx, f));
        }
        let (_, file) = self.current.as_mut().unwrap();
        file.seek(SeekFrom::Start(part_offset))?;
        let n = file.read(&mut out[..part_rem])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for VolumeReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let next: i128 = match pos {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
            SeekFrom::End(d) => self.total_len as i128 + d as i128,
        };
        if next < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek to a negative position",
            ));
        }
        self.pos = next as u64;
        Ok(self.pos)
    }
}

/// Read the End of Central Directory of `.zip` to see how many disks the split
/// set is supposed to have (`this_disk + 1`).
fn split_zip_expected_disks(zip_path: &Path) -> Option<usize> {
    let mut f = File::open(zip_path).ok()?;
    let len = f.metadata().ok()?.len();
    let window = len.min(65_557) as usize;
    let mut tail = vec![0u8; window];
    f.seek(SeekFrom::Start(len - window as u64)).ok()?;
    f.read_exact(&mut tail).ok()?;
    let eocd_rel = (0..=tail.len().saturating_sub(22))
        .rev()
        .find(|&i| &tail[i..i + 4] == b"PK\x05\x06")?;
    let this_disk = u16::from_le_bytes([tail[eocd_rel + 4], tail[eocd_rel + 5]]) as usize;
    Some(this_disk + 1)
}

/// Read the 32-byte 7z signature header to determine the total archive byte length
/// (`32 + next_header_offset + next_header_size`).
fn sevenz_expected_len(first_vol: &Path) -> Option<u64> {
    let mut f = File::open(first_vol).ok()?;
    let mut hdr = [0u8; 32];
    f.read_exact(&mut hdr).ok()?;
    if &hdr[0..6] != b"7z\xBC\xAF\x27\x1C" {
        return None;
    }
    let next_off = u64::from_le_bytes(hdr[12..20].try_into().ok()?);
    let next_size = u64::from_le_bytes(hdr[20..28].try_into().ok()?);
    32u64.checked_add(next_off)?.checked_add(next_size)
}

/// If `path` is a Chrome `.crx` extension (CRX2 or CRX3), return the byte offset
/// where the embedded ZIP payload begins.
pub fn detect_crx_offset(path: &Path) -> io::Result<Option<u64>> {
    let mut f = File::open(path)?;
    let mut hdr = [0u8; 16];
    if f.read_exact(&mut hdr).is_err() || &hdr[0..4] != b"Cr24" {
        return Ok(None);
    }
    let version = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
    match version {
        2 => {
            let pub_len = u32::from_le_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]) as u64;
            let sig_len = u32::from_le_bytes([hdr[12], hdr[13], hdr[14], hdr[15]]) as u64;
            Ok(Some(16 + pub_len + sig_len))
        }
        3 => {
            let header_len = u32::from_le_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]) as u64;
            Ok(Some(12 + header_len))
        }
        _ => Ok(None),
    }
}
