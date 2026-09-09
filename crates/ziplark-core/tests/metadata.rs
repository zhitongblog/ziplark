//! What an archive has to carry besides the bytes: permissions, modification
//! times, and symlinks.
//!
//! Losing these is quiet — the files are all there and the contents match — but
//! it means an extracted binary or script is not executable, a build system
//! sees every file as brand new, and a macOS `.app` or a `node_modules` tree
//! comes out as a pile of duplicated copies instead of the links that held it
//! together.

use ziplark_core::*;
use std::fs;
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ziplark-meta-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// 2020-01-02 03:04:05 UTC — comfortably in the past, so "not preserved" shows
/// up as today's date rather than something ambiguous.
const MTIME: i64 = 1_577_934_245;

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

fn mtime_of(path: &Path) -> i64 {
    let m = fs::symlink_metadata(path).unwrap().modified().unwrap();
    m.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

/// A tree with an executable, a plain file, and a symlink pointing inside it.
fn make_tree(root: &Path) -> PathBuf {
    let src = root.join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("run.sh"), b"#!/bin/sh\necho hi\n").unwrap();
    fs::write(src.join("sub/data.txt"), b"payload").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(src.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("sub/data.txt", src.join("link.txt")).unwrap();
    }

    let t = filetime::FileTime::from_unix_time(MTIME, 0);
    filetime::set_file_times(src.join("run.sh"), t, t).unwrap();
    filetime::set_file_times(src.join("sub/data.txt"), t, t).unwrap();
    src
}

fn roundtrip(root: &Path, format: Format, ext: &str) -> PathBuf {
    let src = make_tree(root);
    let archive = root.join(format!("out.{ext}"));
    create(&archive, &[src], &CreateOptions::new(format), None).unwrap();

    let dest = root.join("out");
    let mut opts = ExtractOptions::new(&dest);
    opts.overwrite = true;
    extract(&archive, &opts, None).unwrap();
    dest.join("src")
}

#[cfg(unix)]
#[test]
fn zip_keeps_the_executable_bit() {
    let root = tmp("zip-mode");
    let out = roundtrip(&root, Format::Zip, "zip");
    assert_eq!(mode_of(&out.join("run.sh")), 0o755, "executable bit lost");
    assert_eq!(mode_of(&out.join("sub/data.txt")), 0o644);
}

#[cfg(unix)]
#[test]
fn sevenz_keeps_the_executable_bit() {
    let root = tmp("7z-mode");
    let out = roundtrip(&root, Format::SevenZ, "7z");
    assert_eq!(mode_of(&out.join("run.sh")), 0o755, "executable bit lost");
}

#[cfg(unix)]
#[test]
fn tar_keeps_the_executable_bit() {
    let root = tmp("tar-mode");
    let out = roundtrip(&root, Format::Tar, "tar");
    assert_eq!(mode_of(&out.join("run.sh")), 0o755, "executable bit lost");
}

#[test]
fn zip_keeps_modification_times() {
    let root = tmp("zip-time");
    let out = roundtrip(&root, Format::Zip, "zip");
    assert_eq!(
        mtime_of(&out.join("sub/data.txt")),
        MTIME,
        "modification time not preserved"
    );
}

#[test]
fn sevenz_keeps_modification_times() {
    let root = tmp("7z-time");
    let out = roundtrip(&root, Format::SevenZ, "7z");
    assert_eq!(mtime_of(&out.join("sub/data.txt")), MTIME);
}

#[test]
fn tar_keeps_modification_times() {
    let root = tmp("tar-time");
    let out = roundtrip(&root, Format::Tar, "tar");
    assert_eq!(mtime_of(&out.join("sub/data.txt")), MTIME);
}

/// The timestamp has to survive as an *instant*, not as whatever the DOS field
/// happens to hold — that field has no time zone, so a writer that puts UTC in
/// it shifts every file by the local offset. ZIP's extended-timestamp field is
/// what makes this exact.
#[test]
fn zip_timestamps_are_exact_not_off_by_a_time_zone() {
    let root = tmp("zip-tz");
    let out = roundtrip(&root, Format::Zip, "zip");
    let delta = (mtime_of(&out.join("sub/data.txt")) - MTIME).abs();
    assert!(
        delta <= 1,
        "timestamp is off by {delta}s — that is a time-zone shift, not rounding"
    );
}

#[cfg(unix)]
#[test]
fn symlinks_stay_symlinks() {
    for (format, ext) in [
        (Format::Zip, "zip"),
        (Format::SevenZ, "7z"),
        (Format::Tar, "tar"),
    ] {
        let root = tmp(&format!("link-{ext}"));
        let out = roundtrip(&root, format, ext);
        let link = out.join("link.txt");

        let ty = fs::symlink_metadata(&link)
            .unwrap_or_else(|e| panic!("{ext}: link.txt missing: {e}"))
            .file_type();
        assert!(ty.is_symlink(), "{ext}: symlink came out as a regular file");
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("sub/data.txt"));
        assert_eq!(fs::read_to_string(&link).unwrap(), "payload");
    }
}

/// Following symlinks while walking the input is not just wasteful — a link
/// that points back up its own tree makes the walk run forever.
#[cfg(unix)]
#[test]
fn a_symlink_loop_does_not_hang_the_walk() {
    let root = tmp("loop");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), b"hi").unwrap();
    std::os::unix::fs::symlink(&src, src.join("self")).unwrap();

    let archive = root.join("loop.zip");
    create(&archive, &[src], &CreateOptions::new(Format::Zip), None).unwrap();

    let info = list(&archive, &ListOptions::default()).unwrap();
    assert_eq!(info.entries.len(), 2, "expected a.txt and the link itself");
}

/// A symlink must be stored as a link, not as a second copy of its target.
#[cfg(unix)]
#[test]
fn symlinks_are_not_silently_dereferenced_into_copies() {
    let root = tmp("no-deref");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    let big = vec![b'x'; 64 * 1024];
    fs::write(src.join("big.bin"), &big).unwrap();
    std::os::unix::fs::symlink("big.bin", src.join("alias.bin")).unwrap();

    let archive = root.join("links.zip");
    let mut opts = CreateOptions::new(Format::Zip);
    opts.level = Level::Store; // no compression, so sizes are directly comparable
    create(&archive, &[src], &opts, None).unwrap();

    let info = list(&archive, &ListOptions::default()).unwrap();
    let alias = info
        .entries
        .iter()
        .find(|e| e.path.ends_with("alias.bin"))
        .expect("alias.bin missing from the archive");
    assert!(
        alias.size < 64,
        "the link was stored as a {}-byte copy of its target",
        alias.size
    );
}
