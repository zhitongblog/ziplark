//! Integration tests for Ziplark v0.4.0 features:
//! - In-memory entry reading (`read_entry`) with offset & byte-limit paging
//! - Smart extraction (`smart_dest` / `ExtractOptions::smart` anti-tarbomb)
//! - Multi-volume `.7z.001`, `.zip.001`, and PKWARE split `.z01` + `.zip`
//! - New archive & package formats: `.ar`, `.deb`, `.cpio`, `.rpm`, `.cab`, `.crx`, `.whl`, `.apk`
//! - In-place archive append (`add`) for `.zip`, `.7z`, and `.tar.gz`

use flate2::write::{DeflateEncoder, GzEncoder};
use flate2::Compression;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use ziplark_core::*;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ziplark-v040-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

// ---------------------------------------------------------------------------
// 1. In-memory `read_entry` across formats
// ---------------------------------------------------------------------------

#[test]
fn read_entry_works_across_formats_with_offset_and_truncation() {
    let root = tmp("read_entry");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    let payload = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    fs::write(src.join("hello.txt"), payload).unwrap();

    for (fmt, ext) in [
        (Format::Zip, "zip"),
        (Format::SevenZ, "7z"),
        (Format::TarGz, "tar.gz"),
    ] {
        let archive = root.join(format!("sample.{ext}"));
        create(&archive, std::slice::from_ref(&src), &CreateOptions::new(fmt), None).unwrap();

        // Full read
        let full = read_entry(&archive, "src/hello.txt", &ReadOptions::default()).unwrap();
        assert_eq!(full.bytes, payload, "{ext}: full read mismatch");
        assert!(!full.truncated, "{ext}: should not be truncated");
        assert_eq!(full.size, payload.len() as u64);

        // Partial read with offset + max_bytes
        let opts = ReadOptions {
            password: None,
            offset: 10,
            max_bytes: 16,
        };
        let slice = read_entry(&archive, "src/hello.txt", &opts).unwrap();
        assert_eq!(slice.bytes, &payload[10..26], "{ext}: slice mismatch");
        assert!(slice.truncated, "{ext}: should report truncated=true");
        assert_eq!(slice.offset, 10);
    }

    // Also verify RAR & ISO fixtures can be read in-memory without touching disk
    let rar = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.rar");
    let rar_info = list(&rar, &ListOptions::default()).unwrap();
    let first_rar_file = rar_info.entries.iter().find(|e| !e.is_dir).unwrap();
    let rar_read = read_entry(&rar, &first_rar_file.path, &ReadOptions::default()).unwrap();
    assert_eq!(rar_read.bytes.len() as u64, first_rar_file.size);
    assert!(!rar_read.truncated);

    let iso = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.iso");
    let iso_info = list(&iso, &ListOptions::default()).unwrap();
    let first_iso_file = iso_info.entries.iter().find(|e| !e.is_dir).unwrap();
    let iso_read = read_entry(&iso, &first_iso_file.path, &ReadOptions::default()).unwrap();
    assert!(!iso_read.bytes.is_empty());
}

// ---------------------------------------------------------------------------
// 2. Smart extraction (anti-tarbomb & anti-double-folder)
// ---------------------------------------------------------------------------

#[test]
fn smart_extract_wraps_tarbombs_and_avoids_double_nesting() {
    let root = tmp("smart");

    // Case A: Tarbomb (multiple loose top-level files)
    let f1 = root.join("alpha.txt");
    let f2 = root.join("beta.txt");
    fs::write(&f1, b"alpha").unwrap();
    fs::write(&f2, b"beta").unwrap();
    let bomb = root.join("loose-files.zip");
    create(&bomb, &[f1, f2], &CreateOptions::new(Format::Zip), None).unwrap();

    let ex_bomb = root.join("ex_bomb");
    let mut opts = ExtractOptions::new(&ex_bomb);
    opts.smart = true;
    extract(&bomb, &opts, None).unwrap();
    // Should be wrapped inside `ex_bomb/loose-files/`
    assert!(ex_bomb.join("loose-files/alpha.txt").is_file());
    assert!(ex_bomb.join("loose-files/beta.txt").is_file());

    // Case B: Clean single-root archive extracted into a folder already named `<stem>`
    let pkg = root.join("my-pkg");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(pkg.join("README.md"), b"# My Package").unwrap();
    fs::write(pkg.join("main.rs"), b"fn main() {}").unwrap();
    let clean_archive = root.join("my-pkg.tar.gz");
    create(&clean_archive, &[pkg], &CreateOptions::new(Format::TarGz), None).unwrap();

    // User chooses destination `.../out/my-pkg` (the archive stem):
    // smart extraction must NOT create `.../out/my-pkg/my-pkg/README.md`.
    let target_dir = root.join("out/my-pkg");
    let mut opts2 = ExtractOptions::new(&target_dir);
    opts2.smart = true;
    extract(&clean_archive, &opts2, None).unwrap();
    assert!(target_dir.join("README.md").is_file());
    assert!(!target_dir.join("my-pkg/README.md").exists());
}

