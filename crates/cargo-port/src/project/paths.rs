use std::borrow::Borrow;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;

/// An absolute filesystem path. Used as `HashMap` keys and for filesystem operations.
/// Wraps `PathBuf`. Created from absolute paths only.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct AbsolutePath(PathBuf);

impl AbsolutePath {
    pub(crate) fn as_path(&self) -> &Path { &self.0 }

    pub(crate) fn to_path_buf(&self) -> PathBuf { self.0.clone() }

    pub(crate) fn display_path(&self) -> DisplayPath {
        DisplayPath::new(home_relative_path(&self.0))
    }
}

impl AsRef<Path> for AbsolutePath {
    fn as_ref(&self) -> &Path { &self.0 }
}

impl Borrow<Path> for AbsolutePath {
    fn borrow(&self) -> &Path { &self.0 }
}

impl fmt::Display for AbsolutePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.display().fmt(f) }
}

impl From<PathBuf> for AbsolutePath {
    fn from(path: PathBuf) -> Self {
        debug_assert!(
            path.is_absolute(),
            "AbsolutePath requires an absolute path: {}",
            path.display()
        );
        Self(path)
    }
}

impl From<&Path> for AbsolutePath {
    fn from(path: &Path) -> Self {
        debug_assert!(
            path.is_absolute(),
            "AbsolutePath requires an absolute path: {}",
            path.display()
        );
        Self(path.to_path_buf())
    }
}

/// A display path for the UI (e.g. `~/rust/bevy`). Never used as a `HashMap` key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DisplayPath(String);

impl DisplayPath {
    pub(crate) const fn new(s: String) -> Self { Self(s) }

    pub(crate) fn as_str(&self) -> &str { &self.0 }

    pub(crate) fn into_string(self) -> String { self.0 }
}

impl fmt::Display for DisplayPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
}

impl AsRef<str> for DisplayPath {
    fn as_ref(&self) -> &str { self.as_str() }
}

/// The last directory component of a project's root checkout path.
/// Used for top-level root labels, disambiguation, and worktree label fallback.
/// Never derived from Cargo metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RootDirectoryName(pub(super) String);

impl RootDirectoryName {
    pub(crate) fn as_str(&self) -> &str { &self.0 }

    pub(crate) fn into_string(self) -> String { self.0 }
}

impl fmt::Display for RootDirectoryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
}

impl AsRef<str> for RootDirectoryName {
    fn as_ref(&self) -> &str { self.as_str() }
}

/// The Cargo package name when present, otherwise the directory leaf.
/// Used for workspace member rows, vendored rows, detail title bars, and
/// finder parent labels. Only available on `RustProject<Kind>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PackageName(pub(super) String);

impl PackageName {
    pub(crate) fn as_str(&self) -> &str { &self.0 }

    pub(crate) fn into_string(self) -> String { self.0 }
}

impl fmt::Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
}

impl AsRef<str> for PackageName {
    fn as_ref(&self) -> &str { self.as_str() }
}

/// Returns a `~/`-prefixed path if under the home directory, otherwise the absolute path.
pub(crate) fn home_relative_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rel) = path.strip_prefix(&home)
    {
        return format!("~/{}", rel.display());
    }
    path.display().to_string()
}

/// Extract the last path component as a `String`, returning an empty string
/// if the path has no file name or is not valid UTF-8.
pub(super) fn directory_leaf(path: &Path) -> String {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("")
        .to_string()
}
