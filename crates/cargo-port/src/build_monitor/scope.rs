//! The roots-and-revisions projection of a monitor scope that build
//! classification consumes.
//!
//! `crate::tui::compile_visibility::MonitorScopeKey` never leaves `crate::tui`.
//! It reaches this module only through the `From<&MonitorScopeKey>` conversion
//! defined beside that type, which drops the selected-row identity and keeps
//! the roots and revisions that identify what may be classified.

use crate::project::AcceptedCargoMetadataRevision;
use crate::project::CanonicalCheckoutRoot;
use crate::project::CanonicalWorkspaceRoot;
use crate::project::ProjectListRevision;

/// Sorted canonical roots plus the revisions that make them non-actionable
/// once either input changes.
///
/// Two selected rows that resolve to the same roots and revisions produce equal
/// keys; the compile-monitor generation, not this key, keeps their results
/// apart.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BuildScopeKey {
    canonical_checkout_roots:         Vec<CanonicalCheckoutRoot>,
    canonical_workspace_roots:        Vec<CanonicalWorkspaceRoot>,
    accepted_cargo_metadata_revision: AcceptedCargoMetadataRevision,
    project_list_revision:            ProjectListRevision,
}

impl BuildScopeKey {
    /// Build a key from roots that already carry the sort-and-dedup invariant
    /// established when the monitor scope was resolved. The conversion that
    /// calls this must not re-sort.
    pub(crate) const fn from_sorted_scope_roots(
        canonical_checkout_roots: Vec<CanonicalCheckoutRoot>,
        canonical_workspace_roots: Vec<CanonicalWorkspaceRoot>,
        accepted_cargo_metadata_revision: AcceptedCargoMetadataRevision,
        project_list_revision: ProjectListRevision,
    ) -> Self {
        Self {
            canonical_checkout_roots,
            canonical_workspace_roots,
            accepted_cargo_metadata_revision,
            project_list_revision,
        }
    }

    /// Sorted canonical checkout roots the scope covers.
    pub(crate) fn canonical_checkout_roots(&self) -> &[CanonicalCheckoutRoot] {
        &self.canonical_checkout_roots
    }

    /// Sorted canonical Cargo workspace roots the scope covers.
    pub(crate) fn canonical_workspace_roots(&self) -> &[CanonicalWorkspaceRoot] {
        &self.canonical_workspace_roots
    }

    /// Accepted `cargo metadata` revision this scope was resolved against.
    pub(crate) const fn accepted_cargo_metadata_revision(&self) -> AcceptedCargoMetadataRevision {
        self.accepted_cargo_metadata_revision
    }

    /// Visible project-list revision this scope was resolved against.
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
