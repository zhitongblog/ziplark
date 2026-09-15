//! Choosing *which* entries an operation touches.
//!
//! Every format used to carry its own copy of `include.iter().any(|p|
//! name.contains(p))`, which meant "extract `docs/a.txt`" also extracted
//! `other/docs/a.txt.bak`, and there was no way to ask for one exact entry —
//! the thing a GUI needs when the user ticks three rows out of a thousand.
//!
//! So selection lives here, once, with the mode spelled out by the caller.

/// How a pattern is compared against an entry's path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchMode {
    /// A pattern containing `*` or `?` is a glob over the whole path;
    /// anything else is a substring match. This is the forgiving mode a
    /// human types into a CLI.
    #[default]
    Auto,
    /// The pattern is a complete entry path — `docs/a.txt` matches that entry
    /// and nothing else. A pattern naming a directory (`docs`) also takes
    /// everything under it, since that is what selecting a folder means.
    Exact,
}

/// Decides whether an entry is included in an operation.
#[derive(Debug, Clone)]
pub struct Selector {
    patterns: Vec<String>,
    mode: MatchMode,
}

impl Selector {
    pub fn new(patterns: &[String], mode: MatchMode) -> Self {
        Self {
            patterns: patterns.iter().map(|p| normalize(p)).collect(),
            mode,
        }
    }

    /// No patterns means "everything", which is the common case.
    pub fn matches_everything(&self) -> bool {
        self.patterns.is_empty()
    }

    pub fn matches(&self, entry_path: &str) -> bool {
        if self.patterns.is_empty() {
            return true;
        }
        let path = normalize(entry_path);
        self.patterns.iter().any(|p| match self.mode {
            MatchMode::Auto => auto_match(p, &path),
            MatchMode::Exact => exact_match(p, &path),
        })
    }
}

/// Entry paths and patterns are compared in one spelling: forward slashes, no
/// `./` prefix, no trailing slash.
fn normalize(s: &str) -> String {
    let s = s.replace('\\', "/");
    let s = s.strip_prefix("./").unwrap_or(&s);
    s.trim_end_matches('/').to_string()
}

fn auto_match(pattern: &str, path: &str) -> bool {
    if pattern.contains('*') || pattern.contains('?') {
        glob_match(pattern, path)
    } else {
        path.contains(pattern)
    }
}

fn exact_match(pattern: &str, path: &str) -> bool {
    path == pattern || path.starts_with(&format!("{pattern}/"))
}

/// Glob match supporting `*` (any run of characters, `/` included) and `?`
/// (exactly one). Iterative backtracking — no regex, no extra dependency.
///
/// `*` deliberately crosses `/`, so `*.txt` finds a text file at any depth.
/// That is what someone typing a pattern into an archiver means by it.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    // Where to resume from if the current `*` guess turns out to be wrong.
    let (mut star, mut resume) = (usize::MAX, 0usize);

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            resume = ti;
            pi += 1;
        } else if star != usize::MAX {
            // Backtrack: let the last `*` swallow one more character.
            pi = star + 1;
            resume += 1;
            ti = resume;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(pats: &[&str], mode: MatchMode) -> Selector {
        Selector::new(
            &pats.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            mode,
        )
    }

    #[test]
    fn empty_selector_takes_everything() {
        let s = sel(&[], MatchMode::Exact);
        assert!(s.matches_everything());
        assert!(s.matches("anything/at/all.txt"));
    }

    #[test]
    fn auto_mode_is_substring_until_a_wildcard_appears() {
        let s = sel(&["docs"], MatchMode::Auto);
        assert!(s.matches("docs/a.txt"));
        assert!(s.matches("other/docs/a.txt"));
        assert!(!s.matches("readme.md"));

        let g = sel(&["docs/*.txt"], MatchMode::Auto);
        assert!(g.matches("docs/a.txt"));
        assert!(!g.matches("other/docs/a.txt"));
    }

    #[test]
    fn exact_mode_does_not_over_match() {
        let s = sel(&["docs/a.txt"], MatchMode::Exact);
        assert!(s.matches("docs/a.txt"));
        // The old substring behaviour matched all three of these.
        assert!(!s.matches("docs/a.txt.bak"));
        assert!(!s.matches("backup/docs/a.txt"));
        assert!(!s.matches("docs/a.tx"));
    }

    #[test]
    fn exact_mode_takes_a_selected_directory_whole() {
        let s = sel(&["docs"], MatchMode::Exact);
        assert!(s.matches("docs"));
        assert!(s.matches("docs/a.txt"));
        assert!(s.matches("docs/deep/b.txt"));
        assert!(!s.matches("docsx/a.txt"));
    }

    #[test]
    fn separators_and_trailing_slashes_do_not_matter() {
        let s = sel(&["docs/"], MatchMode::Exact);
        assert!(s.matches("docs/a.txt"));
        // Archives written on Windows can store backslashes.
        assert!(s.matches("docs\\a.txt"));
        assert!(sel(&["./docs/a.txt"], MatchMode::Exact).matches("docs/a.txt"));
    }

    #[test]
    fn globs() {
        assert!(glob_match("*.txt", "a/b/c.txt"));
        assert!(glob_match("src/*/f0001?.txt", "src/d001/f00010.txt"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("a*b*c", "axxbyyc"));
        assert!(!glob_match("*.txt", "a/b/c.md"));
        assert!(!glob_match("src/?.txt", "src/ab.txt"));
        // A trailing run of stars must still match the empty remainder.
        assert!(glob_match("abc***", "abc"));
        // Non-ASCII is matched per character, not per byte.
        assert!(glob_match("cjk/??.txt", "cjk/中文.txt"));
    }
}
