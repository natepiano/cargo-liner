//! Lexical reservation scopes and repository path comparison policy.

mod antichain;

use std::fmt;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use crate::ids::InvalidReservationScopePath;
use crate::ids::ReservationScopePath;
pub(crate) use crate::ledger::ReservationScope;
pub(crate) use crate::ledger::ReservationScopeSet;
pub(crate) use crate::ledger::ScopeKind;

const FILE_SCOPE_PREFIX: &str = "file:";
const GIT_CONFIG_FILE_NAME: &str = "config";
const TREE_SCOPE_PREFIX: &str = "tree:";

/// Whether repository paths compare with their original component case.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PathCase {
    /// Compare each UTF-8 component exactly.
    Sensitive,
    /// Compare each UTF-8 component after Unicode lowercase folding.
    Insensitive,
}

/// A non-empty set of lexically valid scopes before antichain reduction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeclaredReservationScopeSet(ReservationScopeSet);

/// A lexical path and declared file-versus-tree meaning from the command line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeclaredReservationScope(ReservationScope);

impl PathCase {
    /// Read `core.ignoreCase` from the common git configuration without a subprocess.
    pub(crate) fn read(common_git_directory: &Path) -> Result<Self, PathCaseError> {
        let configuration = fs::read_to_string(common_git_directory.join(GIT_CONFIG_FILE_NAME))?;
        parse_path_case(&configuration)
    }

    pub(super) fn component_eq(self, left: &str, right: &str) -> bool {
        match self {
            Self::Sensitive => left == right,
            Self::Insensitive => left.to_lowercase() == right.to_lowercase(),
        }
    }
}

impl DeclaredReservationScopeSet {
    /// Parse path arguments and preserve their declared scope kinds.
    pub(crate) fn parse(
        paths: Vec<PathBuf>,
        default_kind: ScopeKind,
    ) -> Result<Self, DeclaredReservationScopeSetError> {
        let scopes = paths
            .into_iter()
            .map(|path| DeclaredReservationScope::parse(path, default_kind).map(|scope| scope.0))
            .collect::<Result<Vec<_>, _>>()?;
        ReservationScopeSet::try_from(scopes)
            .map(Self)
            .map_err(|_| DeclaredReservationScopeSetError::Empty)
    }

    /// Reduce the declared scopes to a minimal component antichain.
    pub(crate) fn into_minimal_antichain(self, path_case: PathCase) -> ReservationScopeSet {
        antichain::reduce(self.0, path_case)
    }
}

impl DeclaredReservationScope {
    fn parse(
        path: PathBuf,
        default_kind: ScopeKind,
    ) -> Result<Self, DeclaredReservationScopeError> {
        let path = path
            .into_os_string()
            .into_string()
            .map_err(DeclaredReservationScopeError::NonUtf8)?;
        let (kind, path) = path.strip_prefix(FILE_SCOPE_PREFIX).map_or_else(
            || {
                path.strip_prefix(TREE_SCOPE_PREFIX)
                    .map_or((default_kind, path.as_str()), |path| {
                        (ScopeKind::Tree, path)
                    })
            },
            |path| (ScopeKind::File, path),
        );
        let path = ReservationScopePath::from_str(path)
            .map_err(DeclaredReservationScopeError::InvalidPath)?;
        Ok(Self(ReservationScope { path, kind }))
    }
}

impl ReservationScope {
    /// Return whether this scope conflicts with another scope.
    pub(crate) fn overlaps(&self, other: &Self, path_case: PathCase) -> bool {
        antichain::overlaps(self, other, path_case)
    }
}

impl ReservationScopeSet {
    /// Return a minimal component antichain under the repository's case policy.
    pub(crate) fn minimal_antichain(&self, path_case: PathCase) -> Self {
        antichain::reduce(self.clone(), path_case)
    }
}

/// A failure while reading or interpreting `core.ignoreCase`.
#[derive(Debug)]
pub(crate) enum PathCaseError {
    /// The common git configuration could not be read.
    Io(std::io::Error),
    /// `core.ignoreCase` used a value git does not recognize as Boolean.
    InvalidValue(String),
}

impl fmt::Display for PathCaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "could not read git path-case policy: {error}"),
            Self::InvalidValue(value) => {
                write!(formatter, "invalid core.ignoreCase Boolean value: {value}")
            },
        }
    }
}

impl std::error::Error for PathCaseError {}

impl From<std::io::Error> for PathCaseError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

