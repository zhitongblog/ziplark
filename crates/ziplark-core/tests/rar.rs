//! RAR, the format people actually arrive with: split into volumes, solid,
//! header-encrypted, and often damaged.
//!
//! Every fixture here is a real archive written by the official `rar` tool —
//! there is no Rust RAR writer, so they are committed rather than built at test
//! time. `scripts/make-rar-fixtures.sh` regenerates them and says what each one
//! contains.

use std::fs;
use std::path::{Path, PathBuf};
use ziplark_core::*;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ziplark-rar-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn names(info: &ArchiveInfo) -> Vec<String> {
    let mut v: Vec<String> = info.entries.iter().map(|e| e.path.clone()).collect();
    v.sort();
    v
}

fn plain(path: &Path) -> ArchiveInfo {
    list(path, &ListOptions::default()).unwrap()
}

/// Copy a set of fixture files into a scratch directory, so tests that need an
/// *incomplete* volume set can delete one without touching the fixtures.
fn stage(dir: &Path, files: &[&str]) {
    for f in files {
        fs::copy(fixture(f), dir.join(f)).unwrap();
    }
}

// ───────────────────────────── volumes ─────────────────────────────

#[test]
fn opening_any_volume_lists_the_whole_set() {
    let first = plain(&fixture("multi.part1.rar"));
    assert_eq!(
        names(&first),
        vec!["tree", "tree/big.bin", "tree/docs", "tree/docs/notes.txt", "tree/docs/readme.txt"]
    );

    // Double-clicking part 2 or part 3 has to behave the same as part 1: the
    // archive is the set, not the file that was opened.
    for part in ["multi.part2.rar", "multi.part3.rar"] {
        let info = plain(&fixture(part));
        assert_eq!(names(&info), names(&first), "opened via {part}");
        assert_eq!(info.path, fixture("multi.part1.rar"), "resolved first volume");
        assert_eq!(info.volumes.len(), 3, "{part}: volumes reported");
        assert_eq!(info.missing_volume, None, "{part}: nothing missing");
    }
}

#[test]
fn a_legacy_volume_set_starts_at_rar_not_r01() {
    // `legacy.rar` + `legacy.r00` + `legacy.r01`: the first volume is the one
    // with no number in its name, which is what makes "part 1" arithmetic wrong.
    let info = plain(&fixture("legacy.r01"));
    assert_eq!(info.path, fixture("legacy.rar"));
    assert_eq!(
        info.volumes,
        vec![fixture("legacy.rar"), fixture("legacy.r00"), fixture("legacy.r01")]
    );
    assert!(names(&info).contains(&"tree/big.bin".to_string()));
}

#[test]
fn a_file_spanning_volumes_is_flagged_and_comes_out_whole() {
    let info = plain(&fixture("multi.part2.rar"));
    let big = info.entries.iter().find(|e| e.path == "tree/big.bin").unwrap();
    assert_eq!(big.size, 50_000);
    assert!(big.split, "big.bin spans volumes");
    let small = info.entries.iter().find(|e| e.path == "tree/docs/readme.txt").unwrap();
    assert!(!small.split, "a small file does not");

    // Extracting from the middle volume still reassembles the whole file.
    let dest = tmp("span");
    let report = extract(&fixture("multi.part2.rar"), &ExtractOptions::new(&dest), None).unwrap();
    assert_eq!(report.files_written, 3);
    assert_eq!(fs::metadata(dest.join("tree/big.bin")).unwrap().len(), 50_000);
    assert_eq!(fs::read_to_string(dest.join("tree/docs/readme.txt")).unwrap(), "hello from a rar fixture\n");
}

#[test]
fn an_incomplete_volume_set_names_the_missing_volume() {
    let dir = tmp("incomplete");
    stage(&dir, &["multi.part1.rar", "multi.part2.rar"]);
    let opened = dir.join("multi.part1.rar");

    let info = plain(&opened);
    assert_eq!(info.volumes.len(), 2);
    assert_eq!(
        info.missing_volume,
        Some(dir.join("multi.part3.rar")),
        "should name the volume it needs next"
    );

    // Extraction refuses up front rather than failing part-way through.
    let err = extract(&opened, &ExtractOptions::new(dir.join("out")), None);
    match err {
        Err(Error::MissingVolume(p)) => assert_eq!(p, dir.join("multi.part3.rar")),
        other => panic!("expected MissingVolume, got {other:?}"),
    }
    assert!(!dir.join("out/tree/big.bin").exists(), "nothing written");

    // Asked to keep what it can, it gets as far as the missing volume and says
    // so instead of failing. In this set the spanning file is first, so there is
    // nothing complete to recover — and nothing is claimed to be.
    let mut opts = ExtractOptions::new(dir.join("salvage"));
    opts.keep_broken = true;
    let report = extract(&opened, &opts, None).unwrap();
    assert_eq!(report.files_written, 0);
    assert!(
        report.failed.iter().any(|f| f.contains("multi.part3.rar")),
        "failures should name the missing volume: {:?}",
        report.failed
    );
}

