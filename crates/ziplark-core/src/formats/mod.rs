pub mod rar;
pub mod sevenz;
pub mod stream;
pub mod tar;
pub mod zip;

use crate::error::{Error, Result};
use std::path::{Component, Path, PathBuf};

/// Join an archive-internal entry path onto `dest`, refusing anything that
/// would escape `dest` (absolute paths, `..`, drive prefixes). This is the
/// "zip slip" guard and every format's extract path MUST funnel through it.
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
