//! The roots-and-revisions projection of a monitor scope that build
//! classification consumes.
//!
//! `crate::tui::compile_visibility::MonitorScopeKey` never leaves `crate::tui`.
//! It reaches this module only through the `From<&MonitorScopeKey>` conversion
//! defined beside that type, which drops the selected-row identity and keeps
//! the roots and revisions that identify what may be classified.

use crate::project::AcceptedCargoMetadataRevision;
use crate::project::CanonicalCheckoutRoot;
use crate::project::CanonicalPathResolution;
use crate::project::CanonicalTargetDirectory;
use crate::project::CanonicalWorkspaceRoot;
use crate::project::ProjectListRevision;

/// The canonical roots a monitor scope covers, sorted, deduplicated, and
/// non-empty on both sides.
///
/// A scope naming no root has nothing an observed build could be attributed to.
/// That state is refused where the roots are resolved, so nothing downstream
/// re-checks it per cycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CoveredScopeRoots {
    canonical_checkout_roots:  Vec<CanonicalCheckoutRoot>,
    canonical_workspace_roots: Vec<CanonicalWorkspaceRoot>,
}

/// Whether a resolved set of roots covers anything at all.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ScopeRootCoverage {
    /// At least one checkout root and one workspace root.
    Covered(CoveredScopeRoots),
    /// One side was empty, so the scope covers nothing.
    NoRootsCovered,
}

impl CoveredScopeRoots {
    /// Sort and deduplicate one resolution's roots, refusing an empty side.
    /// This is the only constructor, so every holder of the value has the
    /// non-empty guarantee without asserting it.
    pub(crate) fn from_roots(
        mut canonical_checkout_roots: Vec<CanonicalCheckoutRoot>,
        mut canonical_workspace_roots: Vec<CanonicalWorkspaceRoot>,
    ) -> ScopeRootCoverage {
        if canonical_checkout_roots.is_empty() || canonical_workspace_roots.is_empty() {
            return ScopeRootCoverage::NoRootsCovered;
        }
        canonical_checkout_roots
            .sort_by(|left, right| left.path().as_path().cmp(right.path().as_path()));
        canonical_checkout_roots.dedup();
        canonical_workspace_roots
            .sort_by(|left, right| left.path().as_path().cmp(right.path().as_path()));
        canonical_workspace_roots.dedup();
        ScopeRootCoverage::Covered(Self {
            canonical_checkout_roots,
            canonical_workspace_roots,
        })
    }

    /// Sorted canonical checkout roots, never empty.
    pub(crate) fn canonical_checkout_roots(&self) -> &[CanonicalCheckoutRoot] {
        &self.canonical_checkout_roots
    }

    /// Sorted canonical Cargo workspace roots, never empty.
    pub(crate) fn canonical_workspace_roots(&self) -> &[CanonicalWorkspaceRoot] {
        &self.canonical_workspace_roots
    }
}

/// The live target-directory resolutions one scope resolve observed, in the
/// order that resolve visited the scope's workspaces.
///
/// [`crate::project::CargoWorkspaceView::target_directory_resolution`] reads the
/// filesystem on every call, so a target directory that was missing when the
/// scope was last resolved and now exists — or a `target` symlink created or
/// retargeted at a different directory — produces a different value here while
/// [`AcceptedCargoMetadataRevision`] and [`ProjectListRevision`] are both
/// unchanged. Carrying it in [`BuildScopeKey`] is what makes that filesystem
/// change invalidate the classification and the actionability that were
/// resolved under the old resolution.
///
/// Order is resolve order rather than a sort: two resolves of the same selected
/// row visit the same workspaces in the same order, and a different selected row
/// is already a different `crate::tui::compile_visibility::MonitorScopeKey`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct LiveTargetDirectoryRevision {
    target_directory_resolutions: Vec<CanonicalPathResolution<CanonicalTargetDirectory>>,
}

