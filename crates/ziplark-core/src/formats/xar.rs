//! XAR archives (`.xar`, `.xip`, `.safariextz`) and macOS Installer packages (`.pkg`).
//!
//! A XAR archive starts with a 28+ byte header (`xar!`), followed by a
//! zlib-compressed XML Table of Contents (TOC), followed immediately by the
//! heap containing each file's data stream at `<offset>` (relative to the start
//! of the heap) of `<length>` compressed bytes and `<size>` uncompressed bytes.
//!
//! When opened as [`Format::Pkg`] (or when a XAR contains `Payload` / `Scripts`
//! CPIO streams inside `*.pkg/` component directories or at the root), Ziplark
//! transparently traverses into those compressed CPIO payloads — just like we
//! traverse `data.tar.*` inside `.deb` — so users and agents see the actual
//! application/files inside the macOS installer alongside `Distribution` and
//! `PackageInfo`.

use crate::error::{Error, Result};
use crate::formats::cpio::{next_header, restore_metadata, skip_bytes, wrap_compressed_stream};
use crate::formats::{
    copy_watched, create_file, create_symlink, ensure_parent, prepare_dir, prepare_leaf, report,
    DestGuard,
};
use crate::model::*;
use crate::{ExtractOptions, ListOptions, ProgressFn, ReadOptions};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

pub const XAR_MAGIC: &[u8; 4] = b"xar!";

#[derive(Debug, Clone)]
struct XarNode {
    path: String,
    is_dir: bool,
    mode: Option<u32>,
    mtime: Option<i64>,
    /// Uncompressed size.
    size: u64,
    /// Compressed length on the heap.
    length: u64,
    /// Offset relative to `heap_offset`.
    offset: u64,
    /// Encoding style, e.g. `"application/x-gzip"`, `"application/x-bzip2"`,
    /// `"application/x-xz"`, `"application/octet-stream"`.
    encoding: String,
}

impl XarNode {
    /// True if this node is a macOS `.pkg` CPIO payload (`Payload` or `Scripts`).
    fn is_pkg_cpio_payload(&self) -> bool {
        if self.is_dir || self.length == 0 {
            return false;
        }
        let leaf = self.path.rsplit('/').next().unwrap_or(&self.path);
        leaf == "Payload" || leaf == "Scripts"
    }

    /// Directory prefix to place unpacked CPIO entries under when expanding a
    /// `.pkg` payload.
    ///
    /// - For a single-component or flat pkg where only one `Payload` exists (or
    ///   it sits at the root), `Payload` unpacks at the root (`""`), and
    ///   `Scripts` unpacks under `"Scripts"` (or `<pkg>/Scripts`).
    fn cpio_prefix(&self, multi_payload: bool) -> String {
        let leaf = self.path.rsplit('/').next().unwrap_or(&self.path);
        let parent = self
            .path
            .rsplit_once('/')
            .map(|(p, _)| p)
            .unwrap_or("");
        if leaf == "Scripts" {
            self.path.clone()
        } else if multi_payload && !parent.is_empty() {
            parent.to_string()
        } else {
            String::new()
        }
    }
}

struct ParsedXar {
    heap_offset: u64,
    nodes: Vec<XarNode>,
}

