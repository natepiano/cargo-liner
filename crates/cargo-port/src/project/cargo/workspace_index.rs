//! Immutable workspace views derived from accepted `cargo metadata` results.
//!
//! The index records canonical checkout, workspace, member, target-directory,
//! package, and target-source identities. It deliberately contains only the
//! `cargo metadata --no-deps` view: registry and Git dependency records are
//! not a complete source of workspace ownership.

use std::collections::HashMap;
use std::sync::Arc;

use ExactWorkspaceOwnershipEvidence::Ambiguous;
use ExactWorkspaceOwnershipEvidence::Unavailable;
use ExactWorkspaceOwnershipEvidence::Unique;
use cargo_metadata::PackageId;
use cargo_metadata::TargetKind;

#[cfg(test)]
use super::PackageRecord;
use super::TargetRecord;
use super::WorkspaceMetadata;
use super::WorkspaceMetadataStore;
use crate::project::AbsolutePath;
use crate::project::AcceptedCargoMetadataRevision;

/// Outcome of resolving a declared path to a canonical identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CanonicalPathResolution<T> {
    /// The declared path resolved to a canonical identity.
    Resolved(T),
    /// The declared path could not be resolved to a canonical identity.
    Unresolved,
}

/// Canonical root of a checked-out Cargo workspace.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CanonicalCheckoutRoot(AbsolutePath);

impl CanonicalCheckoutRoot {
    pub(crate) const fn path(&self) -> &AbsolutePath { &self.0 }

    /// Name one canonical checkout root directly, for tests that need a scope
    /// without a live index behind it.
    #[cfg(test)]
    pub(crate) const fn for_test(canonical_path: AbsolutePath) -> Self { Self(canonical_path) }
}

/// Canonical root reported by Cargo for a workspace.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CanonicalWorkspaceRoot(AbsolutePath);

impl CanonicalWorkspaceRoot {
    pub(crate) const fn path(&self) -> &AbsolutePath { &self.0 }

    /// Name one canonical workspace root directly, for tests that need a scope
    /// without a live index behind it.
    #[cfg(test)]
    pub(crate) const fn for_test(canonical_path: AbsolutePath) -> Self { Self(canonical_path) }
}

/// Canonical root directory of a package in an indexed workspace.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CanonicalMemberRoot(AbsolutePath);

impl CanonicalMemberRoot {
    pub(crate) const fn path(&self) -> &AbsolutePath { &self.0 }
}

/// Canonical project path used to resolve visible checkout or member ownership.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CanonicalVisibleProjectOwner(AbsolutePath);

impl CanonicalVisibleProjectOwner {
    const fn path(&self) -> &AbsolutePath { &self.0 }
}

/// Canonical Cargo target directory for a workspace.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CanonicalTargetDirectory(AbsolutePath);

impl CanonicalTargetDirectory {
    pub(crate) const fn path(&self) -> &AbsolutePath { &self.0 }

    /// Name a target directory from a path the caller already canonicalized.
    pub(crate) const fn from_canonical_path(canonical_path: AbsolutePath) -> Self {
        Self(canonical_path)
    }
}

/// Canonical source-file identity of a Cargo target.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CanonicalTargetSource(AbsolutePath);

impl CanonicalTargetSource {
    pub(crate) const fn path(&self) -> &AbsolutePath { &self.0 }
}

/// Revision of visible project-list ownership input.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ProjectListRevision(u64);

impl ProjectListRevision {
    pub(crate) const fn advance(&mut self) { self.0 = self.0.saturating_add(1); }
}

/// Inputs that determine an immutable [`CargoWorkspaceIndex`] view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CargoWorkspaceIndexRevision {
    accepted_cargo_metadata_revision: AcceptedCargoMetadataRevision,
    project_list_revision:            ProjectListRevision,
}

impl CargoWorkspaceIndexRevision {
    const fn new(
        accepted_cargo_metadata_revision: AcceptedCargoMetadataRevision,
        project_list_revision: ProjectListRevision,
    ) -> Self {
        Self {
            accepted_cargo_metadata_revision,
            project_list_revision,
        }
    }

    pub(crate) const fn accepted_cargo_metadata_revision(&self) -> AcceptedCargoMetadataRevision {
        self.accepted_cargo_metadata_revision
    }

    pub(crate) const fn project_list_revision(&self) -> ProjectListRevision {
        self.project_list_revision
    }
}

/// Whether a [`CargoWorkspaceIndex`] has accepted its first revision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum CargoWorkspaceIndexRevisionState {
    /// No metadata-store snapshot has been accepted into the index.
    #[default]
    Uninitialized,
    /// The index reflects the contained metadata and project-list revisions.
    Accepted(crate::project::CargoWorkspaceIndexRevision),
}

/// One package and its target-source identities within an indexed workspace.
#[derive(Clone, Debug)]
pub(crate) struct CargoPackageIdentity {
    declared_member_root:   AbsolutePath,
    member_root_resolution: CanonicalPathResolution<CanonicalMemberRoot>,
    package_id:             PackageId,
    targets:                Vec<CargoTargetIdentity>,
}

impl CargoPackageIdentity {
    pub(crate) const fn declared_member_root_path(&self) -> &AbsolutePath {
        &self.declared_member_root
    }

    pub(crate) const fn member_root_resolution(
        &self,
    ) -> &crate::project::CanonicalPathResolution<crate::project::CanonicalMemberRoot> {
        &self.member_root_resolution
    }

    pub(crate) const fn package_id(&self) -> &PackageId { &self.package_id }

    pub(crate) fn targets(&self) -> impl Iterator<Item = &crate::project::CargoTargetIdentity> {
        self.targets.iter()
    }
}

/// Cargo-reported target definition paired with its canonical source resolution.
#[derive(Clone, Debug)]
pub(crate) struct CargoTargetIdentity {
    record:                      TargetRecord,
    canonical_source_resolution: CanonicalPathResolution<CanonicalTargetSource>,
}

impl CargoTargetIdentity {
    pub(crate) fn name(&self) -> &str { &self.record.name }

    pub(crate) fn kinds(&self) -> &[TargetKind] { &self.record.kinds }

    #[cfg(test)]
    pub(crate) fn required_features(&self) -> &[String] { &self.record.required_features }

