//! Extraction must not write outside the destination directory — ever.
//!
//! The obvious attack is a `../../` in the entry name, which
//! `rejects_zip_slip` in roundtrip.rs covers. These tests cover the subtler
//! one: a name with no `..` in it at all that still escapes, because something
//! along its path is a *symlink*. tar is the format that makes this reachable
//! from an archive alone, since it is the only one of ours that stores links.

use ziplark_core::*;
use std::fs;
use std::path::{Path, PathBuf};

/// A scratch area holding a `dest` we extract into and an `outside` directory
/// that nothing we do may ever touch.
struct Sandbox {
    root: PathBuf,
    dest: PathBuf,
    outside: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "ziplark-safety-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let dest = root.join("dest");
        let outside = root.join("outside");
        fs::create_dir_all(&dest).unwrap();
        fs::create_dir_all(&outside).unwrap();
        Self { root, dest, outside }
    }

    fn archive(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// The whole point of every test in this file.
    fn assert_nothing_escaped(&self) {
        let leaked: Vec<_> = fs::read_dir(&self.outside)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            leaked.is_empty(),
            "extraction wrote outside the destination: {leaked:?}"
        );
    }
}

fn extract_into(archive: &Path, dest: &Path) -> Result<ExtractReport> {
    let mut opts = ExtractOptions::new(dest);
    opts.overwrite = true;
    extract(archive, &opts, None)
}

/// Build a tar from (entry, optional link target) pairs.
fn write_tar(path: &Path, build: impl FnOnce(&mut tar::Builder<fs::File>)) {
    let mut builder = tar::Builder::new(fs::File::create(path).unwrap());
    build(&mut builder);
    builder.finish().unwrap();
}

fn link_entry(kind: tar::EntryType, name: &str, target: &str) -> tar::Header {
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(kind);
    h.set_size(0);
    h.set_mode(0o777);
    h.set_path(name).unwrap();
    h.set_link_name(target).unwrap();
    h.set_cksum();
    h
}

fn file_entry(name: &str, len: u64) -> tar::Header {
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(tar::EntryType::Regular);
    h.set_size(len);
    h.set_mode(0o644);
    h.set_path(name).unwrap();
    h.set_cksum();
    h
}

/// The reported vulnerability: `evil -> /somewhere/else`, then `evil/owned.txt`.
/// Neither name contains `..`, so a name-only guard passes both, and the second
/// write lands wherever the link points.
#[cfg(unix)]
#[test]
fn tar_symlink_cannot_redirect_a_write_outside_dest() {
    let sb = Sandbox::new("tar-symlink-abs");
    let archive = sb.archive("evil.tar");
    let outside = sb.outside.to_string_lossy().to_string();
    write_tar(&archive, |b| {
        b.append(&link_entry(tar::EntryType::Symlink, "evil", &outside), &[][..])
            .unwrap();
        b.append(&file_entry("evil/owned.txt", 8), &b"ESCAPED!"[..])
            .unwrap();
    });

    let err = extract_into(&archive, &sb.dest);
    assert!(
        matches!(err, Err(Error::PathTraversal(_))),
        "expected the escape to be refused, got {err:?}"
    );
    sb.assert_nothing_escaped();
}

/// Same attack with a relative target, which is what actually shows up in the
/// wild since it survives being extracted anywhere.
#[cfg(unix)]
#[test]
fn tar_relative_symlink_cannot_redirect_a_write_outside_dest() {
    let sb = Sandbox::new("tar-symlink-rel");
    let archive = sb.archive("evil.tar");
    write_tar(&archive, |b| {
        b.append(
            &link_entry(tar::EntryType::Symlink, "evil", "../outside"),
            &[][..],
        )
        .unwrap();
        b.append(&file_entry("evil/owned.txt", 8), &b"ESCAPED!"[..])
            .unwrap();
    });

    let err = extract_into(&archive, &sb.dest);
    assert!(
        matches!(err, Err(Error::PathTraversal(_))),
        "expected the escape to be refused, got {err:?}"
    );
    sb.assert_nothing_escaped();
}

/// A hard link names a file *inside* the archive, so one pointing out of the
/// destination is always a lie.
#[test]
fn tar_hardlink_outside_dest_is_refused() {
    let sb = Sandbox::new("tar-hardlink");
    let secret = sb.outside.join("secret.txt");
    fs::write(&secret, b"private").unwrap();

    let archive = sb.archive("evil.tar");
    write_tar(&archive, |b| {
        b.append(
            &link_entry(tar::EntryType::Link, "grab.txt", "../outside/secret.txt"),
            &[][..],
        )
        .unwrap();
    });

    let err = extract_into(&archive, &sb.dest);
    assert!(
        matches!(err, Err(Error::PathTraversal(_))),
        "expected the hard link to be refused, got {err:?}"
    );
    assert!(!sb.dest.join("grab.txt").exists());
}

