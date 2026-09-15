use crate::model::Format;
use std::io::Read;
use std::path::Path;

/// Detect an archive's format. Magic bytes are authoritative; we fall back to
/// the file extension only when the header is ambiguous (e.g. distinguishing a
/// plain `.gz` from a `.tar.gz`, which share the gzip magic).
pub fn detect(path: &Path) -> Option<Format> {
    let mut magic = [0u8; 512];
    let n = read_magic(path, &mut magic).unwrap_or(0);
    let head = &magic[..n];

    // Container magics that are unambiguous.
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Some(Format::Zip);
    }
    if head.starts_with(b"7z\xBC\xAF\x27\x1C") {
        return Some(Format::SevenZ);
    }
    if head.starts_with(b"Rar!\x1A\x07\x00") || head.starts_with(b"Rar!\x1A\x07\x01\x00") {
        return Some(Format::Rar);
    }

    // Compressed streams that may wrap a tar. Decide tar-vs-plain by extension.
    let lname = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if head.starts_with(b"\x1F\x8B") {
        return Some(if is_tar_name(&lname, "gz") { Format::TarGz } else { Format::Gz });
    }
    if head.starts_with(b"BZh") {
        return Some(if is_tar_name(&lname, "bz2") { Format::TarBz2 } else { Format::Bz2 });
    }
    if head.starts_with(b"\xFD7zXZ\x00") {
        return Some(if is_tar_name(&lname, "xz") { Format::TarXz } else { Format::Xz });
    }
    if head.starts_with(b"\x28\xB5\x2F\xFD") {
        return Some(if is_tar_name(&lname, "zst") { Format::TarZst } else { Format::Zst });
    }
    if head.starts_with(b"\x04\x22\x4D\x18") {
        return Some(if is_tar_name(&lname, "lz4") { Format::TarLz4 } else { Format::Lz4 });
    }

    // Uncompressed tar: "ustar" magic lives at offset 257.
    if head.len() > 262 && &head[257..262] == b"ustar" {
        return Some(Format::Tar);
    }

    // ISO 9660: the "CD001" volume-descriptor magic lives at offset 0x8001,
    // past the 512-byte header, so check it with a dedicated seek.
    if is_iso(path) {
        return Some(Format::Iso);
    }

    // Then the name: `.tar.gz` against `.gz`, and volume names like `.r00`.
    if let Some(fmt) = detect_by_extension(&lname) {
        return Some(fmt);
    }

    // Last: a self-extracting archive, which is an executable with the archive
    // appended — so its magic is a long way in. libunrar reads a RAR payload at
    // any offset, so finding the signature is enough to open one, which is how
    // a `movie.exe` from 2008 opens here.
    if is_sfx_rar(path) {
        return Some(Format::Rar);
    }
    None
}

/// Scan for a RAR signature past the start of the file.
///
/// WinRAR's own limit for how far into a file an SFX payload may begin is 1 MB,
/// so that is where the search stops; beyond it, a file that merely mentions
/// "Rar!" somewhere would start being mistaken for an archive.
fn is_sfx_rar(path: &Path) -> bool {
    const LIMIT: usize = 1024 * 1024;
    const CHUNK: usize = 64 * 1024;
    const SIGNATURE: &[u8] = b"Rar!\x1A\x07";

    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = vec![0u8; CHUNK + SIGNATURE.len() - 1];
    let mut carry = 0usize; // bytes kept from the previous chunk
    let mut scanned = 0usize;

    while scanned < LIMIT {
        let Ok(n) = f.read(&mut buf[carry..]) else {
            return false;
        };
        if n == 0 {
            return false;
        }
        let filled = carry + n;
        if buf[..filled].windows(SIGNATURE.len()).any(|w| w == SIGNATURE) {
            return true;
        }
        // A signature could straddle the boundary, so keep the tail.
        carry = SIGNATURE.len() - 1;
        let keep_from = filled - carry;
        buf.copy_within(keep_from..filled, 0);
        scanned += n;
    }
    false
}