    #[cfg(test)]
    pub(crate) const fn declared_source_path(&self) -> &AbsolutePath { &self.record.src_path }

    pub(crate) const fn canonical_source_resolution(
        &self,
    ) -> &crate::project::CanonicalPathResolution<crate::project::CanonicalTargetSource> {
        &self.canonical_source_resolution
    }
}

/// One workspace's immutable ownership data.
#[derive(Clone, Debug)]
pub(crate) struct CargoWorkspaceView {
    declared_checkout_root:    AbsolutePath,
    checkout_root_resolution:  CanonicalPathResolution<CanonicalCheckoutRoot>,
    #[cfg(test)]
    declared_workspace_root:   AbsolutePath,
    workspace_root_resolution: CanonicalPathResolution<CanonicalWorkspaceRoot>,
    declared_target_directory: AbsolutePath,
    packages:                  Vec<CargoPackageIdentity>,
}

impl CargoWorkspaceView {
    pub(crate) const fn declared_checkout_root_path(&self) -> &AbsolutePath {
        &self.declared_checkout_root
    }

    pub(crate) const fn checkout_root_resolution(
        &self,
    ) -> &crate::project::CanonicalPathResolution<crate::project::CanonicalCheckoutRoot> {
        &self.checkout_root_resolution
    }

    #[cfg(test)]
    pub(crate) const fn declared_workspace_root_path(&self) -> &AbsolutePath {
        &self.declared_workspace_root
    }

    pub(crate) const fn workspace_root_resolution(
        &self,
    ) -> &crate::project::CanonicalPathResolution<crate::project::CanonicalWorkspaceRoot> {
        &self.workspace_root_resolution
    }

    pub(crate) const fn declared_target_directory_path(&self) -> &AbsolutePath {
        &self.declared_target_directory
    }

    /// Resolve the declared Cargo target directory on every call so a newly
    /// created directory or retargeted symlink is observed without an index rebuild.
    pub(crate) fn target_directory_resolution(
        &self,
    ) -> crate::project::CanonicalPathResolution<crate::project::CanonicalTargetDirectory> {
        resolve_canonical_path(&self.declared_target_directory, CanonicalTargetDirectory)
    }

    pub(crate) fn packages(&self) -> impl Iterator<Item = &crate::project::CargoPackageIdentity> {
        self.packages.iter()
    }
}

/// Whether a visible target has exact ownership in the shared workspace index.
#[derive(Clone, Copy, Debug)]
pub(crate) enum VisibleTargetWorkspaceOwnership<'a> {
    /// Exact canonical source and project evidence identifies this workspace.
    Indexed(&'a CargoWorkspaceView),
    /// Exact canonical source and project evidence cannot identify one workspace.
    Ambiguous,
    /// Neither canonical source nor project ownership resolved in the index.
    NotIndexed,
}

/// Whether one visible project path has exact workspace ownership in the
/// shared index.
#[derive(Clone, Copy, Debug)]
pub(crate) enum VisibleProjectWorkspaceOwnership<'a> {
    /// One canonical checkout, workspace, or member root identifies this
    /// workspace.
    Indexed(&'a CargoWorkspaceView),
    /// More than one workspace claims the canonical path.
    Ambiguous,
    /// The path has no canonical ownership record in this index.
    NotIndexed,
}

/// Number of indexed workspaces identified by one exact canonical path.
#[derive(Clone, Copy, Debug)]
enum ExactWorkspaceOwnershipEvidence<'a> {
    /// The path did not resolve or resolved to no indexed workspace.
    Unavailable,
    /// The path identifies exactly one indexed workspace.
    Unique(usize),
    /// The path identifies more than one indexed workspace.
    Ambiguous(&'a [usize]),
}

/// Result of reconciling exact canonical project and source evidence.
#[derive(Clone, Copy, Debug)]
enum VisibleTargetOwnershipDecision {
    Indexed(usize),
    Ambiguous,
    NotIndexed,
}

/// All workspace indices associated with one exact canonical path.
#[derive(Debug, Default)]
struct CanonicalWorkspaceCandidates(Vec<usize>);

impl CanonicalWorkspaceCandidates {
    fn include(&mut self, workspace_index: usize) {
        if !self.0.contains(&workspace_index) {
            self.0.push(workspace_index);
        }
    }

    fn ownership_evidence(&self) -> ExactWorkspaceOwnershipEvidence<'_> {
        match self.0.as_slice() {
            [] => ExactWorkspaceOwnershipEvidence::Unavailable,
            [workspace_index] => ExactWorkspaceOwnershipEvidence::Unique(*workspace_index),
            workspace_indices => ExactWorkspaceOwnershipEvidence::Ambiguous(workspace_indices),
        }
    }
}

/// Whether an accepted-metadata or project-list change produced a fresh index.
///
/// A rebuild replaces the handle instead of mutating the shared index, so an
/// in-flight worker request that already cloned the old handle keeps reading
/// the exact index its revision stamps name.
#[derive(Clone, Debug)]
pub(crate) enum WorkspaceIndexRebuild {
    /// The inputs moved; this fresh index supersedes the previous handle.
    Replaced(Arc<CargoWorkspaceIndex>),
    /// Neither accepted metadata nor the visible project list changed.
    Unchanged,
}

/// App-owned cache of immutable workspace ownership views.
#[derive(Debug, Default)]
pub(crate) struct CargoWorkspaceIndex {
    revision:             CargoWorkspaceIndexRevisionState,
    workspaces:           Vec<CargoWorkspaceView>,
    workspaces_by_root:   HashMap<AbsolutePath, CanonicalWorkspaceCandidates>,
    workspaces_by_source: HashMap<AbsolutePath, CanonicalWorkspaceCandidates>,
    workspaces_by_owner:  HashMap<AbsolutePath, CanonicalWorkspaceCandidates>,
    #[cfg(test)]
    rebuild_count:        u64,
}

impl CargoWorkspaceIndex {
    pub(crate) fn from_metadata_store(
        metadata_store: &WorkspaceMetadataStore,
        project_list_revision: ProjectListRevision,
    ) -> Self {
        let mut cargo_workspace_index = Self::default();
        cargo_workspace_index.rebuild(metadata_store, project_list_revision);
        cargo_workspace_index
    }