/// The guard must not break the ordinary case: links that stay inside the
/// destination are still restored as links, exactly as tar and bsdtar do.
#[cfg(unix)]
#[test]
fn tar_links_inside_dest_are_still_restored() {
    let sb = Sandbox::new("tar-good-links");
    let archive = sb.archive("good.tar");
    write_tar(&archive, |b| {
        b.append(&file_entry("sub/a.txt", 5), &b"hello"[..]).unwrap();
        b.append(
            &link_entry(tar::EntryType::Symlink, "link.txt", "sub/a.txt"),
            &[][..],
        )
        .unwrap();
        b.append(
            &link_entry(tar::EntryType::Link, "hard.txt", "sub/a.txt"),
            &[][..],
        )
        .unwrap();
    });

    extract_into(&archive, &sb.dest).unwrap();

    let link = sb.dest.join("link.txt");
    assert!(
        fs::symlink_metadata(&link).unwrap().file_type().is_symlink(),
        "symlink should have been restored as a symlink"
    );
    assert_eq!(fs::read_to_string(&link).unwrap(), "hello");
    assert_eq!(fs::read_to_string(sb.dest.join("hard.txt")).unwrap(), "hello");
    sb.assert_nothing_escaped();
}

/// An absolute symlink that points out of the destination is stored verbatim —
/// that much matches GNU tar, and packaging tarballs rely on it — but it must
/// not become a usable door for the entries that follow. The previous test
/// proves the door is shut; this one proves the link itself survives.
#[cfg(unix)]
#[test]
fn tar_symlink_pointing_outside_is_created_but_leads_nowhere_useful() {
    let sb = Sandbox::new("tar-symlink-alone");
    let archive = sb.archive("link.tar");
    write_tar(&archive, |b| {
        b.append(
            &link_entry(tar::EntryType::Symlink, "etc", "/etc"),
            &[][..],
        )
        .unwrap();
    });

    extract_into(&archive, &sb.dest).unwrap();
    let link = sb.dest.join("etc");
    assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), Path::new("/etc"));
}

// ─────────── the same hazard, from a symlink already on disk ───────────

/// The symlink does not have to come from the archive. A previous extraction,
/// or an attacker with write access to the destination, can leave one there.
/// Every format goes through the same guard, so ZIP is covered too.
#[cfg(unix)]
#[test]
fn zip_will_not_descend_through_a_symlink_already_in_dest() {
    let sb = Sandbox::new("zip-planted-dir");
    std::os::unix::fs::symlink(&sb.outside, sb.dest.join("sub")).unwrap();

    let archive = sb.archive("payload.zip");
    write_zip(&archive, &[("sub/owned.txt", b"ESCAPED!")]);

    let err = extract_into(&archive, &sb.dest);
    assert!(
        matches!(err, Err(Error::PathTraversal(_))),
        "expected the planted symlink to be refused, got {err:?}"
    );
    sb.assert_nothing_escaped();
}

/// A symlink sitting where a *file* is about to be written must be replaced,
/// not written through. `exists()` cannot see this coming when the link is
/// dangling, which is why the guard uses `symlink_metadata`.
#[cfg(unix)]
#[test]
fn extraction_replaces_a_symlink_at_the_leaf_instead_of_following_it() {
    let sb = Sandbox::new("zip-planted-leaf");
    let secret = sb.outside.join("secret.txt");
    fs::write(&secret, b"private").unwrap();
    std::os::unix::fs::symlink(&secret, sb.dest.join("a.txt")).unwrap();

    let archive = sb.archive("payload.zip");
    write_zip(&archive, &[("a.txt", b"from the archive")]);
    extract_into(&archive, &sb.dest).unwrap();

    assert_eq!(
        fs::read_to_string(&secret).unwrap(),
        "private",
        "the write followed the symlink out of the destination"
    );
    let written = sb.dest.join("a.txt");
    assert!(!fs::symlink_metadata(&written).unwrap().file_type().is_symlink());
    assert_eq!(fs::read_to_string(&written).unwrap(), "from the archive");
}

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    use std::io::Write;
    let mut w = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, data) in entries {
        w.start_file::<_, ()>(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap();
}