/// ISO 9660 images carry "CD001" at byte offset 32769 (sector 16, +1).
fn is_iso(path: &Path) -> bool {
    use std::io::{Seek, SeekFrom};
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    if f.seek(SeekFrom::Start(32769)).is_err() {
        return false;
    }
    let mut sig = [0u8; 5];
    f.read_exact(&mut sig).is_ok() && &sig == b"CD001"
}

fn is_tar_name(lname: &str, comp_ext: &str) -> bool {
    lname.ends_with(&format!(".tar.{comp_ext}"))
        || (comp_ext == "gz" && lname.ends_with(".tgz"))
        || (comp_ext == "bz2" && lname.ends_with(".tbz2"))
        || (comp_ext == "xz" && lname.ends_with(".txz"))
        || (comp_ext == "zst" && lname.ends_with(".tzst"))
        || (comp_ext == "lz4" && lname.ends_with(".tlz4"))
}

fn detect_by_extension(lname: &str) -> Option<Format> {
    // A RAR volume can be named `.r00` … `.z99`, which says "RAR" as clearly as
    // `.rar` does and is how every old download is still named.
    if let Some((_, ext)) = lname.rsplit_once('.') {
        let mut chars = ext.chars();
        if ext.len() == 3
            && matches!(chars.next(), Some(c) if ('r'..='z').contains(&c))
            && ext[1..].chars().all(|c| c.is_ascii_digit())
        {
            return Some(Format::Rar);
        }
    }

    let table = [
        (".tar.gz", Format::TarGz),
        (".tgz", Format::TarGz),
        (".tar.bz2", Format::TarBz2),
        (".tbz2", Format::TarBz2),
        (".tar.xz", Format::TarXz),
        (".txz", Format::TarXz),
        (".tar.zst", Format::TarZst),
        (".tzst", Format::TarZst),
        (".tar.lz4", Format::TarLz4),
        (".tlz4", Format::TarLz4),
        (".tar", Format::Tar),
        (".zip", Format::Zip),
        (".7z", Format::SevenZ),
        (".rar", Format::Rar),
        (".gz", Format::Gz),
        (".bz2", Format::Bz2),
        (".xz", Format::Xz),
        (".zst", Format::Zst),
        (".lz4", Format::Lz4),
        (".iso", Format::Iso),
    ];
    table
        .iter()
        .find(|(ext, _)| lname.ends_with(ext))
        .map(|(_, f)| *f)
}

fn read_magic(path: &Path, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut f = std::fs::File::open(path)?;
    let mut filled = 0;
    while filled < buf.len() {
        let n = f.read(&mut buf[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn rar_volume_names_are_recognised_by_extension() {
        assert_eq!(detect_by_extension("movie.r00"), Some(Format::Rar));
        assert_eq!(detect_by_extension("movie.s07"), Some(Format::Rar));
        assert_eq!(detect_by_extension("movie.part02.rar"), Some(Format::Rar));
        // Not a volume: three characters, but not letter-digit-digit.
        assert_eq!(detect_by_extension("notes.rtf"), None);
        assert_eq!(detect_by_extension("photo.raw"), None);
    }

    #[test]
    fn a_self_extracting_archive_is_still_an_archive() {
        // `sfx.exe` is a stub followed by a RAR, the shape every SFX has.
        assert_eq!(detect(&fixture("sfx.exe")), Some(Format::Rar));
        assert!(is_sfx_rar(&fixture("cjk.rar")), "signature at offset 0 counts too");
    }

    #[test]
    fn a_file_that_is_not_an_archive_is_not_detected() {
        let scratch = std::env::temp_dir().join(format!("ziplark-detect-{}", std::process::id()));
        std::fs::write(&scratch, b"just some text, mentioning RAR but not Rar!\x1a").unwrap();
        assert_eq!(detect(&scratch), None);
        let _ = std::fs::remove_file(&scratch);
    }
}