    /// Rebuild only when accepted metadata or the visible project-list input
    /// changes. Calling this from an ordinary event-loop wake leaves the
    /// immutable view intact.
    ///
    /// A change produces a whole fresh index rather than mutating this one, so
    /// a worker request holding an [`Arc`] clone keeps the exact index it was
    /// asked under across the thread boundary. The cost is one allocation per
    /// accepted-metadata change.
    pub(crate) fn rebuild_if_changed(
        &self,
        metadata_store: &WorkspaceMetadataStore,
        project_list_revision: ProjectListRevision,
    ) -> WorkspaceIndexRebuild {
        let accepted_cargo_metadata_revision = metadata_store.accepted_cargo_metadata_revision();
        if matches!(
            self.revision(),
            CargoWorkspaceIndexRevisionState::Accepted(revision)
                if revision.accepted_cargo_metadata_revision()
                    == accepted_cargo_metadata_revision
                    && revision.project_list_revision() == project_list_revision
        ) {
            return WorkspaceIndexRebuild::Unchanged;
        }
        let mut rebuilt = Self::default();
        #[cfg(test)]
        {
            rebuilt.rebuild_count = self.rebuild_count;
        }
        rebuilt.rebuild(metadata_store, project_list_revision);
        WorkspaceIndexRebuild::Replaced(Arc::new(rebuilt))
    }

    pub(crate) fn workspaces(&self) -> impl Iterator<Item = &CargoWorkspaceView> {
        self.workspaces.iter()
    }

    /// Resolve a workspace through an exact canonical target source, checkout
    /// root, Cargo workspace root, or package member root identity.
    pub(crate) fn workspace_for_visible_target(
        &self,
        source_path: &AbsolutePath,
        project_path: &AbsolutePath,
    ) -> VisibleTargetWorkspaceOwnership<'_> {
        let source_evidence = match resolve_canonical_path(source_path, CanonicalTargetSource) {
            CanonicalPathResolution::Resolved(source) => {
                exact_workspace_ownership_evidence(&self.workspaces_by_source, source.path())
            },
            CanonicalPathResolution::Unresolved => ExactWorkspaceOwnershipEvidence::Unavailable,
        };
        let project_evidence =
            match resolve_canonical_path(project_path, CanonicalVisibleProjectOwner) {
                CanonicalPathResolution::Resolved(project_owner) => {
                    exact_workspace_ownership_evidence(
                        &self.workspaces_by_owner,
                        project_owner.path(),
                    )
                },
                CanonicalPathResolution::Unresolved => ExactWorkspaceOwnershipEvidence::Unavailable,
            };

        match reconcile_visible_target_ownership(project_evidence, source_evidence) {
            VisibleTargetOwnershipDecision::Indexed(workspace_index) => self
                .workspaces
                .get(workspace_index)
                .map_or(VisibleTargetWorkspaceOwnership::Ambiguous, |workspace| {
                    VisibleTargetWorkspaceOwnership::Indexed(workspace)
                }),
            VisibleTargetOwnershipDecision::Ambiguous => VisibleTargetWorkspaceOwnership::Ambiguous,
            VisibleTargetOwnershipDecision::NotIndexed => {
                VisibleTargetWorkspaceOwnership::NotIndexed
            },
        }
    }

    /// Resolve a visible checkout, workspace, or member path through its
    /// exact canonical ownership record.
    pub(crate) fn workspace_for_visible_project(
        &self,
        project_path: &AbsolutePath,
    ) -> VisibleProjectWorkspaceOwnership<'_> {
        let project_evidence =
            match resolve_canonical_path(project_path, CanonicalVisibleProjectOwner) {
                CanonicalPathResolution::Resolved(project_owner) => {
                    exact_workspace_ownership_evidence(
                        &self.workspaces_by_owner,
                        project_owner.path(),
                    )
                },
                CanonicalPathResolution::Unresolved => ExactWorkspaceOwnershipEvidence::Unavailable,
            };
        self.visible_project_workspace_ownership(project_evidence)
    }

    /// Resolve an already-canonical checkout, workspace, or member path
    /// through its exact canonical ownership record. The path is not
    /// canonicalized again, so this performs no filesystem work and is safe to
    /// call from pure classification, which resolves every path it compares
    /// before the call.
    pub(crate) fn workspace_for_canonical_owner(
        &self,
        canonical_owner: &AbsolutePath,
    ) -> VisibleProjectWorkspaceOwnership<'_> {
        self.visible_project_workspace_ownership(exact_workspace_ownership_evidence(
            &self.workspaces_by_owner,
            canonical_owner,
        ))
    }

    /// Resolve a path only when it is itself an indexed Cargo checkout or
    /// workspace root. This proves a vendored package or submodule owns a
    /// nested workspace instead of merely belonging to its containing
    /// checkout.
    pub(crate) fn workspace_for_workspace_root(
        &self,
        workspace_root: &AbsolutePath,
    ) -> VisibleProjectWorkspaceOwnership<'_> {
        let workspace_evidence =
            match resolve_canonical_path(workspace_root, CanonicalVisibleProjectOwner) {
                CanonicalPathResolution::Resolved(canonical_workspace_root) => {
                    exact_workspace_ownership_evidence(
                        &self.workspaces_by_root,
                        canonical_workspace_root.path(),
                    )
                },
                CanonicalPathResolution::Unresolved => ExactWorkspaceOwnershipEvidence::Unavailable,
            };
        self.visible_project_workspace_ownership(workspace_evidence)
    }

    #[cfg(test)]
    pub(crate) const fn rebuild_count(&self) -> u64 { self.rebuild_count }

    pub(crate) const fn revision(&self) -> crate::project::CargoWorkspaceIndexRevisionState {
        self.revision
    }

    fn rebuild(
        &mut self,
        metadata_store: &WorkspaceMetadataStore,
        project_list_revision: ProjectListRevision,
    ) {
        let mut workspace_metadata: Vec<_> = metadata_store.accepted_metadata().collect();
        workspace_metadata.sort_by(|left, right| {
            left.declared_checkout_root
                .as_path()
                .cmp(right.declared_checkout_root.as_path())
        });

        self.workspaces.clear();
        self.workspaces_by_root.clear();
        self.workspaces_by_source.clear();
        self.workspaces_by_owner.clear();

        for metadata in workspace_metadata {
            let checkout_root_resolution =
                resolve_canonical_path(&metadata.declared_checkout_root, CanonicalCheckoutRoot);
            let workspace_root_resolution =
                resolve_canonical_path(&metadata.cargo_workspace_root, CanonicalWorkspaceRoot);
            let packages = cargo_package_identities(metadata);

            let index = self.workspaces.len();
            self.workspaces.push(CargoWorkspaceView {
                declared_checkout_root: metadata.declared_checkout_root.clone(),
                checkout_root_resolution,
                #[cfg(test)]
                declared_workspace_root: metadata.cargo_workspace_root.clone(),
                workspace_root_resolution,
                declared_target_directory: metadata.target_directory.clone(),
                packages,
            });
            self.index_workspace_ownership(index);
        }

        self.revision =
            CargoWorkspaceIndexRevisionState::Accepted(CargoWorkspaceIndexRevision::new(
                metadata_store.accepted_cargo_metadata_revision(),
                project_list_revision,
            ));
        #[cfg(test)]
        {
            self.rebuild_count = self.rebuild_count.saturating_add(1);
        }
    }

    fn index_workspace_ownership(&mut self, index: usize) {
        let workspace = &self.workspaces[index];
        if let CanonicalPathResolution::Resolved(checkout_root) =
            workspace.checkout_root_resolution()
        {
            include_workspace_candidate(
                &mut self.workspaces_by_root,
                checkout_root.path().clone(),
                index,
            );
            include_workspace_candidate(
                &mut self.workspaces_by_owner,
                checkout_root.path().clone(),
                index,
            );
        }
        if let CanonicalPathResolution::Resolved(workspace_root) =
            workspace.workspace_root_resolution()
        {
            include_workspace_candidate(
                &mut self.workspaces_by_root,
                workspace_root.path().clone(),
                index,
            );
            include_workspace_candidate(
                &mut self.workspaces_by_owner,
                workspace_root.path().clone(),
                index,
            );
        }
        for package in workspace.packages() {
            if let CanonicalPathResolution::Resolved(member_root) = package.member_root_resolution()
            {
                include_workspace_candidate(
                    &mut self.workspaces_by_owner,
                    member_root.path().clone(),
                    index,
                );
            }
            for target in package.targets() {
                if let CanonicalPathResolution::Resolved(source) =
                    target.canonical_source_resolution()
                {
                    include_workspace_candidate(
                        &mut self.workspaces_by_source,
                        source.path().clone(),
                        index,
                    );
                }
            }
        }
    }

    fn visible_project_workspace_ownership(
        &self,
        workspace_evidence: ExactWorkspaceOwnershipEvidence<'_>,
    ) -> VisibleProjectWorkspaceOwnership<'_> {
        match workspace_evidence {
            ExactWorkspaceOwnershipEvidence::Unique(workspace_index) => self
                .workspaces
                .get(workspace_index)
                .map_or(VisibleProjectWorkspaceOwnership::Ambiguous, |workspace| {
                    VisibleProjectWorkspaceOwnership::Indexed(workspace)
                }),
            ExactWorkspaceOwnershipEvidence::Ambiguous(_) => {
                VisibleProjectWorkspaceOwnership::Ambiguous
            },
            ExactWorkspaceOwnershipEvidence::Unavailable => {
                VisibleProjectWorkspaceOwnership::NotIndexed
            },
        }
    }
}