impl From<Vec<CanonicalPathResolution<CanonicalTargetDirectory>>> for LiveTargetDirectoryRevision {
    fn from(
        target_directory_resolutions: Vec<CanonicalPathResolution<CanonicalTargetDirectory>>,
    ) -> Self {
        Self {
            target_directory_resolutions,
        }
    }
}

/// Covered canonical roots plus the revisions that make them non-actionable
/// once either input changes.
///
/// Two selected rows that resolve to the same roots and revisions produce equal
/// keys; the compile-monitor generation, not this key, keeps their results
/// apart.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BuildScopeKey {
    covered_scope_roots:              CoveredScopeRoots,
    accepted_cargo_metadata_revision: AcceptedCargoMetadataRevision,
    project_list_revision:            ProjectListRevision,
    live_target_directory_revision:   LiveTargetDirectoryRevision,
}

impl BuildScopeKey {
    /// Build a key from roots that already carry the non-empty,
    /// sort-and-dedup invariant established when the monitor scope was
    /// resolved.
    pub(crate) const fn from_covered_scope_roots(
        covered_scope_roots: CoveredScopeRoots,
        accepted_cargo_metadata_revision: AcceptedCargoMetadataRevision,
        project_list_revision: ProjectListRevision,
        live_target_directory_revision: LiveTargetDirectoryRevision,
    ) -> Self {
        Self {
            covered_scope_roots,
            accepted_cargo_metadata_revision,
            project_list_revision,
            live_target_directory_revision,
        }
    }

    /// The covered roots as one value, for the retention rule that keeps prior
    /// rows on screen when a new scope covers exactly the same roots.
    pub(crate) const fn covered_scope_roots(&self) -> &CoveredScopeRoots {
        &self.covered_scope_roots
    }

    /// Sorted canonical checkout roots the scope covers.
    pub(crate) fn canonical_checkout_roots(&self) -> &[CanonicalCheckoutRoot] {
        self.covered_scope_roots.canonical_checkout_roots()
    }

    /// Sorted canonical Cargo workspace roots the scope covers.
    pub(crate) fn canonical_workspace_roots(&self) -> &[CanonicalWorkspaceRoot] {
        self.covered_scope_roots.canonical_workspace_roots()
    }

    /// A key covering one checkout and workspace root, for tests that need an
    /// actionable scope without resolving one from a live index.
    #[cfg(test)]
    pub(crate) fn for_test(canonical_root: crate::project::AbsolutePath) -> Self {
        Self {
            covered_scope_roots:              CoveredScopeRoots {
                canonical_checkout_roots:  vec![CanonicalCheckoutRoot::for_test(
                    canonical_root.clone(),
                )],
                canonical_workspace_roots: vec![CanonicalWorkspaceRoot::for_test(canonical_root)],
            },
            accepted_cargo_metadata_revision: AcceptedCargoMetadataRevision::default(),
            project_list_revision:            ProjectListRevision::default(),
            live_target_directory_revision:   LiveTargetDirectoryRevision::default(),
        }
    }

    /// Accepted `cargo metadata` revision this scope was resolved against.
    #[cfg(test)]
    pub(crate) const fn accepted_cargo_metadata_revision(&self) -> AcceptedCargoMetadataRevision {
        self.accepted_cargo_metadata_revision
    }

    /// Visible project-list revision this scope was resolved against.
    #[cfg(test)]
    pub(crate) const fn project_list_revision(&self) -> ProjectListRevision {
        self.project_list_revision
    }
}

/// Whether a monitor scope authorizes build-monitor work.
///
/// The five monitor-scope resolution states collapse to these two before they
/// cross into `build_monitor`, so no consumer here restates the actionability
/// rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BuildScopeActionability {
    /// The scope resolved and may be classified and acted on.
    Actionable(BuildScopeKey),
    /// The scope did not resolve; nothing in it is actionable.
    NotActionable,
}
