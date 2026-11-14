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
    if head.starts_with(b"PK\x03\x04")
        || head.starts_with(b"PK\x05\x06")
        || head.starts_with(b"PK\x07\x08")
        || head.starts_with(b"Cr24")
    {
        return Some(Format::Zip);
    }
    if head.starts_with(b"7z\xBC\xAF\x27\x1C") {
        return Some(Format::SevenZ);
    }
    if head.starts_with(b"Rar!\x1A\x07\x00") || head.starts_with(b"Rar!\x1A\x07\x01\x00") {
        return Some(Format::Rar);
    }
    if head.starts_with(b"MSCF\x00\x00\x00\x00") {
        return Some(Format::Cab);
    }
    if head.starts_with(b"\xED\xAB\xEE\xDB") {
        return Some(Format::Rpm);
    }
    if head.starts_with(b"070701") || head.starts_with(b"070702") || head.starts_with(b"070707") {
        return Some(Format::Cpio);
    }

    // Compressed streams that may wrap a tar or cpio. Decide by extension
    // (stripping any `.001`..`.9999` volume suffix first so `a.tar.zst.001`
    // is recognised as `TarZst` rather than plain `Zst`).
    let raw_lname = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let lname = strip_split_suffix(&raw_lname);

    if head.starts_with(b"xar!") {
        return Some(if lname.ends_with(".pkg") || lname.ends_with(".mpkg") {
            Format::Pkg
        } else {
            Format::Xar
        });
    }

    if head.starts_with(b"!<arch>\n") {
        let is_deb_ext = lname.ends_with(".deb") || lname.ends_with(".udeb") || lname.ends_with(".ipk");
        let is_deb_member = head.len() >= 24 && head[8..24].starts_with(b"debian-binary");
        return Some(if is_deb_ext || is_deb_member {
            Format::Deb
        } else {
            Format::Ar
        });
    }

    if head.starts_with(b"\x1F\x8B") {
        if lname.ends_with(".cpio.gz") || lname.ends_with(".cpgz") {
            return Some(Format::Cpio);
        }
        return Some(if is_tar_name(&lname, "gz") { Format::TarGz } else { Format::Gz });
    }
    if head.starts_with(b"BZh") {
        if lname.ends_with(".cpio.bz2") {
            return Some(Format::Cpio);
        }
        return Some(if is_tar_name(&lname, "bz2") { Format::TarBz2 } else { Format::Bz2 });
    }
    if head.starts_with(b"\xFD7zXZ\x00") {
        if lname.ends_with(".cpio.xz") {
            return Some(Format::Cpio);
        }
        return Some(if is_tar_name(&lname, "xz") { Format::TarXz } else { Format::Xz });
    }
    if head.starts_with(b"\x28\xB5\x2F\xFD") {
        if lname.ends_with(".cpio.zst") {
            return Some(Format::Cpio);
        }
        return Some(if is_tar_name(&lname, "zst") { Format::TarZst } else { Format::Zst });
    }
    if head.starts_with(b"\x04\x22\x4D\x18") {
        if lname.ends_with(".cpio.lz4") {
            return Some(Format::Cpio);
        }
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

    // Then the name: `.tar.gz` against `.gz`, volume names like `.7z.002` / `.r00` / `.z01`,
    // and package extensions.
    if let Some(fmt) = detect_by_extension_or_set(path, &raw_lname) {
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

fn detect_by_extension_or_set(path: &Path, lname: &str) -> Option<Format> {
    // `<stem>.z01` when `<stem>.zip` exists alongside it is a split ZIP, whereas
    // `<stem>.r00` .. `<stem>.z99` with `<stem>.rar` (or standalone) is legacy RAR.
    if let Some((stem, ext)) = lname.rsplit_once('.') {
        if ext.len() == 3
            && ext.starts_with('z')
            && ext[1..].chars().all(|c| c.is_ascii_digit())
            && path.with_file_name(format!("{stem}.zip")).exists()
        {
            return Some(Format::Zip);
        }
    }
    detect_by_extension(lname)
}

fn strip_split_suffix(lname: &str) -> &str {
    match lname.rsplit_once('.') {
        Some((base, ext))
            if (2..=4).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_digit())
                && base.contains('.') =>
        {
            base
        }
        _ => lname,
    }
}

pub fn detect_by_extension(lname: &str) -> Option<Format> {
    // Strip a trailing numbered split suffix (`.001` .. `.9999`) if it sits
    // behind a known archive extension (`movie.7z.002` -> `movie.7z`).
    let lname = strip_split_suffix(lname);

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
        (".cpio.gz", Format::Cpio),
        (".cpgz", Format::Cpio),
        (".cpio.bz2", Format::Cpio),
        (".cpio.xz", Format::Cpio),
        (".cpio.zst", Format::Cpio),
        (".cpio.lz4", Format::Cpio),
        (".tar", Format::Tar),
        (".cbt", Format::Tar),
        (".zip", Format::Zip),
        (".cbz", Format::Zip),
        (".jar", Format::Zip),
        (".war", Format::Zip),
        (".ear", Format::Zip),
        (".apk", Format::Zip),
        (".aab", Format::Zip),
        (".ipa", Format::Zip),
        (".xapk", Format::Zip),
        (".whl", Format::Zip),
        (".egg", Format::Zip),
        (".nupkg", Format::Zip),
        (".crx", Format::Zip),
        (".vsix", Format::Zip),
        (".xpi", Format::Zip),
        (".epub", Format::Zip),
        (".docx", Format::Zip),
        (".xlsx", Format::Zip),
        (".pptx", Format::Zip),
        (".odt", Format::Zip),
        (".ods", Format::Zip),
        (".odp", Format::Zip),
        (".7z", Format::SevenZ),
        (".cb7", Format::SevenZ),
        (".rar", Format::Rar),
        (".cbr", Format::Rar),
        (".gz", Format::Gz),
        (".bz2", Format::Bz2),
        (".xz", Format::Xz),
        (".zst", Format::Zst),
        (".lz4", Format::Lz4),
        (".iso", Format::Iso),
        (".deb", Format::Deb),
        (".udeb", Format::Deb),
        (".ipk", Format::Deb),
        (".ar", Format::Ar),
        (".a", Format::Ar),
        (".lib", Format::Ar),
        (".cpio", Format::Cpio),
        (".rpm", Format::Rpm),
        (".srpm", Format::Rpm),
        (".cab", Format::Cab),
        (".pkg", Format::Pkg),
        (".mpkg", Format::Pkg),
        (".xar", Format::Xar),
        (".xip", Format::Xar),
        (".safariextz", Format::Xar),
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