// ---------------------------------------------------------------------------
// 3. Multi-volume `.7z.001`, `.zip.001`, and PKWARE `.z01` + `.zip`
// ---------------------------------------------------------------------------

#[test]
fn multivolume_7z_and_missing_part_detection() {
    let root = tmp("7z_vols");
    let src = root.join("data.bin");
    let payload: Vec<u8> = (0..50_000u32).flat_map(|i| i.to_le_bytes()).collect();
    fs::write(&src, &payload).unwrap();

    let whole = root.join("whole.7z");
    create(&whole, &[src], &CreateOptions::new(Format::SevenZ), None).unwrap();
    let bytes = fs::read(&whole).unwrap();
    assert!(bytes.len() > 200);

    let mid = bytes.len() / 2;
    let vol1 = root.join("split.7z.001");
    let vol2 = root.join("split.7z.002");
    fs::write(&vol1, &bytes[..mid]).unwrap();
    fs::write(&vol2, &bytes[mid..]).unwrap();

    // Opening either .001 or .002 resolves the whole set
    let info = list(&vol2, &ListOptions::default()).unwrap();
    assert_eq!(info.format, Format::SevenZ);
    assert_eq!(info.volumes.len(), 2);

    let ex = root.join("ex");
    extract(&vol1, &ExtractOptions::new(&ex), None).unwrap();
    assert_eq!(fs::read(ex.join("data.bin")).unwrap(), payload);

    // Removing .002 should surface `missing_volume` in `list` and return `MissingVolume` on `extract`
    fs::remove_file(&vol2).unwrap();
    let broken_info = list(&vol1, &ListOptions::default()).unwrap();
    assert!(
        broken_info.missing_volume.is_some(),
        "expected missing_volume to be reported in ArchiveInfo"
    );
    let err = extract(&vol1, &ExtractOptions::new(root.join("ex2")), None).unwrap_err();
    assert!(
        matches!(err, Error::MissingVolume(_)),
        "expected MissingVolume when .7z.002 is missing, got {err:?}"
    );
}