fn cargo_package_identities(metadata: &WorkspaceMetadata) -> Vec<CargoPackageIdentity> {
    let mut packages: Vec<_> = metadata.packages.iter().collect();
    packages.sort_by(|(_, left), (_, right)| {
        left.manifest_path
            .as_path()
            .cmp(right.manifest_path.as_path())
    });
    packages
        .into_iter()
        .map(|indexed_package| {
            let record = &indexed_package.1;
            let declared_member_root = manifest_directory(&record.manifest_path);
            let member_root_resolution =
                resolve_canonical_path(&declared_member_root, CanonicalMemberRoot);
            let targets = record
                .targets
                .iter()
                .map(|target| CargoTargetIdentity {
                    record:                      target.clone(),
                    canonical_source_resolution: resolve_canonical_path(
                        &target.src_path,
                        CanonicalTargetSource,
                    ),
                })
                .collect();
            CargoPackageIdentity {
                declared_member_root,
                member_root_resolution,
                package_id: indexed_package.0.clone(),
                targets,
            }
        })
        .collect()
}

fn exact_workspace_ownership_evidence<'a>(
    workspaces_by_path: &'a HashMap<AbsolutePath, CanonicalWorkspaceCandidates>,
    canonical_path: &AbsolutePath,
) -> ExactWorkspaceOwnershipEvidence<'a> {
    workspaces_by_path.get(canonical_path).map_or(
        ExactWorkspaceOwnershipEvidence::Unavailable,
        CanonicalWorkspaceCandidates::ownership_evidence,
    )
}

fn reconcile_visible_target_ownership(
    project_evidence: ExactWorkspaceOwnershipEvidence<'_>,
    source_evidence: ExactWorkspaceOwnershipEvidence<'_>,
) -> VisibleTargetOwnershipDecision {
    match (project_evidence, source_evidence) {
        (Unique(project_index), Unavailable) => {
            VisibleTargetOwnershipDecision::Indexed(project_index)
        },
        (Unique(project_index), Unique(source_index)) if project_index == source_index => {
            VisibleTargetOwnershipDecision::Indexed(project_index)
        },
        (Unique(project_index), Ambiguous(source_indices))
            if source_indices.contains(&project_index) =>
        {
            VisibleTargetOwnershipDecision::Indexed(project_index)
        },
        (Ambiguous(project_indices), Unique(source_index))
            if project_indices.contains(&source_index) =>
        {
            VisibleTargetOwnershipDecision::Indexed(source_index)
        },
        (Ambiguous(project_indices), Ambiguous(source_indices)) => {
            let shared_indices: Vec<_> = project_indices
                .iter()
                .copied()
                .filter(|workspace_index| source_indices.contains(workspace_index))
                .collect();
            match shared_indices.as_slice() {
                [workspace_index] => VisibleTargetOwnershipDecision::Indexed(*workspace_index),
                [] | [_, ..] => VisibleTargetOwnershipDecision::Ambiguous,
            }
        },
        (Unavailable, Unique(source_index)) => {
            VisibleTargetOwnershipDecision::Indexed(source_index)
        },
        (Unique(_), Unique(_) | Ambiguous(_))
        | (Ambiguous(_), Unavailable | Unique(_))
        | (Unavailable, Ambiguous(_)) => VisibleTargetOwnershipDecision::Ambiguous,
        (Unavailable, Unavailable) => VisibleTargetOwnershipDecision::NotIndexed,
    }
}

