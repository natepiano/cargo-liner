use std::ffi::OsStr;

/// Directory-name glob patterns from `tui.exclude_dirs`.
///
/// A directory whose name matches any pattern is skipped by `phase1_discover`
/// and rejected by the watcher's `probe_new_projects`, so short-lived
/// directories — the `.tmpXXXXXX` trees `tempfile` creates during test runs,
/// scratch checkouts — never reach the project list.
///
/// Patterns match the directory name alone, never the full path.
#[derive(Clone, Debug, Default)]
pub(crate) struct ExcludeDirs {
    patterns: Vec<String>,
}

impl ExcludeDirs {
    /// Whether `dir_name` matches any configured pattern. Names that are not
    /// valid UTF-8 never match.
    pub(crate) fn excludes(&self, dir_name: &OsStr) -> bool {
        dir_name.to_str().is_some_and(|name| {
            self.patterns
                .iter()
                .any(|pattern| glob_segment_matches(pattern, name))
        })
    }
}

impl From<&[String]> for ExcludeDirs {
    fn from(patterns: &[String]) -> Self {
        Self {
            patterns: patterns
                .iter()
                .map(|pattern| pattern.trim())
                .filter(|pattern| !pattern.is_empty())
                .map(str::to_string)
                .collect(),
        }
    }
}

/// Match one path segment against a glob pattern supporting `*` (any run of
/// characters, including empty) and `?` (exactly one character). Shared with
/// `workspace_pattern_matches_segments`, which applies it per `/`-delimited
/// segment of a `[workspace] members` pattern.
pub(super) fn glob_segment_matches(pattern: &str, value: &str) -> bool {
    fn matches(pattern: &[u8], value: &[u8]) -> bool {
        match pattern.split_first() {
            None => value.is_empty(),
            Some((b'*', rest)) => {
                matches(rest, value) || (!value.is_empty() && matches(pattern, &value[1..]))
            },
            Some((b'?', rest)) => !value.is_empty() && matches(rest, &value[1..]),
            Some((head, rest)) => {
                !value.is_empty() && *head == value[0] && matches(rest, &value[1..])
            },
        }
    }

    matches(pattern.as_bytes(), value.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exclude_dirs(patterns: &[&str]) -> ExcludeDirs {
        let owned: Vec<String> = patterns.iter().map(|p| (*p).to_string()).collect();
        ExcludeDirs::from(owned.as_slice())
    }

    #[test]
    fn empty_config_excludes_nothing() {
        let excludes = ExcludeDirs::default();
        assert!(!excludes.excludes(OsStr::new(".tmpAbC123")));
        assert!(!excludes.excludes(OsStr::new("cargo-port")));
    }

    #[test]
    fn trailing_star_matches_tempfile_directories() {
        let excludes = exclude_dirs(&[".tmp*"]);
        assert!(excludes.excludes(OsStr::new(".tmpAbC123")));
        assert!(excludes.excludes(OsStr::new(".tmp")));
        assert!(!excludes.excludes(OsStr::new("tmp")));
        assert!(!excludes.excludes(OsStr::new("my.tmpdir")));
    }

    #[test]
    fn patterns_match_the_whole_name() {
        let excludes = exclude_dirs(&["build"]);
        assert!(excludes.excludes(OsStr::new("build")));
        assert!(!excludes.excludes(OsStr::new("rebuild")));
        assert!(!excludes.excludes(OsStr::new("build-scripts")));
    }

    #[test]
    fn question_mark_matches_exactly_one_character() {
        let excludes = exclude_dirs(&["tmp?"]);
        assert!(excludes.excludes(OsStr::new("tmpa")));
        assert!(!excludes.excludes(OsStr::new("tmp")));
        assert!(!excludes.excludes(OsStr::new("tmpab")));
    }

    #[test]
    fn any_pattern_matching_excludes_the_directory() {
        let excludes = exclude_dirs(&["node_modules", ".tmp*"]);
        assert!(excludes.excludes(OsStr::new("node_modules")));
        assert!(excludes.excludes(OsStr::new(".tmpZZ")));
        assert!(!excludes.excludes(OsStr::new("src")));
    }

    #[test]
    fn blank_patterns_are_dropped() {
        let excludes = exclude_dirs(&["", "   ", ".tmp*"]);
        assert!(!excludes.excludes(OsStr::new("anything")));
        assert!(excludes.excludes(OsStr::new(".tmpQ")));
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_from_patterns() {
        let excludes = exclude_dirs(&["  .tmp*  "]);
        assert!(excludes.excludes(OsStr::new(".tmpQ")));
    }
}