#[test]
fn salvage_recovers_the_files_that_are_complete_in_the_volumes_on_hand() {
    // `salvage.*` stores the small files before the one that spans volumes, so
    // losing the last volume — a half-finished download — still leaves the
    // early files whole. That is the case worth recovering.
    let dir = tmp("salvage-set");
    stage(&dir, &["salvage.part1.rar", "salvage.part2.rar"]);
    let opened = dir.join("salvage.part1.rar");

    let mut opts = ExtractOptions::new(dir.join("out"));
    opts.keep_broken = true;
    let report = extract(&opened, &opts, None).unwrap();

    assert_eq!(
        fs::read_to_string(dir.join("out/docs/readme.txt")).unwrap(),
        "hello from a rar fixture\n",
        "the complete file should be recovered"
    );
    assert_eq!(report.files_written, 2, "readme.txt and notes.txt");
    assert!(
        report.failed.iter().any(|f| f.contains("salvage.part3.rar")),
        "and the truncated file accounted for: {:?}",
        report.failed
    );
    // Nothing is claimed for the file that could not be finished.
    assert!(report.bytes_written < 50_000);
}

#[test]
fn a_single_file_archive_reports_no_volumes() {
    let info = plain(&fixture("solid.rar"));
    assert!(info.volumes.is_empty());
    assert_eq!(info.missing_volume, None);
}

// ───────────────────────────── testing ─────────────────────────────

#[test]
fn testing_a_good_archive_writes_nothing() {
    let before = tmp("testclean");
    let report = test(&fixture("multi.part1.rar"), &ListOptions::default(), None).unwrap();
    assert!(report.ok, "{:?}", report.bad_entries);
    assert_eq!(report.entries_tested, 3, "three files, directories excluded");

    // This used to extract the entire archive to a temp directory to check it.
    let stale = std::env::temp_dir().join(format!("ziplark-rartest-{}", std::process::id()));
    assert!(!stale.exists(), "testing must not extract to {}", stale.display());
    assert_eq!(fs::read_dir(&before).unwrap().count(), 0);
}

#[test]
fn testing_a_damaged_archive_names_the_bad_entry_and_clears_the_rest() {
    let report = test(&fixture("damaged.rar"), &ListOptions::default(), None).unwrap();
    assert!(!report.ok);
    assert_eq!(report.bad_entries.len(), 1, "{:?}", report.bad_entries);
    assert!(
        report.bad_entries[0].contains("big.bin"),
        "should name the entry: {:?}",
        report.bad_entries
    );
    // The point of resuming: the two intact files are still verified.
    assert_eq!(report.entries_tested, 2);
}

#[test]
fn test_reports_progress_and_can_be_cancelled() {
    let mut seen = Vec::new();
    let mut watch = |p: Progress| {
        seen.push(p.current_path);
        true
    };
    test(&fixture("multi.part1.rar"), &ListOptions::default(), Some(&mut watch)).unwrap();
    assert_eq!(seen.len(), 3);

    let mut stop_at_once = |_: Progress| false;
    let err = test(
        &fixture("multi.part1.rar"),
        &ListOptions::default(),
        Some(&mut stop_at_once),
    );
    assert!(matches!(err, Err(Error::Cancelled)), "got {err:?}");
}

// ───────────────────────────── damage ─────────────────────────────

#[test]
fn a_damaged_archive_stops_by_default_and_salvages_on_request() {
    let dest = tmp("damaged-strict");
    let err = extract(&fixture("damaged.rar"), &ExtractOptions::new(&dest), None);
    assert!(matches!(err, Err(Error::Corrupt(_))), "got {err:?}");

    let dest = tmp("damaged-salvage");
    let mut opts = ExtractOptions::new(&dest);
    opts.keep_broken = true;
    let report = extract(&fixture("damaged.rar"), &opts, None).unwrap();
    assert_eq!(report.files_written, 2, "the two intact files came out");
    assert_eq!(report.failed.len(), 1, "{:?}", report.failed);
    assert!(report.failed[0].contains("big.bin"));
    assert_eq!(
        fs::read_to_string(dest.join("tree/docs/readme.txt")).unwrap(),
        "hello from a rar fixture\n"
    );
}

// ───────────────────────────── safety ─────────────────────────────