fn include_workspace_candidate(
    workspaces_by_path: &mut HashMap<AbsolutePath, CanonicalWorkspaceCandidates>,
    canonical_path: AbsolutePath,
    workspace_index: usize,
) {
    workspaces_by_path
        .entry(canonical_path)
        .or_default()
        .include(workspace_index);
}

fn resolve_canonical_path<T>(
    declared_path: &AbsolutePath,
    resolved: impl FnOnce(AbsolutePath) -> T,
) -> CanonicalPathResolution<T> {
    declared_path.as_path().canonicalize().map_or(
        CanonicalPathResolution::Unresolved,
        |canonical_path| {
            CanonicalPathResolution::Resolved(resolved(AbsolutePath::from(canonical_path)))
        },
    )
}

fn manifest_directory(manifest_path: &AbsolutePath) -> AbsolutePath {
    manifest_path
        .as_path()
        .parent()
        .map_or_else(|| manifest_path.clone(), AbsolutePath::from)
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should fail on unexpected index states")]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::HashMap;
    #[cfg(unix)]
    use std::error::Error;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::Path;
    use std::path::PathBuf;

    use cargo_metadata::PackageId;
    use cargo_metadata::TargetKind;
    use cargo_metadata::semver::Version;
    use tempfile::TempDir;

    use super::*;
    use crate::project::FileStamp;
    use crate::project::ManifestFingerprint;
    use crate::project::PublishPolicy;
    use crate::project::WorkspaceMetadata;

    struct SharedSourceWorkspaceFixture {
        _temp_dir:              TempDir,
        cargo_workspace_index:  CargoWorkspaceIndex,
        first_checkout_root:    PathBuf,
        second_checkout_root:   PathBuf,
        shared_source_path:     PathBuf,
        unrelated_project_root: PathBuf,
    }

    fn path(path: impl Into<PathBuf>) -> AbsolutePath { AbsolutePath::from(path.into()) }

    fn metadata(
        checkout_root: impl Into<PathBuf>,
        target_directory: impl Into<PathBuf>,
    ) -> WorkspaceMetadata {
        let checkout_root = path(checkout_root);
        WorkspaceMetadata {
            cargo_workspace_root:     checkout_root.clone(),
            declared_checkout_root:   checkout_root,
            target_directory:         path(target_directory),
            packages:                 HashMap::new(),
            fingerprint:              ManifestFingerprint {
                manifest:       FileStamp {
                    content_hash: [0_u8; 32],
                },
                lockfile:       None,
                rust_toolchain: None,
                configs:        BTreeMap::new(),
            },
            out_of_tree_target_bytes: None,
        }
    }

    fn package_record(member_root: impl AsRef<Path>) -> PackageRecord {
        let member_root = member_root.as_ref();
        package_record_with_source(member_root, &member_root.join("src/main.rs"))
    }

    fn package_record_with_source(member_root: &Path, source_path: &Path) -> PackageRecord {
        PackageRecord {
            name:          "member".to_string(),
            version:       Version::new(0, 1, 0),
            edition:       "2024".to_string(),
            description:   None,
            license:       None,
            homepage:      None,
            repository:    None,
            manifest_path: path(member_root.join("Cargo.toml")),
            targets:       vec![TargetRecord {
                name:              "member".to_string(),
                kinds:             vec![TargetKind::Bin],
                src_path:          path(source_path),
                required_features: Vec::new(),
            }],
            publish:       PublishPolicy::Any,
        }
    }

    fn shared_source_workspace_fixture()
    -> Result<SharedSourceWorkspaceFixture, Box<dyn std::error::Error>> {
        let temp_dir = tempfile::tempdir()?;
        let first_checkout_root = temp_dir.path().join("first-checkout");
        let second_checkout_root = temp_dir.path().join("second-checkout");
        let unrelated_project_root = temp_dir.path().join("unrelated-project");
        let shared_source_path = temp_dir.path().join("shared-source/src/main.rs");
        fs::create_dir_all(&first_checkout_root)?;
        fs::create_dir_all(&second_checkout_root)?;
        fs::create_dir_all(&unrelated_project_root)?;
        fs::create_dir_all(
            shared_source_path
                .parent()
                .ok_or("shared source parent should exist")?,
        )?;
        fs::write(&shared_source_path, "fn main() {}")?;

        let mut first_metadata = metadata(&first_checkout_root, first_checkout_root.join("target"));
        first_metadata.packages.insert(
            PackageId {
                repr: "first-member".to_string(),
            },
            package_record_with_source(&first_checkout_root, &shared_source_path),
        );
        let mut second_metadata =
            metadata(&second_checkout_root, second_checkout_root.join("target"));
        second_metadata.packages.insert(
            PackageId {
                repr: "second-member".to_string(),
            },
            package_record_with_source(&second_checkout_root, &shared_source_path),
        );
        let mut metadata_store = WorkspaceMetadataStore::new();
        metadata_store.upsert(first_metadata);
        metadata_store.upsert(second_metadata);
        let cargo_workspace_index = CargoWorkspaceIndex::from_metadata_store(
            &metadata_store,
            ProjectListRevision::default(),
        );

        Ok(SharedSourceWorkspaceFixture {
            _temp_dir: temp_dir,
            cargo_workspace_index,
            first_checkout_root,
            second_checkout_root,
            shared_source_path,
            unrelated_project_root,
        })
    }

    #[test]
    fn accepted_cargo_metadata_revision_rebuilds_the_index_once() {
        let mut metadata_store = WorkspaceMetadataStore::new();
        let project_list_revision = ProjectListRevision::default();
        let mut cargo_workspace_index = Arc::new(CargoWorkspaceIndex::from_metadata_store(
            &metadata_store,
            project_list_revision,
        ));
        let rebuild_count = cargo_workspace_index.rebuild_count();

        assert!(matches!(
            cargo_workspace_index.rebuild_if_changed(&metadata_store, project_list_revision),
            WorkspaceIndexRebuild::Unchanged
        ));
        assert_eq!(cargo_workspace_index.rebuild_count(), rebuild_count);

        metadata_store.upsert(metadata("/workspace", "/workspace/target"));

        let WorkspaceIndexRebuild::Replaced(rebuilt) =
            cargo_workspace_index.rebuild_if_changed(&metadata_store, project_list_revision)
        else {
            panic!("accepted metadata should replace the index");
        };
        cargo_workspace_index = rebuilt;
        assert_eq!(cargo_workspace_index.rebuild_count(), rebuild_count + 1);
        assert_eq!(cargo_workspace_index.workspaces().count(), 1);
    }

    #[test]
    fn project_list_revision_rebuilds_without_another_metadata_arrival() {
        let mut metadata_store = WorkspaceMetadataStore::new();
        metadata_store.upsert(metadata("/workspace", "/workspace/target"));
        let mut project_list_revision = ProjectListRevision::default();
        let mut cargo_workspace_index = Arc::new(CargoWorkspaceIndex::from_metadata_store(
            &metadata_store,
            project_list_revision,
        ));
        let rebuild_count = cargo_workspace_index.rebuild_count();

        project_list_revision.advance();
        let WorkspaceIndexRebuild::Replaced(rebuilt) =
            cargo_workspace_index.rebuild_if_changed(&metadata_store, project_list_revision)
        else {
            panic!("a project-list revision change should replace the index");
        };
        cargo_workspace_index = rebuilt;
        assert_eq!(cargo_workspace_index.rebuild_count(), rebuild_count + 1);
        assert_eq!(
            cargo_workspace_index.revision(),
            CargoWorkspaceIndexRevisionState::Accepted(CargoWorkspaceIndexRevision::new(
                metadata_store.accepted_cargo_metadata_revision(),
                project_list_revision
            ))
        );
    }

    #[test]
    fn canonical_checkout_and_member_roots_remain_exact_identities()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = tempfile::tempdir()?;
        let primary_root = temp_dir.path().join("repo");
        let feature_root = temp_dir.path().join("repo-feature");
        let member_root = primary_root.join("crates/member");
        let source_path = member_root.join("src/main.rs");
        fs::create_dir_all(source_path.parent().ok_or("source parent should exist")?)?;
        fs::create_dir_all(&feature_root)?;
        fs::write(&source_path, "fn main() {}")?;
        let mut metadata_store = WorkspaceMetadataStore::new();
        let mut primary = metadata(&primary_root, primary_root.join("target"));
        primary.packages.insert(
            PackageId {
                repr: "primary-member".to_string(),
            },
            package_record(&member_root),
        );
        metadata_store.upsert(primary);
        metadata_store.upsert(metadata(&feature_root, feature_root.join("target")));
        let cargo_workspace_index = CargoWorkspaceIndex::from_metadata_store(
            &metadata_store,
            ProjectListRevision::default(),
        );
        assert_eq!(cargo_workspace_index.workspaces().count(), 2);
        let mut workspaces = cargo_workspace_index.workspaces();
        let primary_workspace = workspaces.next().ok_or("primary workspace should exist")?;
        let feature_workspace = workspaces.next().ok_or("feature workspace should exist")?;

        assert!(matches!(
            primary_workspace.checkout_root_resolution(),
            CanonicalPathResolution::Resolved(root)
                if root.path().as_path() == primary_root.canonicalize()?
        ));
        assert!(matches!(
            primary_workspace.workspace_root_resolution(),
            CanonicalPathResolution::Resolved(root)
                if root.path().as_path() == primary_root.canonicalize()?
        ));
        assert!(matches!(
            feature_workspace.workspace_root_resolution(),
            CanonicalPathResolution::Resolved(root)
                if root.path().as_path() == feature_root.canonicalize()?
        ));
        assert!(matches!(
            cargo_workspace_index
                .workspace_for_visible_target(&path(&source_path), &path(&member_root)),
            VisibleTargetWorkspaceOwnership::Indexed(workspace)
                if workspace.declared_checkout_root_path().as_path() == primary_root.as_path()
        ));
        Ok(())
    }

    #[test]
    fn distinct_project_roots_resolve_a_shared_source_to_their_own_workspaces()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = shared_source_workspace_fixture()?;

        assert!(matches!(
            fixture.cargo_workspace_index.workspace_for_visible_target(
                &path(&fixture.shared_source_path),
                &path(&fixture.first_checkout_root),
            ),
            VisibleTargetWorkspaceOwnership::Indexed(workspace)
                if workspace.declared_checkout_root_path().as_path()
                    == fixture.first_checkout_root
        ));
        assert!(matches!(
            fixture.cargo_workspace_index.workspace_for_visible_target(
                &path(&fixture.shared_source_path),
                &path(&fixture.second_checkout_root),
            ),
            VisibleTargetWorkspaceOwnership::Indexed(workspace)
                if workspace.declared_checkout_root_path().as_path()
                    == fixture.second_checkout_root
        ));
        Ok(())
    }

    #[test]
    fn shared_source_without_resolving_project_evidence_is_ambiguous()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = shared_source_workspace_fixture()?;

        assert!(matches!(
            fixture.cargo_workspace_index.workspace_for_visible_target(
                &path(&fixture.shared_source_path),
                &path(&fixture.unrelated_project_root),
            ),
            VisibleTargetWorkspaceOwnership::Ambiguous
        ));
        Ok(())
    }

    #[test]
    fn unique_source_and_project_evidence_identifies_the_workspace()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = tempfile::tempdir()?;
        let checkout_root = temp_dir.path().join("checkout");
        let source_path = checkout_root.join("src/main.rs");
        fs::create_dir_all(source_path.parent().ok_or("source parent should exist")?)?;
        fs::write(&source_path, "fn main() {}")?;
        let mut workspace_metadata = metadata(&checkout_root, checkout_root.join("target"));
        workspace_metadata.packages.insert(
            PackageId {
                repr: "unique-member".to_string(),
            },
            package_record(&checkout_root),
        );
        let mut metadata_store = WorkspaceMetadataStore::new();
        metadata_store.upsert(workspace_metadata);
        let cargo_workspace_index = CargoWorkspaceIndex::from_metadata_store(
            &metadata_store,
            ProjectListRevision::default(),
        );

        assert!(matches!(
            cargo_workspace_index
                .workspace_for_visible_target(&path(&source_path), &path(&checkout_root)),
            VisibleTargetWorkspaceOwnership::Indexed(workspace)
                if workspace.declared_checkout_root_path().as_path() == checkout_root
        ));
        Ok(())
    }

    #[test]
    fn nested_cargo_workspace_keeps_distinct_canonical_roots()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = tempfile::tempdir()?;
        let checkout_root = temp_dir.path().join("checkout");
        let cargo_workspace_root = checkout_root.join("nested-workspace");
        std::fs::create_dir_all(&cargo_workspace_root)?;
        let mut workspace_metadata = metadata(&checkout_root, cargo_workspace_root.join("target"));
        workspace_metadata.cargo_workspace_root = path(&cargo_workspace_root);
        let mut metadata_store = WorkspaceMetadataStore::new();
        metadata_store.upsert(workspace_metadata);

        let cargo_workspace_index = CargoWorkspaceIndex::from_metadata_store(
            &metadata_store,
            ProjectListRevision::default(),
        );
        let workspace = cargo_workspace_index
            .workspaces()
            .next()
            .ok_or("indexed workspace should exist")?;

        assert!(matches!(
            workspace.checkout_root_resolution(),
            CanonicalPathResolution::Resolved(root)
                if root.path().as_path() == checkout_root.canonicalize()?
        ));
        assert!(matches!(
            workspace.workspace_root_resolution(),
            CanonicalPathResolution::Resolved(root)
                if root.path().as_path() == cargo_workspace_root.canonicalize()?
        ));
        assert_ne!(
            checkout_root.canonicalize()?,
            cargo_workspace_root.canonicalize()?
        );
        Ok(())
    }

    #[test]
    fn indexed_package_member_root_is_its_manifest_directory() {
        let member_root = path("/workspace/crates/member");
        let mut workspace_metadata = metadata("/workspace", "/workspace/target");
        workspace_metadata.packages.insert(
            PackageId {
                repr: "workspace-member".to_string(),
            },
            package_record(member_root.as_path()),
        );
        let mut metadata_store = WorkspaceMetadataStore::new();
        metadata_store.upsert(workspace_metadata);
        let cargo_workspace_index = CargoWorkspaceIndex::from_metadata_store(
            &metadata_store,
            ProjectListRevision::default(),
        );
        let indexed_member_roots: Vec<_> = cargo_workspace_index
            .workspaces()
            .flat_map(CargoWorkspaceView::packages)
            .map(CargoPackageIdentity::declared_member_root_path)
            .collect();

        assert_eq!(indexed_member_roots, vec![&member_root]);
    }

    #[test]
    fn exact_cargo_package_ids_remain_distinguishable() {
        let registry_package_id = PackageId {
            repr: "member 0.1.0 (registry+https://github.com/rust-lang/crates.io-index)"
                .to_string(),
        };
        let git_package_id = PackageId {
            repr: "member 0.1.0 (git+https://example.com/member?rev=abc#abc)".to_string(),
        };
        let mut workspace_metadata = metadata("/work/repo", "/work/repo/target");
        workspace_metadata.packages.insert(
            registry_package_id.clone(),
            package_record("/work/repo/crates/registry-member"),
        );
        workspace_metadata.packages.insert(
            git_package_id.clone(),
            package_record("/work/repo/crates/git-member"),
        );
        let mut metadata_store = WorkspaceMetadataStore::new();
        metadata_store.upsert(workspace_metadata);

        let cargo_workspace_index = CargoWorkspaceIndex::from_metadata_store(
            &metadata_store,
            ProjectListRevision::default(),
        );
        let cargo_package_ids: Vec<_> = cargo_workspace_index
            .workspaces()
            .flat_map(CargoWorkspaceView::packages)
            .map(CargoPackageIdentity::package_id)
            .collect();

        assert_eq!(cargo_package_ids.len(), 2);
        assert!(cargo_package_ids.contains(&&registry_package_id));
        assert!(cargo_package_ids.contains(&&git_package_id));
        assert_ne!(registry_package_id, git_package_id);
    }

    #[cfg(unix)]
    #[test]
    fn package_target_identity_canonicalizes_declared_symlink_source() -> Result<(), Box<dyn Error>>
    {
        let temp_dir = tempfile::tempdir()?;
        let real_workspace_root = temp_dir.path().join("real-workspace");
        let linked_workspace_root = temp_dir.path().join("linked-workspace");
        let real_source_path = real_workspace_root.join("src/main.rs");
        fs::create_dir_all(real_workspace_root.join("src"))?;
        fs::write(&real_source_path, "fn main() {}")?;
        symlink(&real_workspace_root, &linked_workspace_root)?;

        let mut workspace_metadata = metadata(
            real_workspace_root.as_path(),
            real_workspace_root.join("target"),
        );
        workspace_metadata.packages.insert(
            PackageId {
                repr: "member 0.1.0 (path+file:///real-workspace)".to_string(),
            },
            package_record(&linked_workspace_root),
        );
        let mut metadata_store = WorkspaceMetadataStore::new();
        metadata_store.upsert(workspace_metadata);
        let cargo_workspace_index = CargoWorkspaceIndex::from_metadata_store(
            &metadata_store,
            ProjectListRevision::default(),
        );
        let package = cargo_workspace_index
            .workspaces()
            .flat_map(CargoWorkspaceView::packages)
            .next()
            .ok_or("indexed package should exist")?;
        let target = package
            .targets()
            .next()
            .ok_or("indexed target should exist")?;

        assert_eq!(target.name(), "member");
        assert_eq!(target.kinds(), &[TargetKind::Bin]);
        assert!(target.required_features().is_empty());
        assert_eq!(
            target.declared_source_path().as_path(),
            linked_workspace_root.join("src/main.rs")
        );
        assert!(matches!(
            target.canonical_source_resolution(),
            CanonicalPathResolution::Resolved(source)
                if source.path().as_path() == real_source_path.canonicalize()?
        ));
        Ok(())
    }

    #[test]
    fn package_target_identity_exposes_unresolved_declared_source() {
        let declared_source = path("/missing/member/src/main.rs");
        let mut package_record = package_record("/missing/member");
        package_record.targets[0].src_path = declared_source.clone();
        let mut workspace_metadata = metadata("/missing", "/missing/target");
        workspace_metadata.packages.insert(
            PackageId {
                repr: "missing-member".to_string(),
            },
            package_record,
        );
        let mut metadata_store = WorkspaceMetadataStore::new();
        metadata_store.upsert(workspace_metadata);
        let cargo_workspace_index = CargoWorkspaceIndex::from_metadata_store(
            &metadata_store,
            ProjectListRevision::default(),
        );
        let target = cargo_workspace_index
            .workspaces()
            .flat_map(CargoWorkspaceView::packages)
            .flat_map(CargoPackageIdentity::targets)
            .next();

        assert!(matches!(
            target.map(CargoTargetIdentity::canonical_source_resolution),
            Some(CanonicalPathResolution::Unresolved)
        ));
        assert_eq!(
            target.map(CargoTargetIdentity::declared_source_path),
            Some(&declared_source)
        );
    }

    #[test]
    fn target_directory_resolution_observes_creation_without_index_rebuild()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = tempfile::tempdir()?;
        let workspace_root = temp_dir.path().join("workspace");
        let target_directory = temp_dir.path().join("shared-target");
        fs::create_dir_all(&workspace_root)?;
        let mut metadata_store = WorkspaceMetadataStore::new();
        metadata_store.upsert(metadata(&workspace_root, &target_directory));
        let cargo_workspace_index = CargoWorkspaceIndex::from_metadata_store(
            &metadata_store,
            ProjectListRevision::default(),
        );
        let workspace = cargo_workspace_index
            .workspaces()
            .next()
            .ok_or("indexed workspace should exist")?;

        assert!(matches!(
            workspace.target_directory_resolution(),
            CanonicalPathResolution::Unresolved
        ));
        assert_eq!(
            workspace.declared_target_directory_path().as_path(),
            target_directory
        );

        fs::create_dir_all(&target_directory)?;

        assert!(matches!(
            workspace.target_directory_resolution(),
            CanonicalPathResolution::Resolved(target)
                if target.path().as_path() == target_directory.canonicalize()?
        ));
        Ok(())
    }
}

