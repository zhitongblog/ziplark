use std::path::{Path, PathBuf};
use ziplark_core::{
    convert, create, extract, grep, list, read_entry, remove, ConvertOptions, CreateOptions,
    ExtractOptions, Format, GrepOptions, Level, ListOptions, MatchMode, ReadOptions, RemoveOptions,
};

fn tmpdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ziplark-v050-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn grep_searches_across_zip_7z_tar_and_skips_binaries() {
    let dir = tmpdir("grep-all");
    let src = dir.join("proj");
    std::fs::create_dir_all(src.join("src")).unwrap();
    std::fs::write(
        src.join("src/lib.rs"),
        b"pub fn needle_func() -> u32 {\n    42 // NEEDLE_TOKEN here\n}\n",
    )
    .unwrap();
    std::fs::write(
        src.join("README.md"),
        b"# Project\nNothing to see\nneedle_token in lowercase\n",
    )
    .unwrap();
    // Binary file containing NUL bytes and the search string — must be skipped!
    std::fs::write(
        src.join("blob.dat"),
        b"\x00\x01\x02NEEDLE_TOKEN in binary\x00",
    )
    .unwrap();

    for (ext, fmt) in [
        ("zip", Format::Zip),
        ("7z", Format::SevenZ),
        ("tar.zst", Format::TarZst),
    ] {
        let arc = dir.join(format!("proj.{ext}"));
        create(
            &arc,
            std::slice::from_ref(&src),
            &CreateOptions {
                format: fmt,
                level: Level::Fast,
                password: None,
            },
            None,
        )
        .unwrap();

        // Case-sensitive search for "NEEDLE_TOKEN" finds only src/lib.rs line 2
        let rep = grep(
            &arc,
            "NEEDLE_TOKEN",
            &GrepOptions::default(),
            None,
        )
        .unwrap();
        assert_eq!(rep.files_searched, 2, "format={ext}");
        assert_eq!(rep.files_matched, 1, "format={ext}");
        assert_eq!(rep.matches.len(), 1, "format={ext}");
        assert!(rep.matches[0].path.ends_with("src/lib.rs"));
        assert_eq!(rep.matches[0].line_number, 2);
        assert!(rep.matches[0].line.contains("NEEDLE_TOKEN"));

        // Case-insensitive search finds both src/lib.rs (2 lines) and README.md (1 line)
        let rep_ci = grep(
            &arc,
            "needle_",
            &GrepOptions {
                case_insensitive: true,
                ..GrepOptions::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(rep_ci.files_matched, 2, "format={ext}");
        assert_eq!(rep_ci.matches.len(), 3, "format={ext}");

        // Include filter narrows to *.md
        let rep_md = grep(
            &arc,
            "needle_",
            &GrepOptions {
                case_insensitive: true,
                include: vec!["*.md".to_string()],
                ..GrepOptions::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(rep_md.files_matched, 1, "format={ext}");
        assert_eq!(rep_md.matches.len(), 1, "format={ext}");
        assert!(rep_md.matches[0].path.ends_with("README.md"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn grep_searches_inside_rar_and_single_stream_archives() {
    // RAR multi-volume fixture
    let rar = fixture("multi.part1.rar");
    let rep = grep(
        &rar,
        "Ziplark",
        &GrepOptions {
            case_insensitive: true,
            ..GrepOptions::default()
        },
        None,
    )
    .unwrap();
    assert!(rep.files_searched >= 1);

    // Single-stream .zst
    let dir = tmpdir("grep-stream");
    let log = dir.join("server.log");
    std::fs::write(&log, b"INFO boot\nERROR database timeout\nINFO shutdown\n").unwrap();
    let zst = dir.join("server.log.zst");
    create(
        &zst,
        std::slice::from_ref(&log),
        &CreateOptions::new(Format::Zst),
        None,
    )
    .unwrap();

    let srep = grep(&zst, "ERROR", &GrepOptions::default(), None).unwrap();
    assert_eq!(srep.matches.len(), 1);
    assert_eq!(srep.matches[0].line_number, 2);
    assert_eq!(srep.matches[0].line, "ERROR database timeout");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn remove_deletes_entries_from_zip_7z_and_tar_gz() {
    let dir = tmpdir("remove-all");
    let root = dir.join("pkg");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("keep.txt"), b"staying alive").unwrap();
    std::fs::write(root.join("drop.txt"), b"goodbye").unwrap();
    std::fs::write(root.join("sub/nested.txt"), b"inside sub").unwrap();

    for (ext, fmt) in [
        ("zip", Format::Zip),
        ("7z", Format::SevenZ),
        ("tar.gz", Format::TarGz),
    ] {
        let arc = dir.join(format!("pkg.{ext}"));
        create(
            &arc,
            std::slice::from_ref(&root),
            &CreateOptions {
                format: fmt,
                level: Level::Fast,
                password: None,
            },
            None,
        )
        .unwrap();

        let rm_rep = remove(
            &arc,
            &RemoveOptions {
                password: None,
                include: vec!["pkg/drop.txt".to_string(), "pkg/sub".to_string()],
                match_mode: MatchMode::Exact,
                level: Level::Fast,
            },
            None,
        )
        .unwrap();
        assert!(rm_rep.entries_removed >= 2, "format={ext}");

        let info = list(&arc, &ListOptions::default()).unwrap();
        let paths: Vec<&str> = info.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(
            paths.iter().any(|p| p.ends_with("keep.txt")),
            "keep.txt missing in {ext}: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains("drop.txt") || p.contains("nested.txt")),
            "removed files still present in {ext}: {paths:?}"
        );

        let content = read_entry(&arc, "pkg/keep.txt", &ReadOptions::default()).unwrap();
        assert_eq!(content.bytes, b"staying alive");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn convert_transforms_rar_to_tar_zst_and_zip_to_7z() {
    let dir = tmpdir("convert");
    let rar = fixture("multi.part1.rar");
    let out_tzst = dir.join("converted.tar.zst");

    let rep = convert(
        &rar,
        &out_tzst,
        &ConvertOptions {
            read_password: None,
            create: CreateOptions {
                format: Format::TarZst,
                level: Level::Fast,
                password: None,
            },
        },
        None,
    )
    .unwrap();
    assert_eq!(rep.source_format, Format::Rar);
    assert_eq!(rep.output_format, Format::TarZst);
    assert!(rep.entries_converted >= 3);

    // Convert the .tar.zst further into a .7z and verify contents match original RAR
    let out_7z = dir.join("converted.7z");
    let rep2 = convert(
        &out_tzst,
        &out_7z,
        &ConvertOptions {
            read_password: None,
            create: CreateOptions {
                format: Format::SevenZ,
                level: Level::Fast,
                password: None,
            },
        },
        None,
    )
    .unwrap();
    assert_eq!(rep2.source_format, Format::TarZst);
    assert_eq!(rep2.output_format, Format::SevenZ);

    let orig_readme = read_entry(&rar, "tree/docs/readme.txt", &ReadOptions::default()).unwrap();
    let conv_readme = read_entry(&out_7z, "tree/docs/readme.txt", &ReadOptions::default()).unwrap();
    assert_eq!(orig_readme.bytes, conv_readme.bytes);

    let extract_dir = dir.join("out");
    let ex = extract(&out_7z, &ExtractOptions::new(&extract_dir), None).unwrap();
    assert!(ex.files_written >= 3);
    let _ = std::fs::remove_dir_all(&dir);
}
