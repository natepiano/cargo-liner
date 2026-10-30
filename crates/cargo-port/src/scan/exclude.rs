use std::ffi::OsStr;

pub(super) use tui_pane::glob_segment_matches;

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