#[test]
fn a_symlink_entry_cannot_be_used_to_write_outside_the_destination() {
    let target = PathBuf::from("/tmp/ziplark-rar-escape");
    let _ = fs::remove_dir_all(&target);
    let dest = tmp("escape");

    let err = extract(&fixture("linkescape.rar"), &ExtractOptions::new(&dest), None);
    assert!(matches!(err, Err(Error::PathTraversal(_))), "got {err:?}");
    assert!(
        !target.join("owned.txt").exists(),
        "an entry escaped to {}",
        target.display()
    );
    // The link itself is a faithful part of the archive and may exist inside
    // the destination; what must never happen is a write *through* it.
    assert!(!target.exists() || fs::read_dir(&target).unwrap().count() == 0);
}

#[test]
fn a_directory_entry_does_not_follow_a_symlink_already_in_the_destination() {
    let target = tmp("planted-target");
    let dest = tmp("planted");
    // `solid.rar` contains a `docs` directory entry; stand a link there first.
    std::os::unix::fs::symlink(&target, dest.join("docs")).unwrap();

    let mut opts = ExtractOptions::new(&dest);
    opts.overwrite = true;
    let err = extract(&fixture("solid.rar"), &opts, None);
    assert!(matches!(err, Err(Error::PathTraversal(_))), "got {err:?}");
    assert_eq!(
        fs::read_dir(&target).unwrap().count(),
        0,
        "nothing should have been written through the link"
    );
}

// ───────────────────────────── metadata ─────────────────────────────

#[test]
fn solid_recovery_record_and_comment_are_reported() {
    let info = plain(&fixture("solid.rar"));
    assert!(info.attributes.solid, "fixture was written with -s");
    assert!(info.attributes.recovery_record, "written with -rr5p");
    assert!(!info.attributes.encrypted_headers);
    assert_eq!(
        info.comment.as_deref(),
        Some("Ziplark test fixture.\nSecond line."),
        "archive comment"
    );

    // An archive without a comment says so rather than inventing an empty one.
    assert_eq!(plain(&fixture("multi.part1.rar")).comment, None);
}

#[test]
fn a_header_encrypted_archive_needs_the_password_to_be_listed_at_all() {
    let err = list(fixture("hdrenc.rar"), &ListOptions::default());
    assert!(matches!(err, Err(Error::PasswordRequired)), "got {err:?}");

    let opts = ListOptions {
        password: Some("ziplark".into()),
    };
    let info = list(fixture("hdrenc.rar"), &opts).unwrap();
    assert!(info.attributes.encrypted_headers);
    assert!(info.encrypted);
    assert!(names(&info).contains(&"docs/readme.txt".to_string()));

    let dest = tmp("hdrenc");
    let mut xopts = ExtractOptions::new(&dest);
    xopts.password = Some("ziplark".into());
    extract(fixture("hdrenc.rar"), &xopts, None).unwrap();
    assert_eq!(
        fs::read_to_string(dest.join("docs/readme.txt")).unwrap(),
        "hello from a rar fixture\n"
    );

    let mut wrong = ExtractOptions::new(tmp("hdrenc-wrong"));
    wrong.password = Some("nope".into());
    let err = extract(fixture("hdrenc.rar"), &wrong, None);
    assert!(
        matches!(err, Err(Error::BadPassword) | Err(Error::PasswordRequired)),
        "got {err:?}"
    );
}

#[test]
fn timestamps_are_listed_and_restored() {
    let info = plain(&fixture("solid.rar"));
    let entry = info.entries.iter().find(|e| e.path == "docs/readme.txt").unwrap();
    let stamp = entry.modified.expect("RAR records a timestamp");
    // The fixtures were built in 2026; anything near the epoch means the DOS
    // date was decoded wrong.
    assert!(stamp > 1_700_000_000, "implausible timestamp {stamp}");

    let dest = tmp("mtime");
    extract(&fixture("solid.rar"), &ExtractOptions::new(&dest), None).unwrap();
    let on_disk = filetime::FileTime::from_last_modification_time(
        &fs::metadata(dest.join("docs/readme.txt")).unwrap(),
    )
    .unix_seconds();
    assert!(
        (on_disk - stamp).abs() <= 2,
        "extracted mtime {on_disk} should match the archive's {stamp}"
    );
}

#[test]
fn cjk_names_are_not_mangled() {
    let info = plain(&fixture("cjk.rar"));
    let listed = names(&info);
    assert!(listed.contains(&"cjk/中文文件.txt".to_string()), "{listed:?}");
    assert!(listed.contains(&"cjk/日本語のファイル.txt".to_string()), "{listed:?}");

    let dest = tmp("cjk");
    extract(&fixture("cjk.rar"), &ExtractOptions::new(&dest), None).unwrap();
    assert!(dest.join("cjk/中文文件.txt").exists(), "on-disk name is mojibake");
}

// ───────────────────────────── selection ─────────────────────────────