#[test]
fn pkware_split_z01_and_zip_roundtrip() {
    let root = tmp("pkware_split");
    let f1 = root.join("one.txt");
    let f2 = root.join("two.txt");
    fs::write(&f1, b"first file in disk 0 ".repeat(40)).unwrap();
    fs::write(&f2, b"second file in disk 1 ".repeat(40)).unwrap();

    let whole = root.join("whole.zip");
    create(&whole, &[f1.clone(), f2.clone()], &CreateOptions::new(Format::Zip), None).unwrap();
    let raw = fs::read(&whole).unwrap();

    // Locate the End of Central Directory (PK\x05\x06) and Central Directory (PK\x01\x02)
    let eocd_pos = raw
        .windows(4)
        .rposition(|w| w == [0x50, 0x4B, 0x05, 0x06])
        .unwrap();
    let cd_offset = u32::from_le_bytes(raw[eocd_pos + 16..eocd_pos + 20].try_into().unwrap()) as usize;
    let cd_count = u16::from_le_bytes(raw[eocd_pos + 10..eocd_pos + 12].try_into().unwrap()) as usize;
    assert_eq!(cd_count, 2);

    // Split point right between entry 0's local data and entry 1's local header:
    // Read entry 1's local header offset from the second CD record.
    let mut pos = cd_offset;
    let n0 = u16::from_le_bytes(raw[pos + 28..pos + 30].try_into().unwrap()) as usize;
    let x0 = u16::from_le_bytes(raw[pos + 30..pos + 32].try_into().unwrap()) as usize;
    let c0 = u16::from_le_bytes(raw[pos + 32..pos + 34].try_into().unwrap()) as usize;
    let cd1_pos = pos + 46 + n0 + x0 + c0;
    let entry1_local_off =
        u32::from_le_bytes(raw[cd1_pos + 42..cd1_pos + 46].try_into().unwrap()) as usize;

    // Build a genuine PKWARE split archive:
    // - `split.z01` (disk 0): 4-byte split signature `PK\x07\x08` + entry 0 local header/data
    // - `split.zip` (disk 1): entry 1 local header/data + Central Directory (with disk-relative offsets) + EOCD
    let mut z01 = Vec::with_capacity(4 + entry1_local_off);
    z01.extend_from_slice(&[0x50, 0x4B, 0x07, 0x08]);
    z01.extend_from_slice(&raw[..entry1_local_off]);

    let mut zip_last = raw[entry1_local_off..].to_vec();
    let adj = entry1_local_off;
    // Patch entry 0 in CD: disk = 0, relative_offset = 4 (because of PK\x07\x08 prefix on disk 0)
    pos = cd_offset - adj;
    zip_last[pos + 34..pos + 36].copy_from_slice(&0u16.to_le_bytes());
    zip_last[pos + 42..pos + 46].copy_from_slice(&4u32.to_le_bytes());
    // Patch entry 1 in CD: disk = 1, relative_offset = 0 (starts at byte 0 of disk 1)
    let pos1 = cd1_pos - adj;
    zip_last[pos1 + 34..pos1 + 36].copy_from_slice(&1u16.to_le_bytes());
    zip_last[pos1 + 42..pos1 + 46].copy_from_slice(&0u32.to_le_bytes());
    // Patch EOCD: disk_number = 1, cd_start_disk = 1, cd_offset = cd_offset - adj
    let eocd_rel = eocd_pos - adj;
    zip_last[eocd_rel + 4..eocd_rel + 6].copy_from_slice(&1u16.to_le_bytes());
    zip_last[eocd_rel + 6..eocd_rel + 8].copy_from_slice(&1u16.to_le_bytes());
    zip_last[eocd_rel + 16..eocd_rel + 20].copy_from_slice(&((cd_offset - adj) as u32).to_le_bytes());

    let z01_path = root.join("split.z01");
    let zip_path = root.join("split.zip");
    fs::write(&z01_path, &z01).unwrap();
    fs::write(&zip_path, &zip_last).unwrap();

    let info = list(&zip_path, &ListOptions::default()).unwrap();
    assert_eq!(info.format, Format::Zip);
    assert_eq!(info.volumes.len(), 2);
    assert_eq!(info.entries.len(), 2);

    let ex = root.join("ex");
    extract(&z01_path, &ExtractOptions::new(&ex), None).unwrap();
    assert_eq!(fs::read(ex.join("one.txt")).unwrap(), fs::read(&f1).unwrap());
    assert_eq!(fs::read(ex.join("two.txt")).unwrap(), fs::read(&f2).unwrap());
}

// ---------------------------------------------------------------------------
// 4. Expanded formats: AR, DEB, CPIO, RPM, CAB, CRX
// ---------------------------------------------------------------------------

fn write_ar_member(out: &mut Vec<u8>, name: &str, mtime: u64, mode: u32, data: &[u8]) {
    let mut hdr = [b' '; 60];
    hdr[..name.len().min(16)].copy_from_slice(&name.as_bytes()[..name.len().min(16)]);
    let mt = format!("{mtime}");
    hdr[16..16 + mt.len()].copy_from_slice(mt.as_bytes());
    hdr[28..32].copy_from_slice(b"1000");
    hdr[34..38].copy_from_slice(b"1000");
    let md = format!("{mode:o}");
    hdr[40..40 + md.len()].copy_from_slice(md.as_bytes());
    let sz = format!("{}", data.len());
    hdr[48..48 + sz.len()].copy_from_slice(sz.as_bytes());
    hdr[58..60].copy_from_slice(b"`\n");
    out.extend_from_slice(&hdr);
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(b'\n');
    }
}