fn parse_xar(f: &mut File) -> Result<ParsedXar> {
    let file_len = f.metadata()?.len();
    f.seek(SeekFrom::Start(0))?;
    let mut hdr = [0u8; 28];
    f.read_exact(&mut hdr)
        .map_err(|_| Error::corrupt("XAR archive is truncated (missing 28-byte header)"))?;
    if &hdr[0..4] != XAR_MAGIC {
        return Err(Error::corrupt("invalid XAR magic"));
    }

    let hdr_size = u16::from_be_bytes([hdr[4], hdr[5]]) as u64;
    let toc_len_compressed = u64::from_be_bytes(hdr[8..16].try_into().unwrap());
    let toc_len_uncompressed = u64::from_be_bytes(hdr[16..24].try_into().unwrap());

    if hdr_size < 28 || hdr_size.saturating_add(toc_len_compressed) > file_len {
        return Err(Error::corrupt("XAR TOC extends past end of file"));
    }
    if toc_len_uncompressed > 128 * 1024 * 1024 {
        return Err(Error::corrupt("XAR TOC exceeds 128 MiB safety cap"));
    }

    f.seek(SeekFrom::Start(hdr_size))?;
    let mut toc_z = (&mut *f).take(toc_len_compressed);
    let mut zdec = flate2::read::ZlibDecoder::new(&mut toc_z);
    let mut xml = String::new();
    zdec.read_to_string(&mut xml)
        .map_err(|e| Error::corrupt(format!("corrupt XAR zlib TOC: {e}")))?;

    let heap_offset = hdr_size + toc_len_compressed;
    let mut nodes = Vec::new();
    parse_toc_files(&xml, "", &mut nodes);

    Ok(ParsedXar { heap_offset, nodes })
}

/// Recursively parse `<file>` elements inside the XAR TOC XML.
fn parse_toc_files(xml_slice: &str, parent_path: &str, out: &mut Vec<XarNode>) {
    let mut pos = 0usize;
    while let Some(rel) = xml_slice[pos..].find("<file") {
        let start = pos + rel;
        let after_tag = &xml_slice[start + 5..];
        if !after_tag.starts_with(' ') && !after_tag.starts_with('>') {
            pos = start + 5;
            continue;
        }
        let Some(open_end_rel) = xml_slice[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel;
        if xml_slice[start..=open_end].ends_with("/>") {
            pos = open_end + 1;
            continue;
        }
        let body_start = open_end + 1;
        let Some((body_end, next_pos)) = find_matching_file_close(xml_slice, body_start) else {
            break;
        };
        let body = &xml_slice[body_start..body_end];
        parse_single_file_body(body, parent_path, out);
        pos = next_pos;
    }
}

/// Find the matching `</file>` for a `<file ...>` whose content starts at
/// `body_start`, accounting for nested `<file ...>...</file>` children.
fn find_matching_file_close(s: &str, body_start: usize) -> Option<(usize, usize)> {
    let mut depth = 1usize;
    let mut idx = body_start;
    while idx < s.len() {
        let next_open = s[idx..].find("<file");
        let next_close = s[idx..].find("</file>");
        match (next_open, next_close) {
            (Some(o), Some(c)) if o < c => {
                let abs_o = idx + o;
                let after = &s[abs_o + 5..];
                if after.starts_with(' ') || after.starts_with('>') {
                    let close_angle = s[abs_o..].find('>')?;
                    if !s[abs_o..=abs_o + close_angle].ends_with("/>") {
                        depth += 1;
                    }
                    idx = abs_o + close_angle + 1;
                } else {
                    idx = abs_o + 5;
                }
            }
            (_, Some(c)) => {
                let abs_c = idx + c;
                depth -= 1;
                if depth == 0 {
                    return Some((abs_c, abs_c + "</file>".len()));
                }
                idx = abs_c + "</file>".len();
            }
            _ => return None,
        }
    }
    None
}

fn parse_single_file_body(body: &str, parent_path: &str, out: &mut Vec<XarNode>) {
    // Strip out nested `<file ...>...</file>` blocks and `<ea ...>...</ea>` blocks
    // when reading this node's own properties so child tags don't shadow parent tags.
    let shallow = strip_subblocks(body, "file");
    let shallow_no_ea = strip_subblocks(&shallow, "ea");

    let raw_name = extract_tag_text(&shallow_no_ea, "name").unwrap_or_default();
    let name = xml_unescape(raw_name.trim());
    if name.is_empty() || name == "." || name == ".." {
        return;
    }
    let full_path = if parent_path.is_empty() {
        name.clone()
    } else {
        format!("{parent_path}/{name}")
    };

    let ftype = extract_tag_text(&shallow_no_ea, "type").unwrap_or("file");
    let is_dir = ftype.trim().eq_ignore_ascii_case("directory");
    let mode = extract_tag_text(&shallow_no_ea, "mode")
        .and_then(|m| u32::from_str_radix(m.trim(), 8).ok());
    let mtime = extract_tag_text(&shallow_no_ea, "mtime").and_then(parse_iso8601_unix);

    let (mut size, mut length, mut offset, mut encoding) = (0u64, 0u64, 0u64, String::new());
    if let Some(data_block) = extract_tag_text(&shallow_no_ea, "data") {
        size = extract_tag_text(data_block, "size")
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(0);
        length = extract_tag_text(data_block, "length")
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(0);
        offset = extract_tag_text(data_block, "offset")
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(0);
        encoding = extract_encoding_style(data_block).unwrap_or_default();
    }

    out.push(XarNode {
        path: full_path.clone(),
        is_dir,
        mode,
        mtime,
        size,
        length,
        offset,
        encoding,
    });

    // Recurse into direct child `<file>` elements.
    parse_toc_files(body, &full_path, out);
}

fn strip_subblocks(s: &str, tag: &str) -> String {
    let open_pat = format!("<{tag}");
    let close_pat = format!("</{tag}>");
    let mut out = String::with_capacity(s.len());
    let mut pos = 0usize;
    while pos < s.len() {
        let Some(rel) = s[pos..].find(&open_pat) else {
            out.push_str(&s[pos..]);
            break;
        };
        let start = pos + rel;
        let after = &s[start + open_pat.len()..];
        if !after.starts_with(' ') && !after.starts_with('>') {
            out.push_str(&s[pos..start + open_pat.len()]);
            pos = start + open_pat.len();
            continue;
        }
        out.push_str(&s[pos..start]);
        if tag == "file" {
            let Some(open_end) = s[start..].find('>') else {
                break;
            };
            if s[start..=start + open_end].ends_with("/>") {
                pos = start + open_end + 1;
                continue;
            }
            if let Some((_, next_pos)) = find_matching_file_close(s, start + open_end + 1) {
                pos = next_pos;
            } else {
                break;
            }
        } else if let Some(end_rel) = s[start..].find(&close_pat) {
            pos = start + end_rel + close_pat.len();
        } else {
            break;
        }
    }
    out
}

fn extract_tag_text<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = s.find(&open)? + open.len();
    let end = start + s[start..].find(&close)?;
    Some(&s[start..end])
}