/// A failure while parsing one declared command-line scope.
#[derive(Debug)]
pub(crate) enum DeclaredReservationScopeError {
    /// The operating-system path was not UTF-8.
    NonUtf8(std::ffi::OsString),
    /// The UTF-8 spelling was not a lexical repository path.
    InvalidPath(InvalidReservationScopePath),
}

impl fmt::Display for DeclaredReservationScopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonUtf8(path) => write!(
                formatter,
                "reservation paths must be UTF-8; correct this path and retry: {}",
                path.to_string_lossy()
            ),
            Self::InvalidPath(error) => write!(
                formatter,
                "{error}; use a normalized repository-relative path and retry"
            ),
        }
    }
}

impl std::error::Error for DeclaredReservationScopeError {}

/// A failure while constructing a non-empty declared scope set.
#[derive(Debug)]
pub(crate) enum DeclaredReservationScopeSetError {
    /// One path did not satisfy the lexical scope contract.
    InvalidScope(DeclaredReservationScopeError),
    /// No paths were provided.
    Empty,
}

impl fmt::Display for DeclaredReservationScopeSetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScope(error) => error.fmt(formatter),
            Self::Empty => formatter.write_str("provide at least one reservation path and retry"),
        }
    }
}

impl std::error::Error for DeclaredReservationScopeSetError {}

impl From<DeclaredReservationScopeError> for DeclaredReservationScopeSetError {
    fn from(error: DeclaredReservationScopeError) -> Self { Self::InvalidScope(error) }
}

fn parse_path_case(configuration: &str) -> Result<PathCase, PathCaseError> {
    let mut in_core_section = false;
    let mut path_case = PathCase::Sensitive;
    for line in configuration.lines() {
        let line = line
            .split_once(['#', ';'])
            .map_or(line, |(before_comment, _)| before_comment)
            .trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_core_section = line[1..line.len() - 1].trim().eq_ignore_ascii_case("core");
            continue;
        }
        if !in_core_section || line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            if line.eq_ignore_ascii_case("ignorecase") {
                path_case = PathCase::Insensitive;
            }
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("ignorecase") {
            continue;
        }
        path_case = match value.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => PathCase::Insensitive,
            "false" | "no" | "off" | "0" => PathCase::Sensitive,
            _ => return Err(PathCaseError::InvalidValue(value.trim().to_owned())),
        };
    }
    Ok(path_case)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;

    use super::DeclaredReservationScopeSet;
    use super::PathCase;
    use super::ReservationScope;
    use super::ScopeKind;
    use super::parse_path_case;

    #[test]
    fn parsing_is_lexical_and_accepts_future_paths() {
        let scopes = DeclaredReservationScopeSet::parse(
            vec![PathBuf::from("file:future/generated.rs")],
            ScopeKind::Tree,
        )
        .expect("future path should parse")
        .into_minimal_antichain(PathCase::Sensitive);

        assert_eq!(scopes.as_slice()[0].kind, ScopeKind::File);
        for invalid in ["../outside", "/absolute", "tree:crates/../outside"] {
            assert!(
                DeclaredReservationScopeSet::parse(vec![PathBuf::from(invalid)], ScopeKind::Tree)
                    .is_err()
            );
        }
    }

    #[test]
    fn component_ancestry_does_not_treat_siblings_as_prefixes() {
        let left = reservation_scope("crates/hana_kana", ScopeKind::Tree);
        let right = reservation_scope("crates/hana_kana_extra", ScopeKind::Tree);

        assert!(!left.overlaps(&right, PathCase::Sensitive));
    }

    #[test]
    fn case_insensitive_comparison_blocks_component_case_variants() {
        let left = reservation_scope("Crates/Hana", ScopeKind::Tree);
        let right = reservation_scope("crates/hana/src/lib.rs", ScopeKind::File);

        assert!(left.overlaps(&right, PathCase::Insensitive));
        assert!(!left.overlaps(&right, PathCase::Sensitive));
    }

    #[test]
    fn bare_ignore_case_key_enables_case_insensitive_comparison() {
        let path_case = parse_path_case("[core]\nignorecase\n")
            .expect("bare ignoreCase should be a valid Boolean true");

        assert_eq!(path_case, PathCase::Insensitive);
    }

    fn reservation_scope(path: &str, kind: ScopeKind) -> ReservationScope {
        DeclaredReservationScopeSet::parse(vec![PathBuf::from(path)], kind)
            .expect("scope should parse")
            .into_minimal_antichain(PathCase::Sensitive)
            .as_slice()[0]
            .clone()
    }
}