#[test]
fn ar_and_deb_list_read_test_and_extract() {
    let root = tmp("ar_deb");

    // 1. Plain Unix `.ar` archive with GNU long name table (`//`) and BSD `#1/` member
    let mut ar_bytes = b"!<arch>\n".to_vec();
    write_ar_member(&mut ar_bytes, "//", 0, 0, b"very_long_member_name.txt/\n");
    write_ar_member(&mut ar_bytes, "/0", 1_700_000_000, 0o100644, b"hello from gnu long ar!");
    let bsd_name = b"bsd_extended_file.txt";
    let mut bsd_payload = bsd_name.to_vec();
    bsd_payload.extend_from_slice(b"hello from bsd ar!");
    write_ar_member(
        &mut ar_bytes,
        &format!("#1/{}", bsd_name.len()),
        1_700_000_000,
        0o100644,
        &bsd_payload,
    );

    let ar_path = root.join("libtest.a");
    fs::write(&ar_path, &ar_bytes).unwrap();

    let ar_info = list(&ar_path, &ListOptions::default()).unwrap();
    assert_eq!(ar_info.format, Format::Ar);
    assert_eq!(ar_info.entries.len(), 2);
    assert_eq!(ar_info.entries[0].path, "very_long_member_name.txt");
    assert_eq!(ar_info.entries[1].path, "bsd_extended_file.txt");

    let ar_read = read_entry(&ar_path, "bsd_extended_file.txt", &ReadOptions::default()).unwrap();
    assert_eq!(ar_read.bytes, b"hello from bsd ar!");
    assert!(test(&ar_path, &ListOptions::default(), None).unwrap().ok);

    let ex_ar = root.join("ex_ar");
    extract(&ar_path, &ExtractOptions::new(&ex_ar), None).unwrap();
    assert_eq!(
        fs::read(ex_ar.join("very_long_member_name.txt")).unwrap(),
        b"hello from gnu long ar!"
    );

    // 2. Debian `.deb` package (`debian-binary` + `control.tar.gz` + `data.tar.gz`)
    let make_tar_gz = |entries: &[(&str, &[u8])]| -> Vec<u8> {
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = tar::Builder::new(&mut gz);
            for (path, content) in entries {
                let mut hdr = tar::Header::new_gnu();
                hdr.set_size(content.len() as u64);
                hdr.set_mode(0o644);
                hdr.set_mtime(1_700_000_000);
                hdr.set_cksum();
                builder.append_data(&mut hdr, *path, *content).unwrap();
            }
            builder.finish().unwrap();
        }
        gz.finish().unwrap()
    };

    let ctrl_tgz = make_tar_gz(&[("control", b"Package: hello-ziplark\nVersion: 0.4.0\n")]);
    let data_tgz = make_tar_gz(&[("usr/bin/hello", b"#!/bin/sh\necho hi\n")]);

    let mut deb_bytes = b"!<arch>\n".to_vec();
    write_ar_member(&mut deb_bytes, "debian-binary/", 1_700_000_000, 0o100644, b"2.0\n");
    write_ar_member(&mut deb_bytes, "control.tar.gz/", 1_700_000_000, 0o100644, &ctrl_tgz);
    write_ar_member(&mut deb_bytes, "data.tar.gz/", 1_700_000_000, 0o100644, &data_tgz);

    let deb_path = root.join("hello_0.4.0_amd64.deb");
    fs::write(&deb_path, &deb_bytes).unwrap();

    let deb_info = list(&deb_path, &ListOptions::default()).unwrap();
    assert_eq!(deb_info.format, Format::Deb);
    let paths: Vec<&str> = deb_info.entries.iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"DEBIAN/control"), "got {paths:?}");
    assert!(paths.contains(&"usr/bin/hello"), "got {paths:?}");

    let ctrl = read_entry(&deb_path, "DEBIAN/control", &ReadOptions::default()).unwrap();
    assert!(std::str::from_utf8(&ctrl.bytes).unwrap().contains("hello-ziplark"));

    let ex_deb = root.join("ex_deb");
    extract(&deb_path, &ExtractOptions::new(&ex_deb), None).unwrap();
    assert_eq!(
        fs::read(ex_deb.join("usr/bin/hello")).unwrap(),
        b"#!/bin/sh\necho hi\n"
    );
}