fn extract_encoding_style(data_block: &str) -> Option<String> {
    let idx = data_block.find("<encoding")?;
    let rest = &data_block[idx..];
    let end = rest.find('>')?;
    let tag = &rest[..end];
    let style_idx = tag.find("style=")?;
    let after = &tag[style_idx + 6..];
    let quote = after.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let val_end = after[1..].find(quote)?;
    Some(after[1..1 + val_end].to_string())
}

fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Parse an ISO-8601 timestamp (`YYYY-MM-DDTHH:MM:SSZ`) into a unix timestamp.
fn parse_iso8601_unix(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let year: i32 = s.get(0..4)?.parse().ok()?;
    if year < 1970 {
        return None;
    }
    let month: u8 = s.get(5..7)?.parse().ok()?;
    let day: u8 = s.get(8..10)?.parse().ok()?;
    let hour: u8 = s.get(11..13)?.parse().ok()?;
    let min: u8 = s.get(14..16)?.parse().ok()?;
    let sec: u8 = s.get(17..19)?.parse().ok()?;
    let m = time::Month::try_from(month).ok()?;
    let date = time::Date::from_calendar_date(year, m, day).ok()?;
    let tm = time::Time::from_hms(hour, min, sec).ok()?;
    let ts = date.with_time(tm).assume_utc().unix_timestamp();
    (ts > 0).then_some(ts)
}