#[test]
fn an_exact_selection_extracts_exactly_that_entry() {
    let dest = tmp("select");
    let mut opts = ExtractOptions::new(&dest);
    opts.include = vec!["tree/docs/readme.txt".into()];
    opts.match_mode = MatchMode::Exact;
    let report = extract(&fixture("multi.part1.rar"), &opts, None).unwrap();

    assert_eq!(report.files_written, 1);
    assert!(dest.join("tree/docs/readme.txt").exists());
    assert!(!dest.join("tree/big.bin").exists());
    assert!(!dest.join("tree/docs/notes.txt").exists());
}

#[test]
fn a_glob_selection_works_too() {
    let dest = tmp("glob");
    let mut opts = ExtractOptions::new(&dest);
    opts.include = vec!["*/docs/*.txt".into()];
    let report = extract(&fixture("multi.part1.rar"), &opts, None).unwrap();
    assert_eq!(report.files_written, 2);
    assert!(!dest.join("tree/big.bin").exists());
}

#[test]
fn extraction_reports_progress_against_real_totals_and_can_be_cancelled() {
    let dest = tmp("progress");
    let mut totals = Vec::new();
    let mut watch = |p: Progress| {
        totals.push((p.entries_done, p.entries_total, p.bytes_total));
        true
    };
    extract(&fixture("multi.part1.rar"), &ExtractOptions::new(&dest), Some(&mut watch)).unwrap();
    assert_eq!(totals.len(), 3);
    // A progress bar needs a denominator, and RAR's headers have both: three
    // files totalling 50 000 + 25 + 18 bytes.
    assert!(
        totals.iter().all(|(_, n, b)| *n == 3 && *b == 50_043),
        "{totals:?}"
    );

    let dest = tmp("cancel");
    let mut stop = |_: Progress| false;
    let err = extract(&fixture("multi.part1.rar"), &ExtractOptions::new(&dest), Some(&mut stop));
    assert!(matches!(err, Err(Error::Cancelled)), "got {err:?}");
}

#[test]
fn permissions_and_links_survive_a_round_trip() {
    let info = plain(&fixture("perms.rar"));
    let dest = tmp("perms");
    extract(&fixture("perms.rar"), &ExtractOptions::new(&dest), None).unwrap();

    // An executable script has to come out executable; this is the metadata
    // loss that made extracted .app bundles and node_modules unusable.
    let mode = fs::metadata(dest.join("perms/run.sh"))
        .map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o777)
        .unwrap();
    assert_eq!(mode, 0o755, "run.sh should still be executable");
    let plain_mode = fs::metadata(dest.join("perms/plain.txt"))
        .map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o777)
        .unwrap();
    assert_eq!(plain_mode, 0o644);

    // A symlink comes back as a symlink, pointing where it pointed — not as a
    // second copy of the file.
    let link = dest.join("perms/link.txt");
    assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), Path::new("plain.txt"));

    // And the listing knows the compressed size of each entry, which is what a
    // UI shows next to the original size.
    let entry = info.entries.iter().find(|e| e.path == "perms/plain.txt").unwrap();
    assert!(entry.compressed_size.is_some(), "packed size should be reported");
}

#[test]
fn a_link_entry_is_restored_by_us_rather_than_trusted_to_libunrar() {
    // The escaping archive again, this time salvaging: the link itself is a
    // faithful part of the archive and lands inside the destination, while the
    // entry that would have written *through* it is refused by name.
    let dest = tmp("link-restore");
    let mut opts = ExtractOptions::new(&dest);
    opts.keep_broken = true;
    let report = extract(&fixture("linkescape.rar"), &opts, None).unwrap();

    let link = dest.join("escape/evil");
    assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), Path::new("/tmp/ziplark-rar-escape"));
    assert!(
        report.failed.iter().any(|f| f.contains("owned.txt")),
        "the write through the link should be refused: {:?}",
        report.failed
    );
    assert!(!Path::new("/tmp/ziplark-rar-escape/owned.txt").exists());
}

#[test]
fn a_self_extracting_archive_opens_like_any_other() {
    // `sfx.exe` is a stub followed by a RAR payload — what every self-extracting
    // download looks like. Most modern tools refuse these; libunrar reads the
    // payload wherever it starts.
    let info = plain(&fixture("sfx.exe"));
    assert_eq!(info.format, Format::Rar);
    assert!(names(&info).contains(&"cjk/中文文件.txt".to_string()), "{:?}", names(&info));

    let dest = tmp("sfx");
    let report = extract(&fixture("sfx.exe"), &ExtractOptions::new(&dest), None).unwrap();
    assert_eq!(report.files_written, 2);
    assert!(dest.join("cjk/中文文件.txt").exists());
}
