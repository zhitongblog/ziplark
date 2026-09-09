//! Archives whose entry names are *not* UTF-8 must still list and extract with
//! readable filenames.
//!
//! A ZIP made by Windows Explorer or WinRAR on Chinese Windows stores names as
//! raw CP936 bytes and leaves general-purpose bit 11 clear. Read as CP437 —
//! which is what the `zip` crate does by default — `中文文件.txt` comes out as
//! `ÖÐÎÄÎÄ¼þ.txt`, and that mojibake is what lands on disk.
//!
//! The fixtures here are built byte by byte rather than with a ZIP writer,
//! because every writer worth using (ours included) encodes non-ASCII names as
//! UTF-8 and sets the flag — which is correct, and exactly what makes it
//! impossible to reproduce the bug through one.

use ziplark_core::*;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// `项目文档/说明.doc` in CP936.
const GBK_DOC: &[u8] = &[
    0xcf, 0xee, 0xc4, 0xbf, 0xce, 0xc4, 0xb5, 0xb5, 0x2f, 0xcb, 0xb5, 0xc3, 0xf7, 0x2e, 0x64, 0x6f,
    0x63,
];
/// `照片/春节合影.jpg` in CP936.
const GBK_JPG: &[u8] = &[
    0xd5, 0xd5, 0xc6, 0xac, 0x2f, 0xb4, 0xba, 0xbd, 0xda, 0xba, 0xcf, 0xd3, 0xb0, 0x2e, 0x6a, 0x70,
    0x67,
];
/// `新しいフォルダ/資料.txt` in CP932.
const SJIS_TXT: &[u8] = &[
    0x90, 0x56, 0x82, 0xb5, 0x82, 0xa2, 0x83, 0x74, 0x83, 0x48, 0x83, 0x8b, 0x83, 0x5f, 0x2f, 0x8e,
    0x91, 0x97, 0xbf, 0x2e, 0x74, 0x78, 0x74,
];

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ziplark-names-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn names_of(archive: &Path) -> Vec<String> {
    list(archive, &ListOptions::default())
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.path)
        .collect()
}

fn extract_into(archive: &Path, dest: &Path) {
    let mut opts = ExtractOptions::new(dest);
    opts.overwrite = true;
    extract(archive, &opts, None).unwrap();
}

#[test]
fn zip_with_cp936_names_reads_as_chinese_not_mojibake() {
    let root = tmp("gbk-zip");
    let archive = root.join("windows-made.zip");
    write_raw_name_zip(
        &archive,
        &[(GBK_DOC, b"doc contents"), (GBK_JPG, b"jpg contents")],
    );

    assert_eq!(names_of(&archive), ["项目文档/说明.doc", "照片/春节合影.jpg"]);

    let dest = root.join("out");
    extract_into(&archive, &dest);
    assert_eq!(
        fs::read_to_string(dest.join("项目文档/说明.doc")).unwrap(),
        "doc contents"
    );
    assert!(dest.join("照片/春节合影.jpg").exists());
}

#[test]
fn zip_with_cp932_names_reads_as_japanese() {
    let root = tmp("sjis-zip");
    let archive = root.join("windows-made.zip");
    write_raw_name_zip(&archive, &[(SJIS_TXT, b"data")]);

    assert_eq!(names_of(&archive), ["新しいフォルダ/資料.txt"]);

    let dest = root.join("out");
    extract_into(&archive, &dest);
    assert!(dest.join("新しいフォルダ/資料.txt").exists());
}

/// Names that really are UTF-8 must be left alone — no detector second-guessing
/// an archive that already told us the answer.
#[test]
fn utf8_names_are_untouched() {
    let root = tmp("utf8-zip");
    let src = root.join("中文目录");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("说明.txt"), b"hello").unwrap();

    let archive = root.join("ours.zip");
    create(&archive, &[src], &CreateOptions::new(Format::Zip), None).unwrap();
    assert_eq!(names_of(&archive), ["中文目录/说明.txt"]);

    let dest = root.join("out");
    extract_into(&archive, &dest);
    assert_eq!(
        fs::read_to_string(dest.join("中文目录/说明.txt")).unwrap(),
        "hello"
    );
}

/// tar has no encoding field at all, so the same treatment applies.
#[cfg(unix)]
#[test]
fn tar_with_cp936_names_reads_as_chinese() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let root = tmp("gbk-tar");
    let archive = root.join("legacy.tar");
    {
        let mut b = tar::Builder::new(fs::File::create(&archive).unwrap());
        for (raw, data) in [(GBK_DOC, &b"doc contents"[..]), (GBK_JPG, &b"jpg contents"[..])] {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, Path::new(OsStr::from_bytes(raw)), data)
                .unwrap();
        }
        b.finish().unwrap();
    }

    assert_eq!(names_of(&archive), ["项目文档/说明.doc", "照片/春节合影.jpg"]);

    let dest = root.join("out");
    extract_into(&archive, &dest);
    assert_eq!(
        fs::read_to_string(dest.join("项目文档/说明.doc")).unwrap(),
        "doc contents"
    );
}

// ───────────────────── a ZIP writer that keeps raw names ─────────────────────

/// Write a stored (uncompressed) ZIP whose entry names are the exact bytes
/// given, with general-purpose bit 11 clear — i.e. what a localized Windows
/// produces, and what no ordinary ZIP writer will emit for us.
fn write_raw_name_zip(path: &Path, entries: &[(&[u8], &[u8])]) {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();

    for (name, data) in entries {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let len = data.len() as u32;

        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes()); // local header
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags: bit 11 CLEAR
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0x21u16.to_le_bytes()); // mod date (1980-01-01)
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes()); // compressed size
        out.extend_from_slice(&len.to_le_bytes()); // uncompressed size
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        out.extend_from_slice(name);
        out.extend_from_slice(data);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes()); // central header
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags: bit 11 CLEAR
        central.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        central.extend_from_slice(&0u16.to_le_bytes()); // mod time
        central.extend_from_slice(&0x21u16.to_le_bytes()); // mod date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk number
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }

    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes()); // end of central dir
    out.extend_from_slice(&0u16.to_le_bytes()); // this disk
    out.extend_from_slice(&0u16.to_le_bytes()); // disk with CD
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length

    fs::File::create(path).unwrap().write_all(&out).unwrap();
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