/// Open a reader for a single XarNode's uncompressed bytes.
fn open_node_reader<'a>(
    f: &'a mut File,
    heap_offset: u64,
    node: &XarNode,
) -> Result<Box<dyn Read + 'a>> {
    f.seek(SeekFrom::Start(heap_offset + node.offset))?;
    let slice = (&mut *f).take(node.length);
    let enc = node.encoding.to_ascii_lowercase();
    Ok(if enc.contains("gzip") || enc.contains("zlib") {
        // XAR's "application/x-gzip" is actually raw zlib (RFC 1950) in standard
        // xar, though some tools write gzip (RFC 1952). Peek the 2-byte magic to
        // support both seamlessly!
        let mut peek = [0u8; 2];
        let mut slice = slice;
        let n = slice.read(&mut peek)?;
        let chained = io::Cursor::new(peek[..n].to_vec()).chain(slice);
        if n == 2 && peek == [0x1F, 0x8B] {
            Box::new(flate2::read::GzDecoder::new(chained))
        } else {
            Box::new(flate2::read::ZlibDecoder::new(chained))
        }
    } else if enc.contains("bzip2") {
        Box::new(bzip2::read::BzDecoder::new(slice))
    } else if enc.contains("xz") || enc.contains("lzma") {
        Box::new(xz2::read::XzDecoder::new(slice))
    } else {
        Box::new(slice)
    })
}

fn join_prefix(prefix: &str, name: &str) -> String {
    let clean = name.trim_start_matches("./").trim_matches('/');
    if clean.is_empty() || clean == "." {
        return prefix.to_string();
    }
    if prefix.is_empty() {
        clean.to_string()
    } else {
        format!("{prefix}/{clean}")
    }
}

