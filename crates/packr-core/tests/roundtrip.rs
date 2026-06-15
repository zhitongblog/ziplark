//! End-to-end tests for the Packr engine: every create format round-trips,
//! encryption works, the zip-slip guard holds, and RAR extraction works.

use packr_core::*;
use std::fs;
use std::path::{Path, PathBuf};

/// Build a small source tree and return its directory.
fn make_src(root: &Path) -> PathBuf {
    let src = root.join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), b"hello packr").unwrap();
    fs::write(src.join("sub/b.txt"), b"second file, somewhat compressible content ".repeat(20))
        .unwrap();
    src
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("packr-it-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn dirs_equal(a: &Path, b: &Path) -> bool {
    fn collect(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if p.is_dir() {
                collect(base, &p, out);
            } else {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().to_string();
                out.push((rel, fs::read(&p).unwrap()));
            }
        }
    }
    let (mut va, mut vb) = (Vec::new(), Vec::new());
    collect(a, a, &mut va);
    collect(b, b, &mut vb);
    va.sort();
    vb.sort();
    va == vb
}

fn roundtrip(format: Format, ext: &str) {
    let root = tmp(ext);
    let src = make_src(&root);
    let archive = root.join(format!("out.{ext}"));

    let report = create(&archive, &[src.clone()], &CreateOptions::new(format), None).unwrap();
    assert!(report.entries_added >= 2, "{ext}: entries");
    assert!(archive.exists(), "{ext}: archive written");

    let info = list(&archive, &ListOptions::default()).unwrap();
    assert_eq!(info.format, format, "{ext}: detected format");

    let t = test(&archive, &ListOptions::default(), None).unwrap();
    assert!(t.ok, "{ext}: integrity test failed: {:?}", t.bad_entries);

    let ex = root.join("ex");
    extract(&archive, &ExtractOptions::new(&ex), None).unwrap();
    assert!(dirs_equal(&src, &ex.join("src")), "{ext}: roundtrip mismatch");
}

#[test]
fn roundtrip_zip() {
    roundtrip(Format::Zip, "zip");
}
#[test]
fn roundtrip_7z() {
    roundtrip(Format::SevenZ, "7z");
}
#[test]
fn roundtrip_tar() {
    roundtrip(Format::Tar, "tar");
}
#[test]
fn roundtrip_tar_gz() {
    roundtrip(Format::TarGz, "tar.gz");
}
#[test]
fn roundtrip_tar_bz2() {
    roundtrip(Format::TarBz2, "tar.bz2");
}
#[test]
fn roundtrip_tar_xz() {
    roundtrip(Format::TarXz, "tar.xz");
}
#[test]
fn roundtrip_tar_zst() {
    roundtrip(Format::TarZst, "tar.zst");
}

#[test]
fn zip_aes_roundtrip_and_wrong_password() {
    let root = tmp("aes");
    let src = make_src(&root);
    let archive = root.join("sec.zip");
    let mut opts = CreateOptions::new(Format::Zip);
    opts.password = Some("hunter2".into());
    create(&archive, &[src.clone()], &opts, None).unwrap();

    // No password -> PasswordRequired.
    let no_pw = extract(&archive, &ExtractOptions::new(root.join("no")), None);
    assert!(matches!(no_pw, Err(Error::PasswordRequired)));

    // Wrong password -> BadPassword.
    let mut bad = ExtractOptions::new(root.join("bad"));
    bad.password = Some("nope".into());
    assert!(matches!(extract(&archive, &bad, None), Err(Error::BadPassword)));

    // Correct password -> success.
    let mut good = ExtractOptions::new(root.join("good"));
    good.password = Some("hunter2".into());
    extract(&archive, &good, None).unwrap();
    assert!(dirs_equal(&src, &root.join("good/src")));
}

#[test]
fn sevenz_encrypted_roundtrip() {
    let root = tmp("7zaes");
    let src = make_src(&root);
    let archive = root.join("sec.7z");
    let mut opts = CreateOptions::new(Format::SevenZ);
    opts.password = Some("s3cret".into());
    create(&archive, &[src.clone()], &opts, None).unwrap();

    let mut good = ExtractOptions::new(root.join("good"));
    good.password = Some("s3cret".into());
    extract(&archive, &good, None).unwrap();
    assert!(dirs_equal(&src, &root.join("good/src")));
}

#[test]
fn single_stream_gz_roundtrip() {
    let root = tmp("gz");
    let file = root.join("data.txt");
    fs::write(&file, b"single stream payload ".repeat(100)).unwrap();
    let archive = root.join("data.txt.gz");
    create(&archive, &[file.clone()], &CreateOptions::new(Format::Gz), None).unwrap();

    let ex = root.join("ex");
    extract(&archive, &ExtractOptions::new(&ex), None).unwrap();
    assert_eq!(fs::read(&file).unwrap(), fs::read(ex.join("data.txt")).unwrap());
}

#[test]
fn rejects_zip_slip() {
    // A hand-built zip whose entry name escapes the destination must be refused.
    let root = tmp("slip");
    let archive = root.join("evil.zip");
    {
        use std::io::Write;
        let f = fs::File::create(&archive).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        zw.start_file::<_, ()>(
            "../../escape.txt",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zw.write_all(b"pwned").unwrap();
        zw.finish().unwrap();
    }
    let err = extract(&archive, &ExtractOptions::new(root.join("ex")), None);
    assert!(matches!(err, Err(Error::PathTraversal(_))), "got {err:?}");
}

#[test]
fn rar_list_and_extract() {
    let rar = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.rar");
    let info = list(&rar, &ListOptions::default()).unwrap();
    assert_eq!(info.format, Format::Rar);
    assert!(!info.entries.is_empty());

    let root = tmp("rar");
    let report = extract(&rar, &ExtractOptions::new(&root), None).unwrap();
    assert!(report.files_written >= 1);
}

#[test]
fn rar_create_is_unsupported() {
    let root = tmp("rarcreate");
    let src = make_src(&root);
    let err = create(root.join("x.rar"), &[src], &CreateOptions::new(Format::Rar), None);
    assert!(matches!(err, Err(Error::CreateUnsupported(_))));
}