/// Whether one canonical package member root identifies exactly one indexed
/// package.
///
/// The three states mirror [`VisibleProjectWorkspaceOwnership`]: a caller that
/// must not act on a guess can tell "no such package" apart from "more than one
/// package claims this root".
#[derive(Clone, Copy, Debug)]
pub(crate) enum CanonicalPackageOwnership<'a> {
    /// Exactly one indexed package has this canonical member root.
    Indexed(&'a CargoPackageIdentity),
    /// More than one indexed package claims this canonical member root.
    Ambiguous,
    /// No indexed package has this canonical member root.
    NotIndexed,
}

/// Whether one canonical target source file identifies exactly one indexed
/// package.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CanonicalTargetOwnership<'a> {
    /// Exactly one indexed package declares a target with this source file.
    Indexed(&'a CargoPackageIdentity),
    /// More than one indexed package declares a target with this source file.
    Ambiguous,
    /// No indexed package declares a target with this source file.
    NotIndexed,
}

/// Indexed packages identified by one exact canonical path.
#[derive(Debug, Default)]
struct CanonicalPackageCandidates<'a>(Vec<&'a CargoPackageIdentity>);

impl<'a> CanonicalPackageCandidates<'a> {
    fn include(&mut self, cargo_package_identity: &'a CargoPackageIdentity) {
        if !self
            .0
            .iter()
            .any(|candidate| std::ptr::eq(*candidate, cargo_package_identity))
        {
            self.0.push(cargo_package_identity);
        }
    }