pub fn list(path: &Path, fmt: Format, _opts: &ListOptions) -> Result<ArchiveInfo> {
    let mut f = File::open(path)?;
    let total_compressed = f.metadata()?.len();
    let parsed = parse_xar(&mut f)?;

    let expand_pkg = fmt == Format::Pkg
        || parsed.nodes.iter().any(|n| n.is_pkg_cpio_payload());
    let payload_count = parsed
        .nodes
        .iter()
        .filter(|n| n.path.rsplit('/').next().unwrap_or(&n.path) == "Payload")
        .count();
    let multi_payload = payload_count > 1;

    let mut entries = Vec::new();
    let mut total_size = 0u64;

    for node in &parsed.nodes {
        if expand_pkg && node.is_pkg_cpio_payload() {
            let prefix = node.cpio_prefix(multi_payload);
            let raw_stream = open_node_reader(&mut f, parsed.heap_offset, node)?;
            let mut cpio_r = wrap_compressed_stream(raw_stream)?;
            while let Some(hdr) = next_header(&mut cpio_r)? {
                let skip = hdr.size + hdr.data_pad as u64;
                skip_bytes(&mut cpio_r, skip)?;
                if hdr.name.is_empty() || hdr.name == "." {
                    continue;
                }
                let full_path = join_prefix(&prefix, &hdr.name);
                if full_path.is_empty() {
                    continue;
                }
                let is_dir = hdr.is_dir();
                if !is_dir {
                    total_size += hdr.size;
                }
                entries.push(ArchiveEntry {
                    path: full_path,
                    is_dir,
                    size: if is_dir { 0 } else { hdr.size },
                    compressed_size: None,
                    encrypted: false,
                    modified: (hdr.mtime > 0).then_some(hdr.mtime),
                    crc32: None,
                    split: false,
                });
            }
        } else {
            if !node.is_dir {
                total_size += node.size;
            }
            entries.push(ArchiveEntry {
                path: node.path.clone(),
                is_dir: node.is_dir,
                size: if node.is_dir { 0 } else { node.size },
                compressed_size: (!node.is_dir).then_some(node.length),
                encrypted: false,
                modified: node.mtime,
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
    let parsed = parse_xar(&mut f)?;
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

    let expand_pkg = fmt == Format::Pkg
        || parsed.nodes.iter().any(|n| n.is_pkg_cpio_payload());
    let payload_count = parsed
        .nodes
        .iter()
        .filter(|n| n.path.rsplit('/').next().unwrap_or(&n.path) == "Payload")
        .count();
    let multi_payload = payload_count > 1;
    let mut idx = 0u64;

    for node in &parsed.nodes {
        if expand_pkg && node.is_pkg_cpio_payload() {
            let prefix = node.cpio_prefix(multi_payload);
            let raw_stream = open_node_reader(&mut f, parsed.heap_offset, node)?;
            let mut cpio_r = wrap_compressed_stream(raw_stream)?;
            while let Some(hdr) = next_header(&mut cpio_r)? {
                let full_path = join_prefix(&prefix, &hdr.name);
                if full_path.is_empty() || hdr.name == "." || !selector.matches(&full_path) {
                    skip_bytes(&mut cpio_r, hdr.size + hdr.data_pad as u64)?;
                    continue;
                }
                idx += 1;
                let out_path = guard.join(&full_path)?;
                if hdr.is_dir() {
                    skip_bytes(&mut cpio_r, hdr.size + hdr.data_pad as u64)?;
                    prepare_dir(&out_path, &full_path)?;
                    report_out.dirs_created += 1;
                    continue;
                }
                ensure_parent(&out_path)?;
                if hdr.is_symlink() {
                    let mut target_bytes = vec![0u8; hdr.size as usize];
                    cpio_r.read_exact(&mut target_bytes)?;
                    skip_bytes(&mut cpio_r, hdr.data_pad as u64)?;
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
                let mut entry_reader = (&mut cpio_r).take(hdr.size);
                let n = copy_watched(&mut entry_reader, &mut out, |so_far| {
                    progress(Progress {
                        current_path: full_path.clone(),
                        entries_done: idx,
                        entries_total: 0,
                        bytes_done: done_before + so_far,
                        bytes_total: 0,
                    })
                })?;
                drop(out);
                skip_bytes(&mut cpio_r, hdr.data_pad as u64)?;
                restore_metadata(&out_path, Some(hdr.mode), Some(hdr.mtime));
                report_out.files_written += 1;
                report_out.bytes_written += n;
            }
            continue;
        }

        if !selector.matches(&node.path) {
            continue;
        }
        idx += 1;
        let out_path = guard.join(&node.path)?;
        if node.is_dir {
            prepare_dir(&out_path, &node.path)?;
            report_out.dirs_created += 1;
            continue;
        }
        ensure_parent(&out_path)?;
        let mut out = create_file(&out_path, opts.overwrite)?;
        let done_before = report_out.bytes_written;
        let mut reader = open_node_reader(&mut f, parsed.heap_offset, node)?;
        let n = copy_watched(&mut reader, &mut out, |so_far| {
            progress(Progress {
                current_path: node.path.clone(),
                entries_done: idx,
                entries_total: 0,
                bytes_done: done_before + so_far,
                bytes_total: 0,
            })
        })?;
        drop(out);
        restore_metadata(&out_path, node.mode, node.mtime);
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
    let mut f = File::open(path)?;
    let parsed = parse_xar(&mut f)?;
    let expand_pkg = fmt == Format::Pkg
        || parsed.nodes.iter().any(|n| n.is_pkg_cpio_payload());
    let payload_count = parsed
        .nodes
        .iter()
        .filter(|n| n.path.rsplit('/').next().unwrap_or(&n.path) == "Payload")
        .count();
    let multi_payload = payload_count > 1;

    let mut tested = 0u64;
    let mut bad = Vec::new();
    let mut sink = io::sink();

    for node in &parsed.nodes {
        if node.is_dir {
            continue;
        }
        if expand_pkg && node.is_pkg_cpio_payload() {
            let prefix = node.cpio_prefix(multi_payload);
            let raw_stream = match open_node_reader(&mut f, parsed.heap_offset, node) {
                Ok(s) => s,
                Err(e) => {
                    bad.push(format!("{}: {e}", node.path));
                    continue;
                }
            };
            let mut cpio_r = match wrap_compressed_stream(raw_stream) {
                Ok(r) => r,
                Err(e) => {
                    bad.push(format!("{}: {e}", node.path));
                    continue;
                }
            };
            loop {
                let hdr = match next_header(&mut cpio_r) {
                    Ok(Some(h)) => h,
                    Ok(None) => break,
                    Err(e) => {
                        bad.push(format!("{}: {e}", node.path));
                        break;
                    }
                };
                if hdr.is_dir() {
                    let _ = skip_bytes(&mut cpio_r, hdr.size + hdr.data_pad as u64);
                    continue;
                }
                tested += 1;
                let full_path = join_prefix(&prefix, &hdr.name);
                let mut entry_reader = (&mut cpio_r).take(hdr.size);
                if let Err(e) = io::copy(&mut entry_reader, &mut sink) {
                    bad.push(format!("{full_path}: {e}"));
                    break;
                }
                if let Err(e) = skip_bytes(&mut cpio_r, hdr.data_pad as u64) {
                    bad.push(format!("{full_path}: {e}"));
                    break;
                }
                report(
                    progress,
                    Progress {
                        current_path: full_path,
                        entries_done: tested,
                        entries_total: 0,
                        bytes_done: 0,
                        bytes_total: 0,
                    },
                )?;
            }
        } else {
            tested += 1;
            let mut reader = match open_node_reader(&mut f, parsed.heap_offset, node) {
                Ok(r) => r,
                Err(e) => {
                    bad.push(format!("{}: {e}", node.path));
                    continue;
                }
            };
            if let Err(e) = io::copy(&mut reader, &mut sink) {
                bad.push(format!("{}: {e}", node.path));
            }
            report(
                progress,
                Progress {
                    current_path: node.path.clone(),
                    entries_done: tested,
                    entries_total: 0,
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
    let parsed = parse_xar(&mut f)?;
    let target = entry_path.trim_start_matches("./").trim_matches('/');

    let expand_pkg = fmt == Format::Pkg
        || parsed.nodes.iter().any(|n| n.is_pkg_cpio_payload());
    let payload_count = parsed
        .nodes
        .iter()
        .filter(|n| n.path.rsplit('/').next().unwrap_or(&n.path) == "Payload")
        .count();
    let multi_payload = payload_count > 1;

    // 1. Check direct XAR nodes first (e.g. `Distribution`, `PackageInfo`, `Bom`).
    for node in &parsed.nodes {
        if !(expand_pkg && node.is_pkg_cpio_payload()) && node.path.trim_matches('/') == target {
            let mut reader = open_node_reader(&mut f, parsed.heap_offset, node)?;
            return crate::formats::read_bounded(&mut reader, node.path.clone(), node.size, opts);
        }
    }

    // 2. Check inside expanded `.pkg` CPIO payloads.
    if expand_pkg {
        for node in &parsed.nodes {
            if !node.is_pkg_cpio_payload() {
                continue;
            }
            let prefix = node.cpio_prefix(multi_payload);
            let raw_stream = open_node_reader(&mut f, parsed.heap_offset, node)?;
            let mut cpio_r = wrap_compressed_stream(raw_stream)?;
            while let Some(hdr) = next_header(&mut cpio_r)? {
                let full_path = join_prefix(&prefix, &hdr.name);
                if full_path.trim_matches('/') == target {
                    let mut slice = (&mut cpio_r).take(hdr.size);
                    return crate::formats::read_bounded(&mut slice, full_path, hdr.size, opts);
                }
                skip_bytes(&mut cpio_r, hdr.size + hdr.data_pad as u64)?;
            }
        }
    }

    Err(Error::other(format!("entry not found in archive: {entry_path}")))
}
