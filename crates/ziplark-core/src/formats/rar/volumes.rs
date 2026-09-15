//! Which files make up one RAR archive.
//!
//! RAR splits a large archive into volumes, and has used two naming schemes for
//! them over the years:
//!
//! * **Numbered** — `movie.part01.rar`, `movie.part02.rar`, … The number is in
//!   the name and the first volume is number 1.
//! * **Legacy** — `movie.rar`, then `movie.r00`, `movie.r01` … `movie.r99`,
//!   `movie.s00`, … Here the first volume is the one with *no* number, which is
//!   what makes "the first part is part one" arithmetic quietly wrong: it picks
//!   `movie.r01`, the third file, and extraction then fails or silently loses
//!   whatever spanned the earlier volumes.
//!
//! Every file of a set is a normal thing to double-click, so all of this exists
//! to answer one question: given any one of them, which file do we open?

use std::path::{Path, PathBuf};

/// How a file's name says it belongs to a volume set.
#[derive(Debug, PartialEq, Eq)]
enum Scheme {
    /// `<stem>.part<N>.rar`, with `N` zero-padded to `width`.
    Numbered { stem: String, width: usize },
    /// `<stem>.rar` followed by `<stem>.r00`, `<stem>.r01`, …
    Legacy { stem: String },
    /// One file, all by itself.
    Single,
}

/// The name of one volume of a set, without its directory.
fn volume_name(scheme: &Scheme, n: usize) -> Option<String> {
    match scheme {
        Scheme::Single => (n == 0).then(|| String::new()),
        Scheme::Numbered { stem, width } => Some(format!(
            "{stem}.part{:0width$}.rar",
            n + 1,
            width = *width
        )),
        Scheme::Legacy { stem } => Some(match n {
            0 => format!("{stem}.rar"),
            _ => {
                // `.r99` is followed by `.s00`, not `.r100`.
                let k = n - 1;
                let letter = (b'r' + u8::try_from(k / 100).ok()?) as char;
                if letter > 'z' {
                    return None;
                }
                format!("{stem}.{letter}{:02}", k % 100)
            }
        }),
    }
}

/// Work out the scheme from a file name, consulting the filesystem only to
/// distinguish a lone `x.rar` from the first volume of a legacy set.
fn scheme_of(path: &Path) -> Scheme {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Scheme::Single;
    };
    let lower = name.to_ascii_lowercase();

    // `<stem>.part<N>.rar`
    if let Some(head) = lower.strip_suffix(".rar") {
        if let Some(cut) = head.rfind(".part") {
            let digits = &head[cut + 5..];
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
                return Scheme::Numbered {
                    stem: name[..cut].to_string(),
                    width: digits.len(),
                };
            }
        }
        // A plain `.rar` is the first volume of a legacy set if `.r00` is there.
        let stem = name[..name.len() - 4].to_string();
        let legacy = Scheme::Legacy { stem };
        if volume_name(&legacy, 1)
            .map(|n| path.with_file_name(n).exists())
            .unwrap_or(false)
        {
            return legacy;
        }
        return Scheme::Single;
    }

    // `<stem>.r00` … `<stem>.z99`: a continuation volume of a legacy set.
    if let Some(dot) = lower.rfind('.') {
        let ext = &lower[dot + 1..];
        let mut chars = ext.chars();
        if ext.len() == 3
            && matches!(chars.next(), Some(c) if ('r'..='z').contains(&c))
            && ext[1..].chars().all(|c| c.is_ascii_digit())
        {
            return Scheme::Legacy {
                stem: name[..dot].to_string(),
            };
        }
    }

    Scheme::Single
}

/// The file to actually open, given any one file of an archive.
///
/// Extraction has to start at the first volume: an entry that spans a boundary
/// begins in the earlier volume, and libunrar only ever walks forward. If the
/// first volume is not on disk, the file we were handed is returned unchanged
/// so the error comes from trying to read it, not from a name we invented.
pub fn first_volume(path: &Path) -> PathBuf {
    match volume_name(&scheme_of(path), 0) {
        Some(name) if !name.is_empty() => {
            let first = path.with_file_name(name);
            if first.exists() {
                first
            } else {
                path.to_path_buf()
            }
        }
        _ => path.to_path_buf(),
    }
}

/// The volumes of this set that are on disk, in order, starting at `first`.
/// Empty when the name says this is not a multi-volume archive at all.
pub fn present(first: &Path) -> Vec<PathBuf> {
    let scheme = scheme_of(first);
    if scheme == Scheme::Single {
        return Vec::new();
    }
    let mut found = Vec::new();
    for n in 0.. {
        match volume_name(&scheme, n).map(|name| first.with_file_name(name)) {
            Some(p) if p.exists() => found.push(p),
            _ => break,
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(scheme: &Scheme, n: usize) -> String {
        volume_name(scheme, n).unwrap()
    }

    #[test]
    fn numbered_volumes_keep_their_padding() {
        let two = Scheme::Numbered {
            stem: "movie".into(),
            width: 2,
        };
        assert_eq!(name(&two, 0), "movie.part01.rar");
        assert_eq!(name(&two, 41), "movie.part42.rar");
        let one = Scheme::Numbered {
            stem: "movie".into(),
            width: 1,
        };
        assert_eq!(name(&one, 0), "movie.part1.rar");
    }

    #[test]
    fn legacy_volumes_run_rar_then_r00() {
        let s = Scheme::Legacy {
            stem: "movie".into(),
        };
        assert_eq!(name(&s, 0), "movie.rar");
        assert_eq!(name(&s, 1), "movie.r00");
        assert_eq!(name(&s, 2), "movie.r01");
        // r99 is followed by s00.
        assert_eq!(name(&s, 100), "movie.r99");
        assert_eq!(name(&s, 101), "movie.s00");
    }

    #[test]
    fn schemes_are_read_off_the_name() {
        assert_eq!(
            scheme_of(Path::new("/x/movie.part03.rar")),
            Scheme::Numbered {
                stem: "movie".into(),
                width: 2
            }
        );
        assert_eq!(
            scheme_of(Path::new("/x/movie.PART3.RAR")),
            Scheme::Numbered {
                stem: "movie".into(),
                width: 1
            },
            "the name may be upper case; the stem keeps its own spelling"
        );
        assert_eq!(
            scheme_of(Path::new("/x/movie.r07")),
            Scheme::Legacy {
                stem: "movie".into()
            }
        );
        assert_eq!(scheme_of(Path::new("/x/movie.s00")), Scheme::Legacy { stem: "movie".into() });
        // No sibling `.r00` on disk, so a lone `.rar` is a single archive.
        assert_eq!(scheme_of(Path::new("/x/nonexistent-movie.rar")), Scheme::Single);
        assert_eq!(scheme_of(Path::new("/x/movie.partial.rar")), Scheme::Single);
        assert_eq!(scheme_of(Path::new("/x/movie.zip")), Scheme::Single);
    }

    #[test]
    fn a_single_archive_resolves_to_itself() {
        let p = Path::new("/x/nonexistent-movie.rar");
        assert_eq!(first_volume(p), p);
        assert!(present(p).is_empty());
    }
}
