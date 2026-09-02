//! Absolute-path normalization and canonicalization for coordination decisions.

use std::fs;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

/// Collapse `.` and `..` in an absolute path without reading the filesystem.
///
/// A caller comparing a path against a repository root needs this textual form first:
/// it keeps a path that lies inside the worktree inside the coordination domain even
/// when a symlinked directory would canonicalize it elsewhere.
///
/// Collapsing a `..` textually is only sound when every component to its left is a real
/// directory, so this belongs to a path the caller already trusts to be one — a directory the
/// harness reports itself sitting in, never a path it names as an edit target. On a filesystem
/// `alias/../held.rs` is `held.rs` only when `alias` is a directory; when `alias` is a symlink
/// the two name different files. A path whose `..` must be resolved for real goes to
/// [`canonicalize_through_nearest_existing_ancestor`] uncollapsed instead.
pub(crate) fn normalize_absolute_path(
    candidate_path: &Path,
) -> Result<PathBuf, AbsolutePathNormalizationError> {
    if !candidate_path.is_absolute() {
        return Err(AbsolutePathNormalizationError::NotAbsolute);
    }
    let mut normalized_path = PathBuf::new();
    for component in candidate_path.components() {
        match component {
            Component::Prefix(_) => {
                return Err(AbsolutePathNormalizationError::UnsupportedPrefix);
            },
            Component::RootDir => normalized_path.push(Path::new("/")),
            Component::CurDir => {},
            Component::ParentDir => {
                normalized_path.pop();
            },
            Component::Normal(component) => normalized_path.push(component),
        }
    }
    Ok(normalized_path)
}

/// Canonicalize the nearest existing ancestor and re-append the components below it.
///
/// Coordination decisions are made about paths that do not exist yet, so plain
/// canonicalization is unavailable; this resolves everything the filesystem already
/// knows about and leaves the rest of the path as the caller named it.
pub(crate) fn canonicalize_through_nearest_existing_ancestor(
    candidate_path: &Path,
) -> Result<PathBuf, AncestorCanonicalizationError> {
    let mut existing_ancestor = candidate_path.to_path_buf();
    let mut missing_components = Vec::new();
    while !existing_ancestor.exists() {
        let Some(component) = existing_ancestor.file_name() else {
            return Err(AncestorCanonicalizationError::NoExistingAncestor);
        };
        missing_components.push(component.to_os_string());
        if !existing_ancestor.pop() {
            return Err(AncestorCanonicalizationError::NoExistingAncestor);
        }
    }
    let mut resolved_path = fs::canonicalize(existing_ancestor)
        .map_err(|_| AncestorCanonicalizationError::AncestorUnavailable)?;
    for component in missing_components.into_iter().rev() {
        resolved_path.push(component);
    }
    Ok(resolved_path)
}

/// A path could not be reduced to an absolute form free of relative components.
pub(crate) enum AbsolutePathNormalizationError {
    /// The path is relative, so no repository comparison can be made from it.
    NotAbsolute,
    /// The path carries a platform prefix this coordination domain does not support.
    UnsupportedPrefix,
}

/// The nearest existing ancestor of a path could not be canonicalized.
pub(crate) enum AncestorCanonicalizationError {
    /// Walking upwards left no component that exists on this filesystem.
    NoExistingAncestor,
    /// An ancestor exists but the filesystem refused to resolve it.
    AncestorUnavailable,
}