    fn package_ownership(self) -> CanonicalPackageOwnership<'a> {
        match self.0.as_slice() {
            [] => CanonicalPackageOwnership::NotIndexed,
            [cargo_package_identity] => CanonicalPackageOwnership::Indexed(cargo_package_identity),
            [_, _, ..] => CanonicalPackageOwnership::Ambiguous,
        }
    }

    fn target_ownership(self) -> CanonicalTargetOwnership<'a> {
        match self.0.as_slice() {
            [] => CanonicalTargetOwnership::NotIndexed,
            [cargo_package_identity] => CanonicalTargetOwnership::Indexed(cargo_package_identity),
            [_, _, ..] => CanonicalTargetOwnership::Ambiguous,
        }
    }
}

impl CargoWorkspaceIndex {
    /// Resolve an already-canonical package root to its indexed package. The
    /// path is not canonicalized again, so this performs no filesystem work and
    /// is safe to call from pure classification.
    pub(crate) fn package_for_canonical_member_root(
        &self,
        canonical_member_root: &AbsolutePath,
    ) -> CanonicalPackageOwnership<'_> {
        let mut candidates = CanonicalPackageCandidates::default();
        for package in self
            .workspaces
            .iter()
            .flat_map(CargoWorkspaceView::packages)
        {
            if matches!(
                package.member_root_resolution(),
                CanonicalPathResolution::Resolved(member_root)
                    if member_root.path() == canonical_member_root
            ) {
                candidates.include(package);
            }
        }
        candidates.package_ownership()
    }

    /// Resolve an already-canonical target source file to the indexed package
    /// that declares it. Performs no filesystem work.
    pub(crate) fn package_for_canonical_target_source(
        &self,
        canonical_target_source: &AbsolutePath,
    ) -> CanonicalTargetOwnership<'_> {
        let mut candidates = CanonicalPackageCandidates::default();
        for package in self
            .workspaces
            .iter()
            .flat_map(CargoWorkspaceView::packages)
        {
            if package.targets().any(|target| {
                matches!(
                    target.canonical_source_resolution(),
                    CanonicalPathResolution::Resolved(target_source)
                        if target_source.path() == canonical_target_source
                )
            }) {
                candidates.include(package);
            }
        }
        candidates.target_ownership()
    }
}
