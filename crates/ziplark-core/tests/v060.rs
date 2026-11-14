//! Integration tests for Ziplark v0.6.0 features:
//! - In-archive atomic rename/move (`rename`) across `.zip`, `.7z`, and `.tar.*`
//! - Archive diff (`diff`) across formats with optional unified text diff patches
//! - Streaming entry checksum calculation (`hash`) for SHA-256, SHA-1, and CRC32
//! - Multi-volume split archive creation (`CreateOptions::split_size` -> `.001`, `.002`, …)
//!   and seamless multi-volume reading for `.zip.001`, `.7z.001`, and `.tar.zst.001`
//! - Pure-Rust XAR (`.xar` / `.xip`) and macOS Installer Package (`.pkg`) reader with
//!   transparent CPIO `Payload` / `Scripts` traversal

use flate2::write::{GzEncoder, ZlibEncoder};
use flate2::Compression;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use ziplark_core::*;

fn tmpdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ziplark-v060-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// 1. In-archive atomic rename / move (`rename`)
// ---------------------------------------------------------------------------

#[test]
fn rename_moves_files_and_folders_in_zip_7z_and_tar_gz() {
    let dir = tmpdir("rename-all");
    let root = dir.join("app");
    fs::create_dir_all(root.join("old_dir")).unwrap();
    fs::write(root.join("old_name.txt"), b"hello rename world").unwrap();
    fs::write(root.join("old_dir/inner.txt"), b"inside directory").unwrap();
    fs::write(root.join("untouched.txt"), b"stay put").unwrap();

    for (ext, fmt) in [
        ("zip", Format::Zip),
        ("7z", Format::SevenZ),
        ("tar.gz", Format::TarGz),
    ] {
        let arc = dir.join(format!("app.{ext}"));
        create(&arc, std::slice::from_ref(&root), &CreateOptions::new(fmt), None).unwrap();

        // 1. Rename a single file
        let r1 = rename(
            &arc,
            "app/old_name.txt",
            "app/new_name.txt",
            &RenameOptions {
                password: None,
                level: Level::Fast,
            },
            None,
        )
        .unwrap();
        assert_eq!(r1.entries_renamed, 1, "format={ext}");

        // 2. Rename a directory prefix (`app/old_dir` -> `app/moved_dir`)
        let r2 = rename(
            &arc,
            "app/old_dir",
            "app/moved_dir",
            &RenameOptions {
                password: None,
                level: Level::Fast,
            },
            None,
        )
        .unwrap();
        assert!(r2.entries_renamed >= 1, "format={ext}");

        let info = list(&arc, &ListOptions::default()).unwrap();
        let paths: Vec<&str> = info.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(
            paths.contains(&"app/new_name.txt"),
            "missing app/new_name.txt in {ext}: {paths:?}"
        );
        assert!(
            !paths.contains(&"app/old_name.txt"),
            "old_name.txt still present in {ext}: {paths:?}"
        );
        assert!(
            paths.contains(&"app/moved_dir/inner.txt"),
            "missing app/moved_dir/inner.txt in {ext}: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains("old_dir")),
            "old_dir still present in {ext}: {paths:?}"
        );

        let c1 = read_entry(&arc, "app/new_name.txt", &ReadOptions::default()).unwrap();
        assert_eq!(c1.bytes, b"hello rename world");
        let c2 = read_entry(&arc, "app/moved_dir/inner.txt", &ReadOptions::default()).unwrap();
        assert_eq!(c2.bytes, b"inside directory");
    }

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2. Archive Diff (`diff`) with unified text patches
// ---------------------------------------------------------------------------

#[test]
fn diff_compares_archives_across_formats_and_produces_unified_patches() {
    let dir = tmpdir("diff");
    let v1 = dir.join("v1");
    let v2 = dir.join("v2");
    fs::create_dir_all(&v1).unwrap();
    fs::create_dir_all(&v2).unwrap();

    fs::write(v1.join("same.txt"), b"unchanged content\n").unwrap();
    fs::write(v2.join("same.txt"), b"unchanged content\n").unwrap();

    fs::write(v1.join("removed.txt"), b"only in v1\n").unwrap();
    fs::write(v2.join("added.txt"), b"only in v2\n").unwrap();

    fs::write(v1.join("config.toml"), b"port = 8080\ndebug = false\n").unwrap();
    fs::write(v2.join("config.toml"), b"port = 9090\ndebug = false\n").unwrap();

    let left_zip = dir.join("left.zip");
    let right_tzst = dir.join("right.tar.zst");

    create(
        &left_zip,
        &[
            v1.join("same.txt"),
            v1.join("removed.txt"),
            v1.join("config.toml"),
        ],
        &CreateOptions::new(Format::Zip),
        None,
    )
    .unwrap();
    create(
        &right_tzst,
        &[
            v2.join("same.txt"),
            v2.join("added.txt"),
            v2.join("config.toml"),
        ],
        &CreateOptions::new(Format::TarZst),
        None,
    )
    .unwrap();

    let rep = diff(
        &left_zip,
        &right_tzst,
        &DiffOptions {
            content: true,
            ..DiffOptions::default()
        },
        None,
    )
    .unwrap();

    assert_eq!(rep.added, 1);
    assert_eq!(rep.removed, 1);
    assert_eq!(rep.modified, 1);
    assert_eq!(rep.unchanged, 1);
    assert_eq!(rep.changes.len(), 3);

    let mod_entry = rep
        .changes
        .iter()
        .find(|e| e.path == "config.toml")
        .unwrap();
    assert_eq!(mod_entry.status, DiffStatus::Modified);
    let patch = mod_entry.patch.as_deref().unwrap();
    assert!(patch.contains("-port = 8080"), "patch was:\n{patch}");
    assert!(patch.contains("+port = 9090"), "patch was:\n{patch}");

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3. Checksum Manifest (`hash`)
// ---------------------------------------------------------------------------

#[test]
fn hash_computes_sha256_sha1_and_crc32_without_extracting() {
    let dir = tmpdir("hash");
    let f = dir.join("hello.txt");
    fs::write(&f, b"hello world").unwrap();
    let zip_path = dir.join("hello.zip");
    create(&zip_path, std::slice::from_ref(&f), &CreateOptions::new(Format::Zip), None).unwrap();

    // Known digests for "hello world":
    // SHA-256: b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
    // SHA-1:   2aae6c35c94fcfb415dbe95f408b9ce91ee846ed
    // CRC32:   0d4a1185
    let r256 = hash(
        &zip_path,
        &HashOptions {
            algorithm: HashAlgorithm::Sha256,
            ..HashOptions::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(r256.files_hashed, 1);
    assert_eq!(
        r256.entries[0].digest,
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
    );

    let r1 = hash(
        &zip_path,
        &HashOptions {
            algorithm: HashAlgorithm::Sha1,
            ..HashOptions::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(r1.entries[0].digest, "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed");

    let rcrc = hash(
        &zip_path,
        &HashOptions {
            algorithm: HashAlgorithm::Crc32,
            ..HashOptions::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(rcrc.entries[0].digest, "0d4a1185");

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4. Multi-volume split archive creation (`split_size`) & extraction
// ---------------------------------------------------------------------------

#[test]
fn create_with_split_size_produces_volumes_and_roundtrips() {
    let dir = tmpdir("split-create");
    let src = dir.join("payload.bin");
    // 220 KB pseudo-random payload so compression doesn't shrink it below 64 KiB split threshold
    let mut data = Vec::with_capacity(220_000);
    let mut state = 0x1234_5678u32;
    for _ in 0..220_000 {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        data.push((state >> 24) as u8);
    }
    fs::write(&src, &data).unwrap();

    for (ext, fmt) in [("zip", Format::Zip), ("tar.zst", Format::TarZst)] {
        let out = dir.join(format!("archive.{ext}"));
        let rep = create(
            &out,
            std::slice::from_ref(&src),
            &CreateOptions {
                format: fmt,
                level: Level::Store,
                password: None,
                split_size: Some(64 * 1024), // 64 KiB per volume -> ~4 volumes
            },
            None,
        )
        .unwrap();
        assert!(
            rep.volumes.len() >= 3,
            "{ext}: expected >= 3 volumes, got {:?}",
            rep.volumes
        );
        assert!(!out.exists(), "{ext}: unsplit file should have been removed");
        assert!(rep.volumes[0].exists());

        // List & extract by passing `.001`
        let info = list(&rep.volumes[0], &ListOptions::default()).unwrap();
        assert_eq!(info.format, fmt);
        assert_eq!(info.volumes.len(), rep.volumes.len());

        let ex_dir = dir.join(format!("ex_{ext}"));
        extract(&rep.volumes[0], &ExtractOptions::new(&ex_dir), None).unwrap();
        assert_eq!(fs::read(ex_dir.join("payload.bin")).unwrap(), data);
    }

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 5. Pure-Rust `.xar` & macOS `.pkg` parser with nested CPIO `Payload`
// ---------------------------------------------------------------------------

fn build_cpio_odc(entries: &[(&str, u32, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut ino = 1u32;
    let push_entry = |out: &mut Vec<u8>, ino: u32, name: &str, mode: u32, data: &[u8]| {
        let namesize = name.len() + 1;
        let hdr = format!(
            "070707{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:011o}{:06o}{:011o}",
            1u32,
            ino,
            mode,
            0u32,
            0u32,
            1u32,
            0u32,
            1_700_000_000u64,
            namesize,
            data.len()
        );
        assert_eq!(hdr.len(), 76);
        out.extend_from_slice(hdr.as_bytes());
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        out.extend_from_slice(data);
    };
    for (name, mode, data) in entries {
        push_entry(&mut out, ino, name, *mode, data);
        ino += 1;
    }
    push_entry(&mut out, ino, "TRAILER!!!", 0, b"");
    out
}

#[test]
fn pkg_and_xar_list_read_test_and_extract_with_nested_cpio_payload() {
    let dir = tmpdir("xar-pkg");

    // 1. Build a gzipped CPIO odc payload representing `Applications/Demo.app/Contents/Info.plist`
    let plist_bytes = b"<?xml version=\"1.0\"?>\n<plist><dict><key>CFBundleName</key><string>Demo</string></dict></plist>\n";
    let cpio_raw = build_cpio_odc(&[(
        "./Applications/Demo.app/Contents/Info.plist",
        0o100644,
        plist_bytes,
    )]);
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&cpio_raw).unwrap();
    let payload_gz = gz.finish().unwrap();

    // 2. Build a zlib-compressed Distribution XML
    let dist_xml = b"<?xml version=\"1.0\"?>\n<installer-gui-script minSpecVersion=\"1\"/>\n";
    let mut zenc = ZlibEncoder::new(Vec::new(), Compression::default());
    zenc.write_all(dist_xml).unwrap();
    let dist_zlib = zenc.finish().unwrap();

    // Heap layout:
    // offset 0: dist_zlib
    // offset dist_zlib.len(): payload_gz
    let toc_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<xar>
  <toc>
    <file id="1">
      <name>Distribution</name>
      <type>file</type>
      <data>
        <offset>0</offset>
        <length>{dist_len}</length>
        <size>{dist_uncomp}</size>
        <encoding style="application/x-gzip"/>
      </data>
    </file>
    <file id="2">
      <name>Demo.pkg</name>
      <type>directory</type>
      <file id="3">
        <name>Payload</name>
        <type>file</type>
        <data>
          <offset>{pay_off}</offset>
          <length>{pay_len}</length>
          <size>{pay_uncomp}</size>
          <encoding style="application/octet-stream"/>
        </data>
      </file>
    </file>
  </toc>
</xar>"#,
        dist_len = dist_zlib.len(),
        dist_uncomp = dist_xml.len(),
        pay_off = dist_zlib.len(),
        pay_len = payload_gz.len(),
        pay_uncomp = payload_gz.len(),
    );

    let mut toc_z = ZlibEncoder::new(Vec::new(), Compression::default());
    toc_z.write_all(toc_xml.as_bytes()).unwrap();
    let toc_compressed = toc_z.finish().unwrap();

    // 28-byte XAR header:
    // 0..4: "xar!"
    // 4..6: header_size = 28 (u16 BE)
    // 6..8: version = 1 (u16 BE)
    // 8..16: toc_length_compressed (u64 BE)
    // 16..24: toc_length_uncompressed (u64 BE)
    // 24..28: checksum_alg = 1 (SHA1, u32 BE)
    let mut pkg_bytes = Vec::new();
    pkg_bytes.extend_from_slice(b"xar!");
    pkg_bytes.extend_from_slice(&28u16.to_be_bytes());
    pkg_bytes.extend_from_slice(&1u16.to_be_bytes());
    pkg_bytes.extend_from_slice(&(toc_compressed.len() as u64).to_be_bytes());
    pkg_bytes.extend_from_slice(&(toc_xml.len() as u64).to_be_bytes());
    pkg_bytes.extend_from_slice(&1u32.to_be_bytes());
    pkg_bytes.extend_from_slice(&toc_compressed);
    pkg_bytes.extend_from_slice(&dist_zlib);
    pkg_bytes.extend_from_slice(&payload_gz);

    let pkg_path = dir.join("DemoInstaller.pkg");
    fs::write(&pkg_path, &pkg_bytes).unwrap();

    assert_eq!(detect(&pkg_path), Some(Format::Pkg));
    let info = list(&pkg_path, &ListOptions::default()).unwrap();
    assert_eq!(info.format, Format::Pkg);
    let paths: Vec<&str> = info.entries.iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"Distribution"), "got {paths:?}");
    assert!(
        paths.contains(&"Applications/Demo.app/Contents/Info.plist"),
        "got {paths:?}"
    );

    // In-memory read of both the XAR-level XML file and the nested CPIO payload entry
    let r_dist = read_entry(&pkg_path, "Distribution", &ReadOptions::default()).unwrap();
    assert_eq!(r_dist.bytes, dist_xml);

    let r_plist = read_entry(
        &pkg_path,
        "Applications/Demo.app/Contents/Info.plist",
        &ReadOptions::default(),
    )
    .unwrap();
    assert_eq!(r_plist.bytes, plist_bytes);

    let trep = test(&pkg_path, &ListOptions::default(), None).unwrap();
    assert!(trep.ok, "bad entries: {:?}", trep.bad_entries);

    let ex_dir = dir.join("ex_pkg");
    let ex_rep = extract(&pkg_path, &ExtractOptions::new(&ex_dir), None).unwrap();
    assert!(ex_rep.files_written >= 2);
    assert_eq!(
        fs::read(ex_dir.join("Applications/Demo.app/Contents/Info.plist")).unwrap(),
        plist_bytes
    );

    let _ = fs::remove_dir_all(&dir);
}
