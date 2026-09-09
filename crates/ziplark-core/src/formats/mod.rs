pub mod iso;
pub mod rar;
pub mod sevenz;
pub mod stream;
pub mod tar;
pub mod zip;

use crate::error::{Error, Result};
use std::collections::HashSet;
use std::fs::File;
use std::path::{Component, Path, PathBuf};

/// Guards every write an extraction makes against escaping the destination.
///
/// There are two separate ways out of `dest`, and a guard is only worth
/// anything if it stops both:
///
/// 1. **By name** — `../../etc/passwd`, `/etc/passwd`, `C:\...`. Handled by
///    [`safe_join`], which is a pure function of the entry name.
/// 2. **Through a symlink** — an archive stores `evil -> /tmp` and then an
///    entry named `evil/owned.txt`. That second name contains no `..` at all,
///    so a name-only check waves it through, and the write lands in `/tmp`.
///    Only looking at what is actually on disk catches this.
///
/// So `join` does both: it validates the name, then walks the directories the
/// write would descend through and refuses if any of them is a symlink. Results
/// are cached, since archives share ancestors between consecutive entries.
///
/// The symlink may have been planted by an earlier entry of the same archive,
/// or have been sitting in `dest` beforehand — the guard does not care which.
pub struct DestGuard {
    dest: PathBuf,
    /// Directories already confirmed to be real directories, not symlinks.
    verified: HashSet<PathBuf>,
}

impl DestGuard {
    pub fn new(dest: impl Into<PathBuf>) -> Self {
        Self {
            dest: dest.into(),
            verified: HashSet::new(),
        }
    }

    /// Validate `entry_path` and return the path to write to.
    pub fn join(&mut self, entry_path: &str) -> Result<PathBuf> {
        let out = safe_join(&self.dest, entry_path)?;
        self.verify_ancestors(&out, entry_path)?;
        Ok(out)
    }

    /// Drop any cached judgement about `path` — call after replacing it with
    /// something that is no longer a plain directory (a symlink, say).
    pub fn forget(&mut self, path: &Path) {
        self.verified.remove(path);
    }

    /// Refuse if any directory between `dest` and the entry is a symlink.
    fn verify_ancestors(&mut self, out: &Path, entry_path: &str) -> Result<()> {
        let Ok(rel) = out.strip_prefix(&self.dest) else {
            return Err(Error::PathTraversal(entry_path.to_string()));
        };
        let comps: Vec<Component> = rel.components().collect();
        // The last component is the entry itself; only what we descend
        // *through* has to be a real directory.
        let depth = comps.len().saturating_sub(1);

        let mut cur = self.dest.clone();
        for comp in comps.into_iter().take(depth) {
            cur.push(comp);
            if self.verified.contains(&cur) {
                continue;
            }
            match std::fs::symlink_metadata(&cur) {
                Ok(md) if md.file_type().is_symlink() => {
                    return Err(Error::PathTraversal(entry_path.to_string()));
                }
                // A real directory (or file — the write will fail on its own).
                Ok(_) => {
                    self.verified.insert(cur.clone());
                }
                // Doesn't exist yet; it will be created as a real directory.
                Err(_) => {}
            }
        }
        Ok(())
    }
}

/// Prepare `path` to be written as a fresh file, honouring `overwrite`.
///
/// Uses `symlink_metadata`, not `exists()`: a symlink at the leaf — including a
/// *dangling* one, which `exists()` reports as absent — would otherwise
/// redirect the write to wherever it points. The link itself is removed rather
/// than followed.
pub fn prepare_leaf(path: &Path, overwrite: bool) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(md) => {
            if !overwrite {
                return Err(Error::other(format!(
                    "{} already exists (use overwrite)",
                    path.display()
                )));
            }
            if md.file_type().is_symlink() {
                std::fs::remove_file(path)?;
            }
            Ok(())
        }
        Err(_) => Ok(()),
    }
}

/// `prepare_leaf` + `File::create`, which is what every format's file-writing
/// path wants.
pub fn create_file(path: &Path, overwrite: bool) -> Result<File> {
    prepare_leaf(path, overwrite)?;
    Ok(File::create(path)?)
}

/// Join an archive-internal entry path onto `dest`, refusing anything that
/// would escape `dest` (absolute paths, `..`, drive prefixes). This is the
/// name-level half of the guard; extraction goes through [`DestGuard`], which
/// adds the on-disk half.
pub fn safe_join(dest: &Path, entry_path: &str) -> Result<PathBuf> {
    // Normalise separators; archives may use either.
    let normalized = entry_path.replace('\\', "/");
    let mut out = dest.to_path_buf();

    for comp in Path::new(&normalized).components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            // Reject anything that could climb out of `dest`.
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::PathTraversal(entry_path.to_string()));
            }
        }
    }

    // Defense in depth: the resolved path must still be under `dest`.
    if !out.starts_with(dest) {
        return Err(Error::PathTraversal(entry_path.to_string()));
    }
    Ok(out)
}

/// Recursively collect (absolute_path, archive_relative_path) pairs for a set
/// of input files/dirs, used by every create() implementation. A directory
/// `foo` becomes entries `foo/...`; a file `bar.txt` becomes `bar.txt`.
pub fn collect_inputs(inputs: &[PathBuf]) -> Result<Vec<(PathBuf, String)>> {
    let mut out = Vec::new();
    for input in inputs {
        let input = input.as_path();
        if !input.exists() {
            return Err(Error::other(format!("input does not exist: {}", input.display())));
        }
        let base_name = input
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| Error::other(format!("invalid input name: {}", input.display())))?
            .to_string();

        if input.is_dir() {
            walk_dir(input, &base_name, &mut out)?;
        } else {
            out.push((input.to_path_buf(), base_name));
        }
    }
    Ok(out)
}

fn walk_dir(dir: &Path, prefix: &str, out: &mut Vec<(PathBuf, String)>) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::result::Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    if entries.is_empty() {
        // Preserve empty directories with a trailing slash marker.
        out.push((dir.to_path_buf(), format!("{prefix}/")));
        return Ok(());
    }
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let child_rel = format!("{prefix}/{name}");
        let child = entry.path();
        if child.is_dir() {
            walk_dir(&child, &child_rel, out)?;
        } else {
            out.push((child, child_rel));
        }
    }
    Ok(())
}

/// Create parent directories for a file path about to be written.
pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
