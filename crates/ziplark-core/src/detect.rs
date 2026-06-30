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

    // Last resort: trust the extension.
    detect_by_extension(&lname)
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