fn build_cpio_newc(entries: &[(&str, u32, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut ino = 1u32;
    let push_entry = |out: &mut Vec<u8>, ino: u32, name: &str, mode: u32, data: &[u8]| {
        let namesize = name.len() + 1;
        let hdr = format!(
            "070701{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}",
            ino,
            mode,
            1000,
            1000,
            1,
            1_700_000_000u32,
            data.len(),
            0,
            0,
            0,
            0,
            namesize,
            0
        );
        assert_eq!(hdr.len(), 110);
        out.extend_from_slice(hdr.as_bytes());
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    };
    for (name, mode, data) in entries {
        push_entry(&mut out, ino, name, *mode, data);
        ino += 1;
    }
    push_entry(&mut out, ino, "TRAILER!!!", 0, b"");
    out
}

#[test]
fn cpio_and_rpm_list_read_test_and_extract() {
    let root = tmp("cpio_rpm");
    let cpio_bytes = build_cpio_newc(&[
        ("etc/app.conf", 0o100644, b"port = 8080\n"),
        ("usr/bin/tool", 0o100755, b"#!/bin/sh\necho cpio\n"),
    ]);

    let cpio_path = root.join("archive.cpio");
    fs::write(&cpio_path, &cpio_bytes).unwrap();

    let info = list(&cpio_path, &ListOptions::default()).unwrap();
    assert_eq!(info.format, Format::Cpio);
    assert_eq!(info.entries.len(), 2);

    let read = read_entry(&cpio_path, "etc/app.conf", &ReadOptions::default()).unwrap();
    assert_eq!(read.bytes, b"port = 8080\n");
    assert!(test(&cpio_path, &ListOptions::default(), None).unwrap().ok);

    let ex_cpio = root.join("ex_cpio");
    extract(&cpio_path, &ExtractOptions::new(&ex_cpio), None).unwrap();
    assert_eq!(fs::read(ex_cpio.join("etc/app.conf")).unwrap(), b"port = 8080\n");

    // Build a synthetic RPM v3 file:
    // - 96-byte Lead (`\xED\xAB\xEE\xDB`...)
    // - Signature Header (`\x8E\xAD\xE8\x01\x00\x00\x00\x00` + nindex=0 + hsize=4 -> padded to 8 bytes)
    // - Main Header (`\x8E\xAD\xE8\x01\x00\x00\x00\x00` + nindex=0 + hsize=0)
    // - Gzipped CPIO payload
    let mut rpm = vec![0u8; 96];
    rpm[..4].copy_from_slice(&[0xED, 0xAB, 0xEE, 0xDB]);
    rpm[4] = 3; // major version 3
    // Signature header (16 bytes header + 4 bytes data + 4 bytes pad to 8-byte boundary)
    rpm.extend_from_slice(&[0x8E, 0xAD, 0xE8, 0x01, 0, 0, 0, 0]);
    rpm.extend_from_slice(&0u32.to_be_bytes()); // nindex = 0
    rpm.extend_from_slice(&4u32.to_be_bytes()); // hsize = 4
    rpm.extend_from_slice(&[0u8; 8]); // 4 bytes store + 4 bytes pad
    // Main header (16 bytes header, 0 index, 0 store)
    rpm.extend_from_slice(&[0x8E, 0xAD, 0xE8, 0x01, 0, 0, 0, 0]);
    rpm.extend_from_slice(&0u32.to_be_bytes());
    rpm.extend_from_slice(&0u32.to_be_bytes());
    // Gzipped CPIO payload
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&cpio_bytes).unwrap();
    rpm.extend_from_slice(&gz.finish().unwrap());

    let rpm_path = root.join("pkg-0.4.0-1.x86_64.rpm");
    fs::write(&rpm_path, &rpm).unwrap();

    let rpm_info = list(&rpm_path, &ListOptions::default()).unwrap();
    assert_eq!(rpm_info.format, Format::Rpm);
    assert_eq!(rpm_info.entries.len(), 2);

    let rpm_read = read_entry(&rpm_path, "usr/bin/tool", &ReadOptions::default()).unwrap();
    assert_eq!(rpm_read.bytes, b"#!/bin/sh\necho cpio\n");

    let ex_rpm = root.join("ex_rpm");
    extract(&rpm_path, &ExtractOptions::new(&ex_rpm), None).unwrap();
    assert_eq!(
        fs::read(ex_rpm.join("usr/bin/tool")).unwrap(),
        b"#!/bin/sh\necho cpio\n"
    );
}

#[test]
fn cab_mszip_list_read_test_and_extract() {
    let root = tmp("cab");
    let file_a = b"Hello from inside a Microsoft Cabinet (MSZIP) file!\n";
    let file_b = b"Second file inside the same CAB folder.\n";
    let mut folder_uncomp = Vec::new();
    folder_uncomp.extend_from_slice(file_a);
    folder_uncomp.extend_from_slice(file_b);

    // Compress folder_uncomp as an MSZIP CFDATA block (`CK` + raw Deflate stream)
    let mut def = DeflateEncoder::new(Vec::new(), Compression::default());
    def.write_all(&folder_uncomp).unwrap();
    let deflated = def.finish().unwrap();
    let mut cfdata_payload = Vec::with_capacity(2 + deflated.len());
    cfdata_payload.extend_from_slice(b"CK");
    cfdata_payload.extend_from_slice(&deflated);

    // Layout:
    // 0..36: CFHEADER (1 folder, 2 files)
    // 36..44: CFFOLDER (1 data block, comp_type = 1 [MSZIP])
    // 44..: 2 CFFILE records
    // ..: 1 CFDATA record
    let cffile_entry = |name: &str, size: u32, folder_off: u32| -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&size.to_le_bytes());
        v.extend_from_slice(&folder_off.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes()); // iFolder = 0
        v.extend_from_slice(&0x5421u16.to_le_bytes()); // date
        v.extend_from_slice(&0x6000u16.to_le_bytes()); // time
        v.extend_from_slice(&0x20u16.to_le_bytes()); // attribs
        v.extend_from_slice(name.as_bytes());
        v.push(0);
        v
    };
    let cff_a = cffile_entry("readme.txt", file_a.len() as u32, 0);
    let cff_b = cffile_entry("sub\\notes.txt", file_b.len() as u32, file_a.len() as u32);
    let coff_files = 36u32 + 8;
    let coff_data = coff_files + (cff_a.len() + cff_b.len()) as u32;
    let cb_cabinet = coff_data + 8 + cfdata_payload.len() as u32;

    let mut cab = Vec::with_capacity(cb_cabinet as usize);
    // CFHEADER (36 bytes)
    cab.extend_from_slice(b"MSCF");
    cab.extend_from_slice(&0u32.to_le_bytes());
    cab.extend_from_slice(&cb_cabinet.to_le_bytes());
    cab.extend_from_slice(&0u32.to_le_bytes());
    cab.extend_from_slice(&coff_files.to_le_bytes());
    cab.extend_from_slice(&0u32.to_le_bytes());
    cab.push(3); // versionMinor
    cab.push(1); // versionMajor
    cab.extend_from_slice(&1u16.to_le_bytes()); // cFolders = 1
    cab.extend_from_slice(&2u16.to_le_bytes()); // cFiles = 2
    cab.extend_from_slice(&0u16.to_le_bytes()); // flags = 0
    cab.extend_from_slice(&1u16.to_le_bytes()); // setID
    cab.extend_from_slice(&0u16.to_le_bytes()); // iCabinet
    // CFFOLDER (8 bytes)
    cab.extend_from_slice(&coff_data.to_le_bytes());
    cab.extend_from_slice(&1u16.to_le_bytes()); // cCFData = 1
    cab.extend_from_slice(&1u16.to_le_bytes()); // typeCompress = 1 (MSZIP)
    // CFFILEs
    cab.extend_from_slice(&cff_a);
    cab.extend_from_slice(&cff_b);
    // CFDATA (8-byte header + payload)
    cab.extend_from_slice(&0u32.to_le_bytes()); // csum = 0 (not computed)
    cab.extend_from_slice(&(cfdata_payload.len() as u16).to_le_bytes()); // cbData
    cab.extend_from_slice(&(folder_uncomp.len() as u16).to_le_bytes()); // cbUncomp
    cab.extend_from_slice(&cfdata_payload);

    let cab_path = root.join("setup.cab");
    fs::write(&cab_path, &cab).unwrap();

    let info = list(&cab_path, &ListOptions::default()).unwrap();
    assert_eq!(info.format, Format::Cab);
    assert_eq!(info.entries.len(), 2);
    assert_eq!(info.entries[0].path, "readme.txt");
    assert_eq!(info.entries[1].path, "sub/notes.txt");

    let rd = read_entry(&cab_path, "sub/notes.txt", &ReadOptions::default()).unwrap();
    assert_eq!(rd.bytes, file_b);
    assert!(test(&cab_path, &ListOptions::default(), None).unwrap().ok);

    let ex = root.join("ex_cab");
    extract(&cab_path, &ExtractOptions::new(&ex), None).unwrap();
    assert_eq!(fs::read(ex.join("readme.txt")).unwrap(), file_a);
    assert_eq!(fs::read(ex.join("sub/notes.txt")).unwrap(), file_b);
}

#[test]
fn crx_and_package_extensions_open_as_zip() {
    let root = tmp("crx_pkg");
    let manifest = root.join("manifest.json");
    fs::write(&manifest, br#"{"name":"ziplark-ext","version":"0.4.0"}"#).unwrap();

    // 1. Standard package extension (.whl)
    let whl = root.join("ziplark-0.4.0-py3-none-any.whl");
    create(&whl, std::slice::from_ref(&manifest), &CreateOptions::new(Format::Zip), None).unwrap();
    assert_eq!(detect(&whl).unwrap(), Format::Zip);
    assert_eq!(detect_by_extension("app.aab"), Some(Format::Zip));
    assert_eq!(detect_by_extension("pkg.srpm"), Some(Format::Rpm));
    assert_eq!(detect_by_extension("foo.lib"), Some(Format::Ar));

    // 2. Chrome Extension `.crx` (Cr24 v3 header + arbitrary protobuf header + ZIP payload)
    let zip_bytes = fs::read(&whl).unwrap();
    let fake_crx_hdr = b"fake-crx3-public-key-header-bytes";
    let mut crx_bytes = Vec::new();
    crx_bytes.extend_from_slice(b"Cr24");
    crx_bytes.extend_from_slice(&3u32.to_le_bytes()); // version 3
    crx_bytes.extend_from_slice(&(fake_crx_hdr.len() as u32).to_le_bytes());
    crx_bytes.extend_from_slice(fake_crx_hdr);
    crx_bytes.extend_from_slice(&zip_bytes);

    let crx = root.join("extension.crx");
    fs::write(&crx, &crx_bytes).unwrap();

    let info = list(&crx, &ListOptions::default()).unwrap();
    assert_eq!(info.format, Format::Zip);
    assert_eq!(info.entries.len(), 1);
    assert_eq!(info.entries[0].path, "manifest.json");

    let rd = read_entry(&crx, "manifest.json", &ReadOptions::default()).unwrap();
    assert_eq!(rd.bytes, fs::read(&manifest).unwrap());
}

// ---------------------------------------------------------------------------
// 5. In-place archive append (`add`)
// ---------------------------------------------------------------------------

#[test]
fn add_appends_and_replaces_entries_in_zip_7z_and_targz() {
    let root = tmp("add_append");
    let f1 = root.join("first.txt");
    let f2 = root.join("second.txt");
    fs::write(&f1, b"initial first").unwrap();
    fs::write(&f2, b"added second").unwrap();

    for (fmt, ext) in [
        (Format::Zip, "zip"),
        (Format::SevenZ, "7z"),
        (Format::TarGz, "tar.gz"),
    ] {
        let archive = root.join(format!("bundle.{ext}"));
        create(&archive, std::slice::from_ref(&f1), &CreateOptions::new(fmt), None).unwrap();

        // Append `second.txt`
        add(&archive, std::slice::from_ref(&f2), &CreateOptions::new(fmt), None).unwrap();

        let info = list(&archive, &ListOptions::default()).unwrap();
        let names: Vec<&str> = info.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(names.contains(&"first.txt"), "{ext}: missing first.txt ({names:?})");
        assert!(names.contains(&"second.txt"), "{ext}: missing second.txt ({names:?})");

        let r1 = read_entry(&archive, "first.txt", &ReadOptions::default()).unwrap();
        let r2 = read_entry(&archive, "second.txt", &ReadOptions::default()).unwrap();
        assert_eq!(r1.bytes, b"initial first");
        assert_eq!(r2.bytes, b"added second");
    }
}
