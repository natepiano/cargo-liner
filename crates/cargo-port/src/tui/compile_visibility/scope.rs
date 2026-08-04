use crate::build_monitor::CoveredScopeRoots;
use crate::build_monitor::LiveTargetDirectoryRevision;
use crate::build_monitor::ScopeRootCoverage;
use crate::project::AbsolutePath;
use crate::project::AcceptedCargoMetadataRevision;
use crate::project::CanonicalPathResolution;
use crate::project::CargoWorkspaceIndex;
use crate::project::CargoWorkspaceIndexRevision;
use crate::project::CargoWorkspaceIndexRevisionState;
use crate::project::CargoWorkspaceView;
use crate::project::ProjectListRevision;
use crate::project::RootCheckoutOwnership;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::VisibleProjectWorkspaceOwnership;
use crate::tui::project_list::CurrentVisibleRow;
use crate::tui::project_list::ProjectList;
use crate::tui::project_list::VisibleRow;
#[cfg(test)]
use crate::tui::project_list::VisibleRowKind;
use crate::tui::workspace_index::WorkspaceIndexReadiness;

/// Stable identity of one selected typed project-list row.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MonitorSelectedRowIdentity {
    visible_row: VisibleRow,
}

impl MonitorSelectedRowIdentity {
    pub(crate) const fn visible_row(&self) -> VisibleRow { self.visible_row }

    #[cfg(test)]
    pub(crate) const fn visible_row_kind(&self) -> VisibleRowKind { self.visible_row.kind() }
}

impl From<VisibleRow> for MonitorSelectedRowIdentity {
    fn from(visible_row: VisibleRow) -> Self { Self { visible_row } }
}

/// Selected-row state supplied to compile monitoring.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MonitorSelectedRow {
    Selected(MonitorSelectedRowIdentity),
    NoVisibleRow,
}

impl From<CurrentVisibleRow> for MonitorSelectedRow {
    fn from(current_visible_row: CurrentVisibleRow) -> Self {
        match current_visible_row {
            CurrentVisibleRow::Selected(visible_row) => {
                Self::Selected(MonitorSelectedRowIdentity::from(visible_row))
            },
            CurrentVisibleRow::NoVisibleRow => Self::NoVisibleRow,
        }
    }
}

/// Immutable roots and revisions that identify an actionable monitor scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MonitorScopeKey {
    monitor_selected_row_identity:    MonitorSelectedRowIdentity,
    covered_scope_roots:              CoveredScopeRoots,
    accepted_cargo_metadata_revision: AcceptedCargoMetadataRevision,
    project_list_revision:            ProjectListRevision,
    live_target_directory_revision:   LiveTargetDirectoryRevision,
}

impl MonitorScopeKey {
    const fn new(
        monitor_selected_row_identity: MonitorSelectedRowIdentity,
        covered_scope_roots: CoveredScopeRoots,
        accepted_cargo_metadata_revision: AcceptedCargoMetadataRevision,
        project_list_revision: ProjectListRevision,
        live_target_directory_revision: LiveTargetDirectoryRevision,
    ) -> Self {
        Self {
            monitor_selected_row_identity,
            covered_scope_roots,
            accepted_cargo_metadata_revision,
            project_list_revision,
            live_target_directory_revision,
        }
    }

    /// A key covering one checkout and workspace root, for tests that need an
    /// actionable resolution without resolving one from a live index.
    #[cfg(test)]
    pub(crate) fn for_test(canonical_root: AbsolutePath) -> Self {
        Self::new(
            MonitorSelectedRowIdentity::from(VisibleRow::Root { node_index: 0 }),
            crate::build_monitor::BuildScopeKey::for_test(canonical_root)
                .covered_scope_roots()
                .clone(),
            AcceptedCargoMetadataRevision::default(),
            ProjectListRevision::default(),
            LiveTargetDirectoryRevision::from(Vec::new()),
        )
    }

    /// The covered roots themselves, for the conversion that hands them to
    /// build classification without re-establishing the invariant.
    pub(crate) const fn covered_scope_roots(&self) -> &CoveredScopeRoots {
        &self.covered_scope_roots
    }

    /// The selected row this key was resolved for. `BuildScopeKey` carries no
    /// row identity, so only the scope tests read this.
    #[cfg(test)]
    pub(crate) const fn monitor_selected_row_identity(&self) -> &MonitorSelectedRowIdentity {
        &self.monitor_selected_row_identity
    }

    /// Which display row kind the selection resolved through. `BuildScopeKey`
    /// carries no row identity, so only the scope tests read this.
    #[cfg(test)]
    pub(crate) const fn selected_row_kind(&self) -> VisibleRowKind {
        self.monitor_selected_row_identity.visible_row_kind()
    }

    /// Accepted `cargo metadata` revision this scope was resolved against.
    pub(crate) const fn accepted_cargo_metadata_revision(&self) -> AcceptedCargoMetadataRevision {
        self.accepted_cargo_metadata_revision
    }

    /// Visible project-list revision this scope was resolved against.
    pub(crate) const fn project_list_revision(&self) -> ProjectListRevision {
        self.project_list_revision
    }

    /// The target-directory resolutions observed when this scope was resolved,
    /// so a directory that appears or a symlink that is retargeted makes the
    /// key — and everything classified under it — non-current.
    pub(crate) const fn live_target_directory_revision(&self) -> &LiveTargetDirectoryRevision {
        &self.live_target_directory_revision
    }
}

/// Index readiness recorded by a non-actionable monitor-scope resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MonitorWorkspaceIndexReadiness {
    Current(CargoWorkspaceIndexRevision),
    RetainedLastAccepted(CargoWorkspaceIndexRevision),
    Uninitialized,
}

/// Inputs that distinguish non-actionable scope results across selections and
/// accepted-index changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MonitorScopeResolutionRevision {
    project_list_revision:             ProjectListRevision,
    monitor_workspace_index_readiness: MonitorWorkspaceIndexReadiness,
}

impl MonitorScopeResolutionRevision {
    const fn new(
        project_list_revision: ProjectListRevision,
        monitor_workspace_index_readiness: MonitorWorkspaceIndexReadiness,
    ) -> Self {
        Self {
            project_list_revision,
            monitor_workspace_index_readiness,
        }
    }

    pub(crate) const fn project_list_revision(&self) -> ProjectListRevision {
        self.project_list_revision
    }

    /// A revision naming only the index readiness a message renders.
    #[cfg(test)]
    pub(crate) fn for_test(
        monitor_workspace_index_readiness: MonitorWorkspaceIndexReadiness,
    ) -> Self {
        Self::new(
            ProjectListRevision::default(),
            monitor_workspace_index_readiness,
        )
    }

    /// Which index readiness state produced this resolution, so a non-actionable
    /// scope reports why rather than only that it is unavailable.
    pub(crate) const fn monitor_workspace_index_readiness(&self) -> MonitorWorkspaceIndexReadiness {
        self.monitor_workspace_index_readiness
    }
}

/// Result of resolving the selected row into a compile-monitor scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MonitorScopeResolution {
    Ready(MonitorScopeKey),
    EmptyNonRust(MonitorScopeResolutionRevision),
    PendingIndex(MonitorScopeResolutionRevision),
    AmbiguousOwnership(MonitorScopeResolutionRevision),
    UnresolvedPath(MonitorScopeResolutionRevision),
}

/// Whether a resolved scope can authorize monitoring work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MonitorScopeActionability<'a> {
    Actionable(&'a MonitorScopeKey),
    NotActionable,
}

impl MonitorScopeResolution {
    pub(crate) const fn actionability(&self) -> MonitorScopeActionability<'_> {
        match self {
            Self::Ready(monitor_scope_key) => {
                MonitorScopeActionability::Actionable(monitor_scope_key)
            },
            Self::EmptyNonRust(_)
            | Self::PendingIndex(_)
            | Self::AmbiguousOwnership(_)
            | Self::UnresolvedPath(_) => MonitorScopeActionability::NotActionable,
        }
    }
}

/// One selected-row and scope-resolution replacement for active monitoring.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MonitorScopeUpdate {
    monitor_selected_row:     MonitorSelectedRow,
    monitor_scope_resolution: MonitorScopeResolution,
}

/// Selected-row data collected before consulting the App-owned workspace
/// index. Collecting this value performs no metadata or process refresh.
pub(crate) struct MonitorScopeInput {
    monitor_selected_row:  MonitorSelectedRow,
    monitor_scope_target:  MonitorScopeTarget,
    project_list_revision: ProjectListRevision,
}

impl MonitorScopeUpdate {
    pub(crate) const fn monitor_selected_row(&self) -> &MonitorSelectedRow {
        &self.monitor_selected_row
    }

    pub(crate) const fn monitor_scope_resolution(&self) -> &MonitorScopeResolution {
        &self.monitor_scope_resolution
    }

    pub(crate) fn into_parts(self) -> (MonitorSelectedRow, MonitorScopeResolution) {
        (self.monitor_selected_row, self.monitor_scope_resolution)
    }
}

enum MonitorScopeTarget {
    CheckoutRoots(Vec<AbsolutePath>),
    NestedWorkspaceOrContainingCheckout {
        nested_workspace_path:    AbsolutePath,
        containing_checkout_root: AbsolutePath,
    },
    EmptyNonRust,
    UnresolvedPath,
}

enum WorkspaceScopeResolution<'a> {
    Indexed(Vec<&'a CargoWorkspaceView>),
    AmbiguousOwnership,
    UnresolvedPath,
}

/// Resolve the selected project-list row without launching Cargo or refreshing
/// host process observation.
pub(crate) fn monitor_scope_input(project_list: &ProjectList) -> MonitorScopeInput {
    let monitor_selected_row = MonitorSelectedRow::from(project_list.current_visible_row());
    let monitor_scope_target = match &monitor_selected_row {
        MonitorSelectedRow::Selected(monitor_selected_row_identity) => {
            monitor_scope_target(project_list, monitor_selected_row_identity)
        },
        MonitorSelectedRow::NoVisibleRow => MonitorScopeTarget::UnresolvedPath,
    };
    MonitorScopeInput {
        monitor_selected_row,
        monitor_scope_target,
        project_list_revision: project_list.revision(),
    }
}

pub(crate) fn resolve_monitor_scope(
    monitor_scope_input: MonitorScopeInput,
    workspace_index_readiness: WorkspaceIndexReadiness<'_>,
) -> MonitorScopeUpdate {
    let monitor_scope_resolution_revision = MonitorScopeResolutionRevision::new(
        monitor_scope_input.project_list_revision,
        MonitorWorkspaceIndexReadiness::from(&workspace_index_readiness),
    );
    let monitor_scope_resolution = match &monitor_scope_input.monitor_selected_row {
        MonitorSelectedRow::Selected(monitor_selected_row_identity) => {
            resolve_monitor_scope_target(
                monitor_selected_row_identity,
                monitor_scope_input.monitor_scope_target,
                workspace_index_readiness,
                monitor_scope_resolution_revision,
            )
        },
        MonitorSelectedRow::NoVisibleRow => {
            MonitorScopeResolution::UnresolvedPath(monitor_scope_resolution_revision)
        },
    };
    MonitorScopeUpdate {
        monitor_selected_row: monitor_scope_input.monitor_selected_row,
        monitor_scope_resolution,
    }
}

impl From<&WorkspaceIndexReadiness<'_>> for MonitorWorkspaceIndexReadiness {
    fn from(workspace_index_readiness: &WorkspaceIndexReadiness<'_>) -> Self {
        match workspace_index_readiness {
            WorkspaceIndexReadiness::Current {
                cargo_workspace_index,
            } => match cargo_workspace_index.revision() {
                CargoWorkspaceIndexRevisionState::Accepted(cargo_workspace_index_revision) => {
                    Self::Current(cargo_workspace_index_revision)
                },
                CargoWorkspaceIndexRevisionState::Uninitialized => Self::Uninitialized,
            },
            WorkspaceIndexReadiness::RetainedLastAccepted {
                cargo_workspace_index,
            } => match cargo_workspace_index.revision() {
                CargoWorkspaceIndexRevisionState::Accepted(cargo_workspace_index_revision) => {
                    Self::RetainedLastAccepted(cargo_workspace_index_revision)
                },
                CargoWorkspaceIndexRevisionState::Uninitialized => Self::Uninitialized,
            },
            WorkspaceIndexReadiness::Uninitialized => Self::Uninitialized,
        }
    }
}

fn resolve_monitor_scope_target(
    monitor_selected_row_identity: &MonitorSelectedRowIdentity,
    monitor_scope_target: MonitorScopeTarget,
    workspace_index_readiness: WorkspaceIndexReadiness<'_>,
    monitor_scope_resolution_revision: MonitorScopeResolutionRevision,
) -> MonitorScopeResolution {
    match monitor_scope_target {
        MonitorScopeTarget::EmptyNonRust => {
            MonitorScopeResolution::EmptyNonRust(monitor_scope_resolution_revision)
        },
        MonitorScopeTarget::UnresolvedPath => {
            MonitorScopeResolution::UnresolvedPath(monitor_scope_resolution_revision)
        },
        MonitorScopeTarget::CheckoutRoots(checkout_roots) => resolve_checkout_roots_scope(
            monitor_selected_row_identity,
            &checkout_roots,
            workspace_index_readiness,
            monitor_scope_resolution_revision,
        ),
        MonitorScopeTarget::NestedWorkspaceOrContainingCheckout {
            nested_workspace_path,
            containing_checkout_root,
        } => resolve_nested_workspace_scope(
            monitor_selected_row_identity,
            &nested_workspace_path,
            containing_checkout_root,
            workspace_index_readiness,
            monitor_scope_resolution_revision,
        ),
    }
}

fn resolve_checkout_roots_scope(
    monitor_selected_row_identity: &MonitorSelectedRowIdentity,
    checkout_roots: &[AbsolutePath],
    workspace_index_readiness: WorkspaceIndexReadiness<'_>,
    monitor_scope_resolution_revision: MonitorScopeResolutionRevision,
) -> MonitorScopeResolution {
    let (WorkspaceIndexReadiness::Current {
        cargo_workspace_index,
    }
    | WorkspaceIndexReadiness::RetainedLastAccepted {
        cargo_workspace_index,
    }) = workspace_index_readiness
    else {
        return MonitorScopeResolution::PendingIndex(monitor_scope_resolution_revision);
    };
    match workspace_scope_for_checkout_roots(cargo_workspace_index, checkout_roots) {
        WorkspaceScopeResolution::Indexed(workspaces) => monitor_scope_key(
            monitor_selected_row_identity,
            workspaces,
            cargo_workspace_index,
            monitor_scope_resolution_revision,
        ),
        WorkspaceScopeResolution::AmbiguousOwnership => {
            MonitorScopeResolution::AmbiguousOwnership(monitor_scope_resolution_revision)
        },
        WorkspaceScopeResolution::UnresolvedPath => {
            MonitorScopeResolution::UnresolvedPath(monitor_scope_resolution_revision)
        },
    }
}

fn resolve_nested_workspace_scope(
    monitor_selected_row_identity: &MonitorSelectedRowIdentity,
    nested_workspace_path: &AbsolutePath,
    containing_checkout_root: AbsolutePath,
    workspace_index_readiness: WorkspaceIndexReadiness<'_>,
    monitor_scope_resolution_revision: MonitorScopeResolutionRevision,
) -> MonitorScopeResolution {
    let (WorkspaceIndexReadiness::Current {
        cargo_workspace_index,
    }
    | WorkspaceIndexReadiness::RetainedLastAccepted {
        cargo_workspace_index,
    }) = workspace_index_readiness
    else {
        return MonitorScopeResolution::PendingIndex(monitor_scope_resolution_revision);
    };
    match cargo_workspace_index.workspace_for_workspace_root(nested_workspace_path) {
        VisibleProjectWorkspaceOwnership::Indexed(workspace) => monitor_scope_key(
            monitor_selected_row_identity,
            vec![workspace],
            cargo_workspace_index,
            monitor_scope_resolution_revision,
        ),
        VisibleProjectWorkspaceOwnership::Ambiguous => {
            MonitorScopeResolution::AmbiguousOwnership(monitor_scope_resolution_revision)
        },
        VisibleProjectWorkspaceOwnership::NotIndexed => resolve_checkout_roots_scope(
            monitor_selected_row_identity,
            &[containing_checkout_root],
            workspace_index_readiness,
            monitor_scope_resolution_revision,
        ),
    }
}

fn workspace_scope_for_checkout_roots<'a>(
    cargo_workspace_index: &'a CargoWorkspaceIndex,
    checkout_roots: &[AbsolutePath],
) -> WorkspaceScopeResolution<'a> {
    if checkout_roots.is_empty() {
        return WorkspaceScopeResolution::UnresolvedPath;
    }
    let mut workspaces = Vec::new();
    for checkout_root in checkout_roots {
        match cargo_workspace_index.workspace_for_visible_project(checkout_root) {
            VisibleProjectWorkspaceOwnership::Indexed(workspace) => workspaces.push(workspace),
            VisibleProjectWorkspaceOwnership::Ambiguous => {
                return WorkspaceScopeResolution::AmbiguousOwnership;
            },
            VisibleProjectWorkspaceOwnership::NotIndexed => {
                return WorkspaceScopeResolution::UnresolvedPath;
            },
        }
    }
    WorkspaceScopeResolution::Indexed(workspaces)
}

fn monitor_scope_key(
    monitor_selected_row_identity: &MonitorSelectedRowIdentity,
    workspaces: Vec<&CargoWorkspaceView>,
    cargo_workspace_index: &CargoWorkspaceIndex,
    monitor_scope_resolution_revision: MonitorScopeResolutionRevision,
) -> MonitorScopeResolution {
    let CargoWorkspaceIndexRevisionState::Accepted(cargo_workspace_index_revision) =
        cargo_workspace_index.revision()
    else {
        return MonitorScopeResolution::PendingIndex(monitor_scope_resolution_revision);
    };
    let mut canonical_checkout_roots = Vec::new();
    let mut canonical_workspace_roots = Vec::new();
    let mut target_directory_resolutions = Vec::new();
    for workspace in workspaces {
        let CanonicalPathResolution::Resolved(canonical_checkout_root) =
            workspace.checkout_root_resolution()
        else {
            return MonitorScopeResolution::UnresolvedPath(monitor_scope_resolution_revision);
        };
        let CanonicalPathResolution::Resolved(canonical_workspace_root) =
            workspace.workspace_root_resolution()
        else {
            return MonitorScopeResolution::UnresolvedPath(monitor_scope_resolution_revision);
        };
        canonical_checkout_roots.push(canonical_checkout_root.clone());
        canonical_workspace_roots.push(canonical_workspace_root.clone());
        target_directory_resolutions.push(workspace.target_directory_resolution());
    }
    let ScopeRootCoverage::Covered(covered_scope_roots) =
        CoveredScopeRoots::from_roots(canonical_checkout_roots, canonical_workspace_roots)
    else {
        return MonitorScopeResolution::UnresolvedPath(monitor_scope_resolution_revision);
    };
    MonitorScopeResolution::Ready(MonitorScopeKey::new(
        monitor_selected_row_identity.clone(),
        covered_scope_roots,
        cargo_workspace_index_revision.accepted_cargo_metadata_revision(),
        monitor_scope_resolution_revision.project_list_revision(),
        LiveTargetDirectoryRevision::from(target_directory_resolutions),
    ))
}

fn monitor_scope_target(
    project_list: &ProjectList,
    monitor_selected_row_identity: &MonitorSelectedRowIdentity,
) -> MonitorScopeTarget {
    let visible_row = monitor_selected_row_identity.visible_row();
    let Some(project_entry) = project_list.get(visible_row.node_index()) else {
        return MonitorScopeTarget::UnresolvedPath;
    };
    let root_item = &project_entry.root_item;
    match visible_row {
        VisibleRow::Root { .. } => root_monitor_scope_target(root_item),
        VisibleRow::GroupHeader { .. } => workspace_checkout_scope_target(root_item),
        VisibleRow::Member {
            group_index,
            member_index,
            ..
        } => member_checkout_scope_target(root_item, group_index, member_index),
        VisibleRow::MemberVendored {
            group_index,
            member_index,
            vendored_index,
            ..
        } => member_vendored_scope_target(root_item, group_index, member_index, vendored_index),
        VisibleRow::Vendored { vendored_index, .. } => {
            vendored_scope_target(root_item, vendored_index)
        },
        VisibleRow::WorktreeEntry { worktree_index, .. }
        | VisibleRow::WorktreeGroupHeader { worktree_index, .. } => {
            worktree_checkout_scope_target(root_item, worktree_index)
        },
        VisibleRow::WorktreeMember {
            worktree_index,
            group_index,
            member_index,
            ..
        } => worktree_member_scope_target(root_item, worktree_index, group_index, member_index),
        VisibleRow::WorktreeMemberVendored {
            worktree_index,
            group_index,
            member_index,
            vendored_index,
            ..
        } => worktree_member_vendored_scope_target(
            root_item,
            worktree_index,
            group_index,
            member_index,
            vendored_index,
        ),
        VisibleRow::WorktreeVendored {
            worktree_index,
            vendored_index,
            ..
        } => worktree_vendored_scope_target(root_item, worktree_index, vendored_index),
        VisibleRow::Submodule {
            submodule_index, ..
        } => submodule_scope_target(root_item, submodule_index),
    }
}

fn root_monitor_scope_target(root_item: &RootItem) -> MonitorScopeTarget {
    match root_item.checkout_ownership() {
        RootCheckoutOwnership::NonRust => MonitorScopeTarget::EmptyNonRust,
        RootCheckoutOwnership::RustCheckout(rust_project) => {
            MonitorScopeTarget::CheckoutRoots(vec![rust_project.path().clone()])
        },
        RootCheckoutOwnership::WorktreeGroup(worktree_group)
            if worktree_group.renders_as_group() =>
        {
            let checkout_roots: Vec<_> = worktree_group
                .represented_visible_checkout_roots()
                .cloned()
                .collect();
            if checkout_roots.is_empty() {
                MonitorScopeTarget::UnresolvedPath
            } else {
                MonitorScopeTarget::CheckoutRoots(checkout_roots)
            }
        },
        RootCheckoutOwnership::WorktreeGroup(worktree_group) => worktree_group
            .single_live()
            .map_or(MonitorScopeTarget::UnresolvedPath, |rust_project| {
                MonitorScopeTarget::CheckoutRoots(vec![rust_project.path().clone()])
            }),
    }
}

fn workspace_checkout_scope_target(root_item: &RootItem) -> MonitorScopeTarget {
    match root_item {
        RootItem::Rust(RustProject::Workspace(workspace)) => {
            MonitorScopeTarget::CheckoutRoots(vec![workspace.path.clone()])
        },
        RootItem::Rust(RustProject::Package(_)) => MonitorScopeTarget::UnresolvedPath,
        RootItem::NonRust(_) => MonitorScopeTarget::EmptyNonRust,
        RootItem::Worktrees(worktree_group) => worktree_group
            .single_live_workspace()
            .map_or(MonitorScopeTarget::UnresolvedPath, |workspace| {
                MonitorScopeTarget::CheckoutRoots(vec![workspace.path.clone()])
            }),
    }
}

fn member_checkout_scope_target(
    root_item: &RootItem,
    group_index: usize,
    member_index: usize,
) -> MonitorScopeTarget {
    match root_item {
        RootItem::Rust(RustProject::Workspace(workspace)) => workspace
            .groups()
            .get(group_index)
            .and_then(|member_group| member_group.members().get(member_index))
            .map_or(MonitorScopeTarget::UnresolvedPath, |_| {
                MonitorScopeTarget::CheckoutRoots(vec![workspace.path.clone()])
            }),
        RootItem::Rust(RustProject::Package(_)) => MonitorScopeTarget::UnresolvedPath,
        RootItem::NonRust(_) => MonitorScopeTarget::EmptyNonRust,
        RootItem::Worktrees(worktree_group) => worktree_group.single_live_workspace().map_or(
            MonitorScopeTarget::UnresolvedPath,
            |workspace| {
                workspace
                    .groups()
                    .get(group_index)
                    .and_then(|member_group| member_group.members().get(member_index))
                    .map_or(MonitorScopeTarget::UnresolvedPath, |_| {
                        MonitorScopeTarget::CheckoutRoots(vec![workspace.path.clone()])
                    })
            },
        ),
    }
}

fn member_vendored_scope_target(
    root_item: &RootItem,
    group_index: usize,
    member_index: usize,
    vendored_index: usize,
) -> MonitorScopeTarget {
    match root_item {
        RootItem::Rust(RustProject::Workspace(workspace)) => workspace
            .groups()
            .get(group_index)
            .and_then(|member_group| member_group.members().get(member_index))
            .and_then(|member| member.vendored().get(vendored_index))
            .map_or(MonitorScopeTarget::UnresolvedPath, |vendored_package| {
                nested_workspace_scope_target(vendored_package.path.clone(), workspace.path.clone())
            }),
        RootItem::Rust(RustProject::Package(_)) => MonitorScopeTarget::UnresolvedPath,
        RootItem::NonRust(_) => MonitorScopeTarget::EmptyNonRust,
        RootItem::Worktrees(worktree_group) => worktree_group.single_live_workspace().map_or(
            MonitorScopeTarget::UnresolvedPath,
            |workspace| {
                workspace
                    .groups()
                    .get(group_index)
                    .and_then(|member_group| member_group.members().get(member_index))
                    .and_then(|member| member.vendored().get(vendored_index))
                    .map_or(MonitorScopeTarget::UnresolvedPath, |vendored_package| {
                        nested_workspace_scope_target(
                            vendored_package.path.clone(),
                            workspace.path.clone(),
                        )
                    })
            },
        ),
    }
}

fn vendored_scope_target(root_item: &RootItem, vendored_index: usize) -> MonitorScopeTarget {
    match root_item {
        RootItem::Rust(RustProject::Workspace(workspace)) => workspace
            .vendored()
            .get(vendored_index)
            .map_or(MonitorScopeTarget::UnresolvedPath, |vendored_package| {
                nested_workspace_scope_target(vendored_package.path.clone(), workspace.path.clone())
            }),
        RootItem::Rust(RustProject::Package(package)) => package
            .vendored()
            .get(vendored_index)
            .map_or(MonitorScopeTarget::UnresolvedPath, |vendored_package| {
                nested_workspace_scope_target(vendored_package.path.clone(), package.path.clone())
            }),
        RootItem::NonRust(_) => MonitorScopeTarget::EmptyNonRust,
        RootItem::Worktrees(worktree_group) => worktree_group.single_live().map_or(
            MonitorScopeTarget::UnresolvedPath,
            |rust_project| {
                rust_project
                    .rust_info()
                    .vendored()
                    .get(vendored_index)
                    .map_or(MonitorScopeTarget::UnresolvedPath, |vendored_package| {
                        nested_workspace_scope_target(
                            vendored_package.path.clone(),
                            rust_project.path().clone(),
                        )
                    })
            },
        ),
    }
}

fn worktree_checkout_scope_target(
    root_item: &RootItem,
    worktree_index: usize,
) -> MonitorScopeTarget {
    match root_item {
        RootItem::Rust(_) | RootItem::NonRust(_) => MonitorScopeTarget::UnresolvedPath,
        RootItem::Worktrees(worktree_group) => worktree_group
            .entry(worktree_index)
            .map_or(MonitorScopeTarget::UnresolvedPath, |rust_project| {
                MonitorScopeTarget::CheckoutRoots(vec![rust_project.path().clone()])
            }),
    }
}

fn worktree_member_scope_target(
    root_item: &RootItem,
    worktree_index: usize,
    group_index: usize,
    member_index: usize,
) -> MonitorScopeTarget {
    match root_item {
        RootItem::Rust(_) | RootItem::NonRust(_) => MonitorScopeTarget::UnresolvedPath,
        RootItem::Worktrees(worktree_group) => worktree_group
            .member_ref(worktree_index, group_index, member_index)
            .and_then(|_| worktree_group.entry(worktree_index))
            .map_or(MonitorScopeTarget::UnresolvedPath, |rust_project| {
                MonitorScopeTarget::CheckoutRoots(vec![rust_project.path().clone()])
            }),
    }
}

fn worktree_member_vendored_scope_target(
    root_item: &RootItem,
    worktree_index: usize,
    group_index: usize,
    member_index: usize,
    vendored_index: usize,
) -> MonitorScopeTarget {
    match root_item {
        RootItem::Rust(_) | RootItem::NonRust(_) => MonitorScopeTarget::UnresolvedPath,
        RootItem::Worktrees(worktree_group) => worktree_group
            .member_vendored_ref(worktree_index, group_index, member_index, vendored_index)
            .and_then(|vendored_package| {
                worktree_group.entry(worktree_index).map(|rust_project| {
                    nested_workspace_scope_target(
                        vendored_package.path.clone(),
                        rust_project.path().clone(),
                    )
                })
            })
            .unwrap_or(MonitorScopeTarget::UnresolvedPath),
    }
}

fn worktree_vendored_scope_target(
    root_item: &RootItem,
    worktree_index: usize,
    vendored_index: usize,
) -> MonitorScopeTarget {
    match root_item {
        RootItem::Rust(_) | RootItem::NonRust(_) => MonitorScopeTarget::UnresolvedPath,
        RootItem::Worktrees(worktree_group) => worktree_group
            .vendored_ref(worktree_index, vendored_index)
            .and_then(|vendored_package| {
                worktree_group.entry(worktree_index).map(|rust_project| {
                    nested_workspace_scope_target(
                        vendored_package.path.clone(),
                        rust_project.path().clone(),
                    )
                })
            })
            .unwrap_or(MonitorScopeTarget::UnresolvedPath),
    }
}

fn submodule_scope_target(root_item: &RootItem, submodule_index: usize) -> MonitorScopeTarget {
    match root_item {
        RootItem::Rust(RustProject::Workspace(workspace)) => workspace
            .project_info
            .submodules
            .get(submodule_index)
            .map_or(MonitorScopeTarget::UnresolvedPath, |submodule| {
                nested_workspace_scope_target(submodule.path.clone(), workspace.path.clone())
            }),
        RootItem::Rust(RustProject::Package(package)) => package
            .project_info
            .submodules
            .get(submodule_index)
            .map_or(MonitorScopeTarget::UnresolvedPath, |submodule| {
                nested_workspace_scope_target(submodule.path.clone(), package.path.clone())
            }),
        RootItem::NonRust(_) => MonitorScopeTarget::EmptyNonRust,
        RootItem::Worktrees(worktree_group) => worktree_group
            .primary
            .rust_info()
            .project_info
            .submodules
            .get(submodule_index)
            .map_or(MonitorScopeTarget::UnresolvedPath, |submodule| {
                nested_workspace_scope_target(
                    submodule.path.clone(),
                    worktree_group.primary_path().clone(),
                )
            }),
    }
}

const fn nested_workspace_scope_target(
    nested_workspace_path: AbsolutePath,
    containing_checkout_root: AbsolutePath,
) -> MonitorScopeTarget {
    MonitorScopeTarget::NestedWorkspaceOrContainingCheckout {
        nested_workspace_path,
        containing_checkout_root,
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::HashMap;
    use std::collections::HashSet;
    use std::error::Error;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use std::time::Instant;

    use super::*;
    use crate::build_monitor::BuildScopeActionability;
    use crate::build_monitor::BuildScopeKey;
    use crate::process_observation::CompileMonitorRefreshSchedule;
    use crate::project::FileStamp;
    use crate::project::ManifestFingerprint;
    use crate::project::MemberGroup;
    use crate::project::NonRustProject;
    use crate::project::RustInfo;
    use crate::project::Submodule;
    use crate::project::VendoredPackage;
    use crate::project::Workspace;
    use crate::project::WorkspaceMetadata;
    use crate::project::WorkspaceMetadataStore;
    use crate::project::WorktreeGroup;
    use crate::tui::compile_visibility::CompileMonitorGeneration;
    use crate::tui::compile_visibility::CompileVisibilityState;
    use crate::tui::compile_visibility::constants::COMPILE_MONITOR_REFRESH_INTERVAL;

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    const ALL_VISIBLE_ROW_KINDS: [VisibleRowKind; 11] = [
        VisibleRowKind::Root,
        VisibleRowKind::GroupHeader,
        VisibleRowKind::Member,
        VisibleRowKind::MemberVendored,
        VisibleRowKind::Vendored,
        VisibleRowKind::WorktreeEntry,
        VisibleRowKind::WorktreeGroupHeader,
        VisibleRowKind::WorktreeMember,
        VisibleRowKind::WorktreeMemberVendored,
        VisibleRowKind::WorktreeVendored,
        VisibleRowKind::Submodule,
    ];

    /// Directory name for a metadata-proven nested Cargo workspace root
    /// inside a fixture vendored/submodule checkout, mirroring a checkout
    /// whose Cargo workspace lives in a subdirectory.
    const NESTED_WORKSPACE_DIR: &str = "nested-workspace";

    /// Exact canonical roots expected from one fixture workspace scope.
    struct FixtureScopeRoots<'a> {
        canonical_checkout_roots:  Vec<&'a AbsolutePath>,
        canonical_workspace_roots: Vec<&'a AbsolutePath>,
    }

    /// Expected actionability and roots for one fixture row.
    enum FixtureScopeExpectation<'a> {
        Ready(FixtureScopeRoots<'a>),
        EmptyNonRust,
    }

    /// Whether a fixture row must exercise nested-workspace fallback.
    enum NestedWorkspaceFallbackExpectation<'a> {
        NotNestedWorkspaceRow,
        ContainingCheckout(&'a AbsolutePath),
    }

    struct FixtureCheckoutRoots {
        standard: AbsolutePath,
        primary:  AbsolutePath,
        linked:   AbsolutePath,
    }

    /// One vendored or submodule fixture checkout whose accepted metadata
    /// proves a nested Cargo workspace root distinct from the checkout root.
    struct FixtureNestedWorkspace {
        checkout_root:  AbsolutePath,
        workspace_root: AbsolutePath,
    }

    struct FixtureNestedWorkspaceRoots {
        standard_member_vendored: FixtureNestedWorkspace,
        standard_vendored:        FixtureNestedWorkspace,
        standard_submodule:       FixtureNestedWorkspace,
        primary_member_vendored:  FixtureNestedWorkspace,
        primary_vendored:         FixtureNestedWorkspace,
        primary_submodule:        FixtureNestedWorkspace,
        linked_member_vendored:   FixtureNestedWorkspace,
        linked_vendored:          FixtureNestedWorkspace,
    }

    struct ScopeFixture {
        temp_dir:                            tempfile::TempDir,
        cargo_workspace_index:               Arc<CargoWorkspaceIndex>,
        containing_checkout_workspace_index: Arc<CargoWorkspaceIndex>,
        checkout_roots:                      FixtureCheckoutRoots,
        nested_workspace_roots:              FixtureNestedWorkspaceRoots,
        project_list:                        ProjectList,
    }

    impl ScopeFixture {
        fn metadata_proven_scope_expectation(
            &self,
            visible_row: VisibleRow,
        ) -> FixtureScopeExpectation<'_> {
            match visible_row {
                VisibleRow::Root { node_index: 0 } => {
                    FixtureScopeExpectation::Ready(fixture_scope_roots(vec![
                        &self.checkout_roots.standard,
                    ]))
                },
                VisibleRow::Root { node_index: 1 } => {
                    FixtureScopeExpectation::Ready(fixture_scope_roots(vec![
                        &self.checkout_roots.primary,
                        &self.checkout_roots.linked,
                    ]))
                },
                VisibleRow::Root { node_index: 2 } => FixtureScopeExpectation::EmptyNonRust,
                VisibleRow::Root { node_index } => {
                    panic!("fixture has unexpected root row {node_index}")
                },
                VisibleRow::GroupHeader {
                    node_index: 0,
                    group_index: 0,
                }
                | VisibleRow::Member {
                    node_index: 0,
                    group_index: 0,
                    member_index: 0,
                } => FixtureScopeExpectation::Ready(fixture_scope_roots(vec![
                    &self.checkout_roots.standard,
                ])),
                VisibleRow::GroupHeader { .. } => {
                    panic!("fixture has unexpected group header row {visible_row:?}")
                },
                VisibleRow::Member { .. } => {
                    panic!("fixture has unexpected member row {visible_row:?}")
                },
                VisibleRow::MemberVendored {
                    node_index: 0,
                    group_index: 0,
                    member_index: 0,
                    vendored_index: 0,
                } => FixtureScopeExpectation::Ready(nested_scope_roots(
                    &self.nested_workspace_roots.standard_member_vendored,
                )),
                VisibleRow::MemberVendored { .. } => {
                    panic!("fixture has unexpected member vendored row {visible_row:?}")
                },
                VisibleRow::Vendored {
                    node_index: 0,
                    vendored_index: 0,
                } => FixtureScopeExpectation::Ready(nested_scope_roots(
                    &self.nested_workspace_roots.standard_vendored,
                )),
                VisibleRow::Vendored { .. } => {
                    panic!("fixture has unexpected vendored row {visible_row:?}")
                },
                VisibleRow::WorktreeEntry { .. }
                | VisibleRow::WorktreeGroupHeader { .. }
                | VisibleRow::WorktreeMember { .. }
                | VisibleRow::WorktreeMemberVendored { .. }
                | VisibleRow::WorktreeVendored { .. } => {
                    self.worktree_metadata_proven_scope_expectation(visible_row)
                },
                VisibleRow::Submodule {
                    node_index: 0,
                    submodule_index: 0,
                } => FixtureScopeExpectation::Ready(nested_scope_roots(
                    &self.nested_workspace_roots.standard_submodule,
                )),
                VisibleRow::Submodule {
                    node_index: 1,
                    submodule_index: 0,
                } => FixtureScopeExpectation::Ready(nested_scope_roots(
                    &self.nested_workspace_roots.primary_submodule,
                )),
                VisibleRow::Submodule { .. } => {
                    panic!("fixture has unexpected submodule row {visible_row:?}")
                },
            }
        }

        fn worktree_metadata_proven_scope_expectation(
            &self,
            visible_row: VisibleRow,
        ) -> FixtureScopeExpectation<'_> {
            match visible_row {
                VisibleRow::WorktreeEntry {
                    node_index: 1,
                    worktree_index: 0,
                }
                | VisibleRow::WorktreeGroupHeader {
                    node_index: 1,
                    worktree_index: 0,
                    group_index: 0,
                }
                | VisibleRow::WorktreeMember {
                    node_index: 1,
                    worktree_index: 0,
                    group_index: 0,
                    member_index: 0,
                } => FixtureScopeExpectation::Ready(fixture_scope_roots(vec![
                    &self.checkout_roots.primary,
                ])),
                VisibleRow::WorktreeEntry {
                    node_index: 1,
                    worktree_index: 1,
                }
                | VisibleRow::WorktreeGroupHeader {
                    node_index: 1,
                    worktree_index: 1,
                    group_index: 0,
                }
                | VisibleRow::WorktreeMember {
                    node_index: 1,
                    worktree_index: 1,
                    group_index: 0,
                    member_index: 0,
                } => FixtureScopeExpectation::Ready(fixture_scope_roots(vec![
                    &self.checkout_roots.linked,
                ])),
                VisibleRow::WorktreeEntry { .. } => {
                    panic!("fixture has unexpected worktree entry row {visible_row:?}")
                },
                VisibleRow::WorktreeGroupHeader { .. } => {
                    panic!("fixture has unexpected worktree group header row {visible_row:?}")
                },
                VisibleRow::WorktreeMember { .. } => {
                    panic!("fixture has unexpected worktree member row {visible_row:?}")
                },
                VisibleRow::WorktreeMemberVendored {
                    node_index: 1,
                    worktree_index: 0,
                    group_index: 0,
                    member_index: 0,
                    vendored_index: 0,
                } => FixtureScopeExpectation::Ready(nested_scope_roots(
                    &self.nested_workspace_roots.primary_member_vendored,
                )),
                VisibleRow::WorktreeMemberVendored {
                    node_index: 1,
                    worktree_index: 1,
                    group_index: 0,
                    member_index: 0,
                    vendored_index: 0,
                } => FixtureScopeExpectation::Ready(nested_scope_roots(
                    &self.nested_workspace_roots.linked_member_vendored,
                )),
                VisibleRow::WorktreeMemberVendored { .. } => {
                    panic!("fixture has unexpected worktree member vendored row {visible_row:?}")
                },
                VisibleRow::WorktreeVendored {
                    node_index: 1,
                    worktree_index: 0,
                    vendored_index: 0,
                } => FixtureScopeExpectation::Ready(nested_scope_roots(
                    &self.nested_workspace_roots.primary_vendored,
                )),
                VisibleRow::WorktreeVendored {
                    node_index: 1,
                    worktree_index: 1,
                    vendored_index: 0,
                } => FixtureScopeExpectation::Ready(nested_scope_roots(
                    &self.nested_workspace_roots.linked_vendored,
                )),
                VisibleRow::WorktreeVendored { .. } => {
                    panic!("fixture has unexpected worktree vendored row {visible_row:?}")
                },
                VisibleRow::Root { .. }
                | VisibleRow::GroupHeader { .. }
                | VisibleRow::Member { .. }
                | VisibleRow::MemberVendored { .. }
                | VisibleRow::Vendored { .. }
                | VisibleRow::Submodule { .. } => {
                    panic!("fixture row {visible_row:?} is not a worktree row")
                },
            }
        }

        fn nested_workspace_fallback_expectation(
            &self,
            visible_row: VisibleRow,
        ) -> NestedWorkspaceFallbackExpectation<'_> {
            match visible_row {
                VisibleRow::Root { .. }
                | VisibleRow::GroupHeader { .. }
                | VisibleRow::Member { .. }
                | VisibleRow::WorktreeEntry { .. }
                | VisibleRow::WorktreeGroupHeader { .. }
                | VisibleRow::WorktreeMember { .. } => {
                    NestedWorkspaceFallbackExpectation::NotNestedWorkspaceRow
                },
                VisibleRow::MemberVendored {
                    node_index: 0,
                    group_index: 0,
                    member_index: 0,
                    vendored_index: 0,
                }
                | VisibleRow::Vendored {
                    node_index: 0,
                    vendored_index: 0,
                }
                | VisibleRow::Submodule {
                    node_index: 0,
                    submodule_index: 0,
                } => NestedWorkspaceFallbackExpectation::ContainingCheckout(
                    &self.checkout_roots.standard,
                ),
                VisibleRow::WorktreeMemberVendored {
                    node_index: 1,
                    worktree_index: 0,
                    group_index: 0,
                    member_index: 0,
                    vendored_index: 0,
                }
                | VisibleRow::WorktreeVendored {
                    node_index: 1,
                    worktree_index: 0,
                    vendored_index: 0,
                }
                | VisibleRow::Submodule {
                    node_index: 1,
                    submodule_index: 0,
                } => NestedWorkspaceFallbackExpectation::ContainingCheckout(
                    &self.checkout_roots.primary,
                ),
                VisibleRow::WorktreeMemberVendored {
                    node_index: 1,
                    worktree_index: 1,
                    group_index: 0,
                    member_index: 0,
                    vendored_index: 0,
                }
                | VisibleRow::WorktreeVendored {
                    node_index: 1,
                    worktree_index: 1,
                    vendored_index: 0,
                } => NestedWorkspaceFallbackExpectation::ContainingCheckout(
                    &self.checkout_roots.linked,
                ),
                VisibleRow::MemberVendored { .. } => {
                    panic!("fixture has unexpected member vendored row {visible_row:?}")
                },
                VisibleRow::Vendored { .. } => {
                    panic!("fixture has unexpected vendored row {visible_row:?}")
                },
                VisibleRow::WorktreeMemberVendored { .. } => {
                    panic!("fixture has unexpected worktree member vendored row {visible_row:?}")
                },
                VisibleRow::WorktreeVendored { .. } => {
                    panic!("fixture has unexpected worktree vendored row {visible_row:?}")
                },
                VisibleRow::Submodule { .. } => {
                    panic!("fixture has unexpected submodule row {visible_row:?}")
                },
            }
        }
    }

    #[test]
    fn every_visible_row_kind_resolves_to_its_exact_canonical_roots_or_empty_scope() -> TestResult {
        let mut scope_fixture = scope_fixture()?;
        scope_fixture.project_list.expand_all(true);
        scope_fixture.project_list.recompute_visibility(true);
        let rows = scope_fixture.project_list.visible_rows().to_vec();
        let expected_row_kinds: HashSet<_> = ALL_VISIBLE_ROW_KINDS.into_iter().collect();
        let observed_row_kinds: HashSet<_> =
            rows.iter().map(|visible_row| visible_row.kind()).collect();
        assert_eq!(observed_row_kinds, expected_row_kinds);

        for (index, visible_row) in rows.into_iter().enumerate() {
            scope_fixture.project_list.set_cursor(index);
            let monitor_scope_update = resolve_monitor_scope(
                monitor_scope_input(&scope_fixture.project_list),
                WorkspaceIndexReadiness::Current {
                    cargo_workspace_index: &scope_fixture.cargo_workspace_index,
                },
            );
            assert_monitor_scope_matches_fixture_expectation(
                &monitor_scope_update,
                visible_row,
                scope_fixture.metadata_proven_scope_expectation(visible_row),
            );
        }
        Ok(())
    }

    #[test]
    fn vendored_and_submodule_rows_fall_back_to_containing_checkout_without_nested_workspace_metadata()
    -> TestResult {
        let mut scope_fixture = scope_fixture()?;
        scope_fixture.project_list.expand_all(true);
        scope_fixture.project_list.recompute_visibility(true);
        let rows = scope_fixture.project_list.visible_rows().to_vec();

        for (index, visible_row) in rows.into_iter().enumerate() {
            scope_fixture.project_list.set_cursor(index);
            let nested_workspace_fallback_expectation =
                scope_fixture.nested_workspace_fallback_expectation(visible_row);
            if let NestedWorkspaceFallbackExpectation::ContainingCheckout(containing_checkout) =
                nested_workspace_fallback_expectation
            {
                let monitor_scope_update = resolve_monitor_scope(
                    monitor_scope_input(&scope_fixture.project_list),
                    WorkspaceIndexReadiness::Current {
                        cargo_workspace_index: &scope_fixture.containing_checkout_workspace_index,
                    },
                );
                assert_monitor_scope_matches_fixture_expectation(
                    &monitor_scope_update,
                    visible_row,
                    FixtureScopeExpectation::Ready(fixture_scope_roots(vec![containing_checkout])),
                );
            }
        }
        Ok(())
    }

    #[test]
    fn group_root_and_individual_checkout_keep_distinct_scope_keys() -> TestResult {
        let mut scope_fixture = scope_fixture()?;
        scope_fixture.project_list.expand_all(true);
        scope_fixture.project_list.recompute_visibility(true);
        let group_scope_key = ready_scope_key_for_row(
            &mut scope_fixture.project_list,
            &scope_fixture.cargo_workspace_index,
            |visible_row| matches!(visible_row, VisibleRow::Root { node_index: 1 }),
        )?;
        let checkout_scope_key = ready_scope_key_for_row(
            &mut scope_fixture.project_list,
            &scope_fixture.cargo_workspace_index,
            |visible_row| {
                matches!(
                    visible_row,
                    VisibleRow::WorktreeEntry {
                        node_index:     1,
                        worktree_index: 0,
                    }
                )
            },
        )?;

        assert_ne!(
            group_scope_key.monitor_selected_row_identity(),
            checkout_scope_key.monitor_selected_row_identity()
        );
        assert_ne!(group_scope_key, checkout_scope_key);
        assert_eq!(
            canonical_checkout_root_paths(&group_scope_key),
            sorted_absolute_paths(vec![
                &scope_fixture.checkout_roots.primary,
                &scope_fixture.checkout_roots.linked,
            ])
        );
        assert_eq!(
            canonical_checkout_root_paths(&checkout_scope_key),
            vec![scope_fixture.checkout_roots.primary.clone()]
        );
        Ok(())
    }

    #[test]
    fn identical_roots_from_two_rows_replace_the_active_generation() -> TestResult {
        let mut scope_fixture = scope_fixture()?;
        scope_fixture.project_list.expand_all(true);
        scope_fixture.project_list.recompute_visibility(true);
        let first_monitor_scope_update = monitor_scope_update_for_row(
            &mut scope_fixture.project_list,
            &scope_fixture.cargo_workspace_index,
            |visible_row| matches!(visible_row, VisibleRow::Root { node_index: 0 }),
        )?;
        let second_monitor_scope_update = monitor_scope_update_for_row(
            &mut scope_fixture.project_list,
            &scope_fixture.cargo_workspace_index,
            |visible_row| matches!(visible_row, VisibleRow::GroupHeader { node_index: 0, .. }),
        )?;
        let project_list_revision = scope_fixture.project_list.revision();
        let mut compile_monitor_generation = CompileMonitorGeneration::default();
        compile_monitor_generation.advance();
        let first_generation = compile_monitor_generation;
        let mut compile_visibility_state = CompileVisibilityState::Off;
        assert!(matches!(
            &compile_visibility_state,
            CompileVisibilityState::Off
        ));

        // The two rows are the scenario this test is named for only while they
        // still resolve to the same roots; without this the generation
        // assertions below would keep passing after a regression separated them.
        let (
            MonitorScopeResolution::Ready(first_monitor_scope_key),
            MonitorScopeResolution::Ready(second_monitor_scope_key),
        ) = (
            first_monitor_scope_update.monitor_scope_resolution(),
            second_monitor_scope_update.monitor_scope_resolution(),
        )
        else {
            panic!("both rows must resolve to actionable scopes");
        };
        assert_eq!(
            first_monitor_scope_key
                .covered_scope_roots()
                .canonical_checkout_roots(),
            second_monitor_scope_key
                .covered_scope_roots()
                .canonical_checkout_roots()
        );
        assert_eq!(
            first_monitor_scope_key
                .covered_scope_roots()
                .canonical_workspace_roots(),
            second_monitor_scope_key
                .covered_scope_roots()
                .canonical_workspace_roots()
        );
        assert_ne!(
            first_monitor_scope_key.monitor_selected_row_identity(),
            second_monitor_scope_key.monitor_selected_row_identity()
        );

        compile_visibility_state.enable(
            first_monitor_scope_update,
            first_generation,
            Instant::now(),
        );

        assert!(compile_visibility_state.requires_scope_replacement(&second_monitor_scope_update));
        assert_eq!(scope_fixture.project_list.revision(), project_list_revision);
        compile_monitor_generation.advance();
        let second_generation = compile_monitor_generation;
        compile_visibility_state.replace_scope(
            second_monitor_scope_update,
            second_generation,
            Instant::now(),
        );

        assert!(!compile_visibility_state.accepts_generation(first_generation));
        assert!(compile_visibility_state.accepts_generation(second_generation));
        compile_visibility_state.disable();
        assert!(!compile_visibility_state.accepts_generation(second_generation));
        compile_monitor_generation.advance();
        let third_generation = compile_monitor_generation;
        let replacement_monitor_scope_update = monitor_scope_update_for_row(
            &mut scope_fixture.project_list,
            &scope_fixture.cargo_workspace_index,
            |visible_row| matches!(visible_row, VisibleRow::Root { node_index: 0 }),
        )?;
        compile_visibility_state.enable(
            replacement_monitor_scope_update,
            third_generation,
            Instant::now(),
        );
        assert!(!compile_visibility_state.accepts_generation(second_generation));
        assert!(compile_visibility_state.accepts_generation(third_generation));
        Ok(())
    }

    #[test]
    fn pending_ambiguous_and_unresolved_index_results_stay_non_actionable() -> TestResult {
        let mut scope_fixture = scope_fixture()?;
        scope_fixture.project_list.recompute_visibility(true);
        scope_fixture.project_list.set_cursor(0);
        let selected_monitor_scope_input = monitor_scope_input(&scope_fixture.project_list);
        let pending_monitor_scope_update = resolve_monitor_scope(
            selected_monitor_scope_input,
            WorkspaceIndexReadiness::Uninitialized,
        );
        assert!(matches!(
            pending_monitor_scope_update.monitor_scope_resolution(),
            MonitorScopeResolution::PendingIndex(_)
        ));

        let retained_monitor_scope_update = resolve_monitor_scope(
            monitor_scope_input(&scope_fixture.project_list),
            WorkspaceIndexReadiness::RetainedLastAccepted {
                cargo_workspace_index: &scope_fixture.cargo_workspace_index,
            },
        );
        assert!(matches!(
            retained_monitor_scope_update.monitor_scope_resolution(),
            MonitorScopeResolution::Ready(_)
        ));

        let empty_workspace_metadata_store = WorkspaceMetadataStore::new();
        let empty_cargo_workspace_index = Arc::new(CargoWorkspaceIndex::from_metadata_store(
            &empty_workspace_metadata_store,
            scope_fixture.project_list.revision(),
        ));
        let unresolved_monitor_scope_update = resolve_monitor_scope(
            monitor_scope_input(&scope_fixture.project_list),
            WorkspaceIndexReadiness::Current {
                cargo_workspace_index: &empty_cargo_workspace_index,
            },
        );
        assert!(matches!(
            unresolved_monitor_scope_update.monitor_scope_resolution(),
            MonitorScopeResolution::UnresolvedPath(_)
        ));

        let ambiguous_cargo_workspace_index =
            Arc::new(ambiguous_cargo_workspace_index(&scope_fixture)?);
        let ambiguous_monitor_scope_update = resolve_monitor_scope(
            monitor_scope_input(&scope_fixture.project_list),
            WorkspaceIndexReadiness::Current {
                cargo_workspace_index: &ambiguous_cargo_workspace_index,
            },
        );
        assert!(matches!(
            ambiguous_monitor_scope_update.monitor_scope_resolution(),
            MonitorScopeResolution::AmbiguousOwnership(_)
        ));
        Ok(())
    }

    fn scope_fixture() -> TestResult<ScopeFixture> {
        let temp_dir = tempfile::tempdir()?;
        // Canonicalize the fixture base so expected roots match the
        // canonical resolutions stored in `MonitorScopeKey` (macOS `/tmp`
        // and `/var` are symlinks into `/private`).
        let temp_root = temp_dir.path().canonicalize()?;
        let standard_root = fixture_dir(&temp_root, "standard")?;
        let standard_member = fixture_dir(&standard_root, "member")?;
        let standard_member_vendored = fixture_dir(&standard_member, "member-vendored")?;
        let standard_vendored = fixture_dir(&standard_root, "vendored")?;
        let standard_submodule = fixture_dir(&standard_root, "submodule")?;
        let primary_root = fixture_dir(&temp_root, "primary")?;
        let primary_member = fixture_dir(&primary_root, "member")?;
        let primary_member_vendored = fixture_dir(&primary_member, "member-vendored")?;
        let primary_vendored = fixture_dir(&primary_root, "vendored")?;
        let primary_submodule = fixture_dir(&primary_root, "submodule")?;
        let linked_root = fixture_dir(&temp_root, "linked")?;
        let linked_member = fixture_dir(&linked_root, "member")?;
        let linked_member_vendored = fixture_dir(&linked_member, "member-vendored")?;
        let linked_vendored = fixture_dir(&linked_root, "vendored")?;
        let non_rust_root = fixture_dir(&temp_root, "non-rust")?;
        let checkout_roots = FixtureCheckoutRoots {
            standard: AbsolutePath::from(standard_root.as_path()),
            primary:  AbsolutePath::from(primary_root.as_path()),
            linked:   AbsolutePath::from(linked_root.as_path()),
        };
        let nested_workspace_roots = FixtureNestedWorkspaceRoots {
            standard_member_vendored: fixture_nested_workspace(&standard_member_vendored)?,
            standard_vendored:        fixture_nested_workspace(&standard_vendored)?,
            standard_submodule:       fixture_nested_workspace(&standard_submodule)?,
            primary_member_vendored:  fixture_nested_workspace(&primary_member_vendored)?,
            primary_vendored:         fixture_nested_workspace(&primary_vendored)?,
            primary_submodule:        fixture_nested_workspace(&primary_submodule)?,
            linked_member_vendored:   fixture_nested_workspace(&linked_member_vendored)?,
            linked_vendored:          fixture_nested_workspace(&linked_vendored)?,
        };

        let standard_workspace = workspace(
            &standard_root,
            &standard_member,
            &standard_member_vendored,
            &standard_vendored,
            Some(&standard_submodule),
        );
        let primary_workspace = workspace(
            &primary_root,
            &primary_member,
            &primary_member_vendored,
            &primary_vendored,
            Some(&primary_submodule),
        );
        let linked_workspace = workspace(
            &linked_root,
            &linked_member,
            &linked_member_vendored,
            &linked_vendored,
            None,
        );
        let project_list = ProjectList::new(vec![
            RootItem::Rust(RustProject::Workspace(standard_workspace)),
            RootItem::Worktrees(WorktreeGroup::new(
                RustProject::Workspace(primary_workspace),
                vec![RustProject::Workspace(linked_workspace)],
            )),
            RootItem::NonRust(NonRustProject::new(AbsolutePath::from(non_rust_root), None)),
        ]);
        let cargo_workspace_index = Arc::new(nested_workspace_index(
            &project_list,
            &checkout_roots,
            &nested_workspace_roots,
        ));
        let containing_checkout_workspace_index =
            Arc::new(containing_checkout_index(&project_list, &checkout_roots));
        Ok(ScopeFixture {
            temp_dir,
            cargo_workspace_index,
            containing_checkout_workspace_index,
            checkout_roots,
            nested_workspace_roots,
            project_list,
        })
    }

    fn workspace(
        root: &Path,
        member_root: &Path,
        member_vendored_root: &Path,
        vendored_root: &Path,
        submodule_root: Option<&Path>,
    ) -> Workspace {
        let mut project_info = crate::project::ProjectInfo::default();
        if let Some(submodule_root) = submodule_root {
            project_info.submodules.push(Submodule {
                name:          "submodule".to_string(),
                path:          AbsolutePath::from(submodule_root),
                relative_path: "submodule".to_string(),
                url:           None,
                branch:        None,
                commit:        None,
                project_info:  crate::project::ProjectInfo::default(),
                git_repo:      None,
            });
        }
        Workspace {
            path: AbsolutePath::from(root),
            rust: RustInfo {
                project_info,
                vendored: vec![vendored_package(vendored_root)],
                ..RustInfo::default()
            },
            groups: vec![MemberGroup::Named {
                name:    "members".to_string(),
                members: vec![crate::project::Package {
                    path: AbsolutePath::from(member_root),
                    rust: RustInfo {
                        vendored: vec![vendored_package(member_vendored_root)],
                        ..RustInfo::default()
                    },
                    ..crate::project::Package::default()
                }],
            }],
            ..Workspace::default()
        }
    }

    fn vendored_package(root: &Path) -> VendoredPackage {
        VendoredPackage {
            path: AbsolutePath::from(root),
            ..VendoredPackage::default()
        }
    }

    fn fixture_nested_workspace(checkout_root: &Path) -> TestResult<FixtureNestedWorkspace> {
        let workspace_root = checkout_root.join(NESTED_WORKSPACE_DIR);
        fs::create_dir_all(&workspace_root)?;
        Ok(FixtureNestedWorkspace {
            checkout_root:  AbsolutePath::from(checkout_root),
            workspace_root: AbsolutePath::from(workspace_root.as_path()),
        })
    }

    fn nested_workspace_metadata(nested_workspace: &FixtureNestedWorkspace) -> WorkspaceMetadata {
        workspace_metadata(
            nested_workspace.checkout_root.as_path(),
            nested_workspace.workspace_root.as_path(),
        )
    }

    /// Create one fixture directory under `parent` and return its path.
    fn fixture_dir(parent: &Path, name: &str) -> TestResult<PathBuf> {
        let path = parent.join(name);
        fs::create_dir_all(&path)?;
        Ok(path)
    }

    /// Index whose accepted metadata proves a nested Cargo workspace for every
    /// fixture vendored package and submodule.
    fn nested_workspace_index(
        project_list: &ProjectList,
        checkout_roots: &FixtureCheckoutRoots,
        nested_workspace_roots: &FixtureNestedWorkspaceRoots,
    ) -> CargoWorkspaceIndex {
        workspace_index_for_metadata(
            project_list,
            vec![
                checkout_root_metadata(&checkout_roots.standard),
                nested_workspace_metadata(&nested_workspace_roots.standard_member_vendored),
                nested_workspace_metadata(&nested_workspace_roots.standard_vendored),
                nested_workspace_metadata(&nested_workspace_roots.standard_submodule),
                checkout_root_metadata(&checkout_roots.primary),
                nested_workspace_metadata(&nested_workspace_roots.primary_member_vendored),
                nested_workspace_metadata(&nested_workspace_roots.primary_vendored),
                nested_workspace_metadata(&nested_workspace_roots.primary_submodule),
                checkout_root_metadata(&checkout_roots.linked),
                nested_workspace_metadata(&nested_workspace_roots.linked_member_vendored),
                nested_workspace_metadata(&nested_workspace_roots.linked_vendored),
            ],
        )
    }

    /// Index whose accepted metadata covers only the fixture checkout roots, so
    /// vendored packages and submodules fall back to their containing checkout.
    fn containing_checkout_index(
        project_list: &ProjectList,
        checkout_roots: &FixtureCheckoutRoots,
    ) -> CargoWorkspaceIndex {
        workspace_index_for_metadata(
            project_list,
            vec![
                checkout_root_metadata(&checkout_roots.standard),
                checkout_root_metadata(&checkout_roots.primary),
                checkout_root_metadata(&checkout_roots.linked),
            ],
        )
    }

    /// Metadata for a checkout whose Cargo workspace root is the checkout root.
    fn checkout_root_metadata(checkout_root: &AbsolutePath) -> WorkspaceMetadata {
        workspace_metadata(checkout_root.as_path(), checkout_root.as_path())
    }

    fn workspace_metadata(
        declared_checkout_root: &Path,
        cargo_workspace_root: &Path,
    ) -> WorkspaceMetadata {
        WorkspaceMetadata {
            declared_checkout_root:   AbsolutePath::from(declared_checkout_root),
            cargo_workspace_root:     AbsolutePath::from(cargo_workspace_root),
            target_directory:         AbsolutePath::from(cargo_workspace_root.join("target")),
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

    fn workspace_index_for_metadata(
        project_list: &ProjectList,
        workspace_metadata_entries: Vec<WorkspaceMetadata>,
    ) -> CargoWorkspaceIndex {
        let mut workspace_metadata_store = WorkspaceMetadataStore::new();
        for workspace_metadata_entry in workspace_metadata_entries {
            workspace_metadata_store.upsert(workspace_metadata_entry);
        }
        CargoWorkspaceIndex::from_metadata_store(&workspace_metadata_store, project_list.revision())
    }

    fn fixture_scope_roots(canonical_roots: Vec<&AbsolutePath>) -> FixtureScopeRoots<'_> {
        FixtureScopeRoots {
            canonical_checkout_roots:  canonical_roots.clone(),
            canonical_workspace_roots: canonical_roots,
        }
    }

    fn nested_scope_roots(nested_workspace: &FixtureNestedWorkspace) -> FixtureScopeRoots<'_> {
        FixtureScopeRoots {
            canonical_checkout_roots:  vec![&nested_workspace.checkout_root],
            canonical_workspace_roots: vec![&nested_workspace.workspace_root],
        }
    }

    fn assert_monitor_scope_matches_fixture_expectation(
        monitor_scope_update: &MonitorScopeUpdate,
        visible_row: VisibleRow,
        fixture_scope_expectation: FixtureScopeExpectation<'_>,
    ) {
        match monitor_scope_update.monitor_selected_row() {
            MonitorSelectedRow::Selected(monitor_selected_row_identity) => {
                assert_eq!(monitor_selected_row_identity.visible_row(), visible_row);
            },
            MonitorSelectedRow::NoVisibleRow => panic!("fixture row should remain selected"),
        }
        match fixture_scope_expectation {
            FixtureScopeExpectation::Ready(fixture_scope_roots) => {
                let MonitorScopeResolution::Ready(monitor_scope_key) =
                    monitor_scope_update.monitor_scope_resolution()
                else {
                    panic!("fixture Rust row should resolve to an actionable monitor scope");
                };
                let canonical_checkout_roots: Vec<_> = monitor_scope_key
                    .covered_scope_roots()
                    .canonical_checkout_roots()
                    .iter()
                    .map(|canonical_checkout_root| canonical_checkout_root.path().clone())
                    .collect();
                let canonical_workspace_roots: Vec<_> = monitor_scope_key
                    .covered_scope_roots()
                    .canonical_workspace_roots()
                    .iter()
                    .map(|canonical_workspace_root| canonical_workspace_root.path().clone())
                    .collect();
                assert_eq!(
                    canonical_checkout_roots,
                    sorted_absolute_paths(fixture_scope_roots.canonical_checkout_roots),
                );
                assert_eq!(
                    canonical_workspace_roots,
                    sorted_absolute_paths(fixture_scope_roots.canonical_workspace_roots),
                );
                assert_eq!(monitor_scope_key.selected_row_kind(), visible_row.kind());
            },
            FixtureScopeExpectation::EmptyNonRust => assert!(matches!(
                monitor_scope_update.monitor_scope_resolution(),
                MonitorScopeResolution::EmptyNonRust(_)
            )),
        }
    }

    fn canonical_checkout_root_paths(monitor_scope_key: &MonitorScopeKey) -> Vec<AbsolutePath> {
        monitor_scope_key
            .covered_scope_roots()
            .canonical_checkout_roots()
            .iter()
            .map(|canonical_checkout_root| canonical_checkout_root.path().clone())
            .collect()
    }

    fn sorted_absolute_paths(absolute_paths: Vec<&AbsolutePath>) -> Vec<AbsolutePath> {
        let mut absolute_paths: Vec<_> = absolute_paths.into_iter().cloned().collect();
        absolute_paths.sort_by(|left, right| left.as_path().cmp(right.as_path()));
        absolute_paths
    }

    #[test]
    fn an_actionable_resolution_yields_a_build_scope_key_without_the_row_identity() -> TestResult {
        let mut scope_fixture = scope_fixture()?;
        scope_fixture.project_list.expand_all(true);
        scope_fixture.project_list.recompute_visibility(true);
        let monitor_scope_key = ready_scope_key_for_row(
            &mut scope_fixture.project_list,
            &scope_fixture.cargo_workspace_index,
            |visible_row| matches!(visible_row, VisibleRow::Root { node_index: 1 }),
        )?;
        let monitor_scope_resolution = MonitorScopeResolution::Ready(monitor_scope_key.clone());

        let BuildScopeActionability::Actionable(build_scope_key) =
            crate::tui::compile_visibility::build_scope_actionability(&monitor_scope_resolution)
        else {
            return Err("a ready resolution is actionable".into());
        };
        assert_eq!(
            build_scope_key.canonical_checkout_roots(),
            monitor_scope_key
                .covered_scope_roots()
                .canonical_checkout_roots(),
            "the conversion must not re-sort the roots"
        );
        assert_eq!(
            build_scope_key.canonical_workspace_roots(),
            monitor_scope_key
                .covered_scope_roots()
                .canonical_workspace_roots()
        );
        assert_eq!(
            build_scope_key.accepted_cargo_metadata_revision(),
            monitor_scope_key.accepted_cargo_metadata_revision()
        );
        assert_eq!(
            build_scope_key.project_list_revision(),
            monitor_scope_key.project_list_revision()
        );
        assert_eq!(
            build_scope_key,
            BuildScopeKey::from(&monitor_scope_key),
            "two conversions of the same scope produce equal keys"
        );
        let uninitialized_revision = MonitorScopeResolutionRevision::new(
            ProjectListRevision::default(),
            MonitorWorkspaceIndexReadiness::Uninitialized,
        );
        assert_eq!(
            uninitialized_revision.monitor_workspace_index_readiness(),
            MonitorWorkspaceIndexReadiness::Uninitialized
        );
        assert!(matches!(
            crate::tui::compile_visibility::build_scope_actionability(
                &MonitorScopeResolution::EmptyNonRust(uninitialized_revision)
            ),
            BuildScopeActionability::NotActionable
        ));
        Ok(())
    }

    #[test]
    fn two_rows_over_the_same_roots_produce_one_build_scope_key() -> TestResult {
        let mut scope_fixture = scope_fixture()?;
        scope_fixture.project_list.expand_all(true);
        scope_fixture.project_list.recompute_visibility(true);
        // A root row and a member row inside it resolve to the same checkout,
        // so the build monitor must see one scope rather than restart on every
        // move of the selection inside a checkout.
        let root_scope_key = ready_scope_key_for_row(
            &mut scope_fixture.project_list,
            &scope_fixture.cargo_workspace_index,
            |visible_row| matches!(visible_row, VisibleRow::Root { node_index: 0 }),
        )?;
        let member_scope_key = ready_scope_key_for_row(
            &mut scope_fixture.project_list,
            &scope_fixture.cargo_workspace_index,
            |visible_row| {
                matches!(
                    visible_row,
                    VisibleRow::Member {
                        node_index:   0,
                        group_index:  0,
                        member_index: 0,
                    }
                )
            },
        )?;

        assert_ne!(
            root_scope_key.monitor_selected_row_identity(),
            member_scope_key.monitor_selected_row_identity(),
            "the two rows are different selections"
        );
        assert_ne!(
            root_scope_key, member_scope_key,
            "the monitor scope key still distinguishes the selected row"
        );
        assert_eq!(
            BuildScopeKey::from(&root_scope_key),
            BuildScopeKey::from(&member_scope_key),
            "the build scope key drops the row identity, so both rows build the same scope"
        );
        Ok(())
    }

    /// The declared target directory is resolved on every scope resolve, so a
    /// directory that was missing and now exists — or a `target` symlink created
    /// and then retargeted at a different directory — makes the scope key, and
    /// everything classified under it, non-current. Accepted `cargo metadata`,
    /// project-list content, and the selected row are all held constant here, so
    /// the live target-directory resolution is the only input that moved.
    #[test]
    fn a_target_directory_that_appears_or_is_retargeted_invalidates_the_scope() -> TestResult<()> {
        let mut scope_fixture = scope_fixture()?;
        scope_fixture.project_list.recompute_visibility(true);
        let checkout_root = scope_fixture
            .project_list
            .get(0)
            .ok_or("standard root should exist")?
            .root_item
            .path()
            .as_path()
            .to_path_buf();
        let target_directory = checkout_root.join("target");
        let retargeted_directory = scope_fixture.temp_dir.path().join("retargeted-target");
        fs::create_dir_all(&retargeted_directory)?;
        let root_row =
            |visible_row: VisibleRow| matches!(visible_row, VisibleRow::Root { node_index: 0 });

        let missing = monitor_scope_update_for_row(
            &mut scope_fixture.project_list,
            &scope_fixture.cargo_workspace_index,
            root_row,
        )?;
        fs::create_dir_all(&target_directory)?;
        let present = monitor_scope_update_for_row(
            &mut scope_fixture.project_list,
            &scope_fixture.cargo_workspace_index,
            root_row,
        )?;
        fs::remove_dir(&target_directory)?;
        std::os::unix::fs::symlink(&retargeted_directory, &target_directory)?;
        let retargeted = monitor_scope_update_for_row(
            &mut scope_fixture.project_list,
            &scope_fixture.cargo_workspace_index,
            root_row,
        )?;

        let missing_scope_key = ready_scope_key(&missing)?;
        let present_scope_key = ready_scope_key(&present)?;
        let retargeted_scope_key = ready_scope_key(&retargeted)?;
        assert_eq!(
            missing_scope_key.monitor_selected_row_identity(),
            present_scope_key.monitor_selected_row_identity(),
            "the same row stays selected across all three resolves"
        );
        assert_eq!(
            missing_scope_key.accepted_cargo_metadata_revision(),
            present_scope_key.accepted_cargo_metadata_revision(),
            "no metadata was accepted between the two resolves"
        );
        assert_eq!(
            missing_scope_key.project_list_revision(),
            present_scope_key.project_list_revision(),
            "no visible ownership changed between the two resolves"
        );
        assert_ne!(
            missing_scope_key.live_target_directory_revision(),
            present_scope_key.live_target_directory_revision(),
            "the target directory appearing is a new live resolution"
        );
        assert_ne!(
            BuildScopeKey::from(missing_scope_key),
            BuildScopeKey::from(present_scope_key),
            "classification stamped before the target directory existed is not current"
        );
        assert_ne!(
            present_scope_key.live_target_directory_revision(),
            retargeted_scope_key.live_target_directory_revision(),
            "a symlink pointing the target directory elsewhere is a new live resolution"
        );

        let mut compile_visibility_state = CompileVisibilityState::default();
        compile_visibility_state.enable(
            missing,
            CompileMonitorGeneration::default(),
            Instant::now(),
        );

        assert!(
            compile_visibility_state.requires_scope_replacement(&present),
            "the appeared target directory makes the resolved scope non-actionable"
        );
        Ok(())
    }

    fn ready_scope_key(monitor_scope_update: &MonitorScopeUpdate) -> TestResult<&MonitorScopeKey> {
        match monitor_scope_update.monitor_scope_resolution() {
            MonitorScopeResolution::Ready(monitor_scope_key) => Ok(monitor_scope_key),
            MonitorScopeResolution::EmptyNonRust(_)
            | MonitorScopeResolution::PendingIndex(_)
            | MonitorScopeResolution::AmbiguousOwnership(_)
            | MonitorScopeResolution::UnresolvedPath(_) => {
                Err("row should resolve to an actionable monitor scope".into())
            },
        }
    }

    fn ready_scope_key_for_row(
        project_list: &mut ProjectList,
        cargo_workspace_index: &Arc<CargoWorkspaceIndex>,
        row_matches: impl Fn(VisibleRow) -> bool,
    ) -> TestResult<MonitorScopeKey> {
        let monitor_scope_update =
            monitor_scope_update_for_row(project_list, cargo_workspace_index, row_matches)?;
        match monitor_scope_update.monitor_scope_resolution() {
            MonitorScopeResolution::Ready(monitor_scope_key) => Ok(monitor_scope_key.clone()),
            MonitorScopeResolution::EmptyNonRust(_)
            | MonitorScopeResolution::PendingIndex(_)
            | MonitorScopeResolution::AmbiguousOwnership(_)
            | MonitorScopeResolution::UnresolvedPath(_) => {
                Err("row should resolve to an actionable monitor scope".into())
            },
        }
    }

    fn monitor_scope_update_for_row(
        project_list: &mut ProjectList,
        cargo_workspace_index: &Arc<CargoWorkspaceIndex>,
        row_matches: impl Fn(VisibleRow) -> bool,
    ) -> TestResult<MonitorScopeUpdate> {
        let row_index = project_list
            .visible_rows()
            .iter()
            .position(|visible_row| row_matches(*visible_row))
            .ok_or("selected row should exist")?;
        project_list.set_cursor(row_index);
        Ok(resolve_monitor_scope(
            monitor_scope_input(project_list),
            WorkspaceIndexReadiness::Current {
                cargo_workspace_index,
            },
        ))
    }

    fn ambiguous_cargo_workspace_index(
        scope_fixture: &ScopeFixture,
    ) -> TestResult<CargoWorkspaceIndex> {
        let shared_root = scope_fixture
            .project_list
            .get(0)
            .ok_or("standard root should exist")?
            .root_item
            .path()
            .clone();
        let alternate_checkout = scope_fixture.temp_dir.path().join("alternate-checkout");
        fs::create_dir_all(&alternate_checkout)?;
        let mut workspace_metadata_store = WorkspaceMetadataStore::new();
        workspace_metadata_store.upsert(WorkspaceMetadata {
            declared_checkout_root:   AbsolutePath::from(alternate_checkout),
            cargo_workspace_root:     shared_root.clone(),
            target_directory:         AbsolutePath::from(shared_root.as_path().join("target")),
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
        });
        workspace_metadata_store.upsert(workspace_metadata(
            shared_root.as_path(),
            shared_root.as_path(),
        ));
        Ok(CargoWorkspaceIndex::from_metadata_store(
            &workspace_metadata_store,
            scope_fixture.project_list.revision(),
        ))
    }

    /// A disabled monitor owns no schedule at all: it contributes no dispatch
    /// deadline, records nothing, and refuses every result, so nothing
    /// compile-specific wakes the event loop while visibility is off.
    #[test]
    fn a_disabled_monitor_owns_no_refresh_deadline() {
        let mut compile_visibility_state = CompileVisibilityState::Off;
        let now = Instant::now();

        compile_visibility_state.record_refresh_dispatch(now);
        compile_visibility_state.record_refresh_settled(now);

        assert_eq!(compile_visibility_state, CompileVisibilityState::Off);
        assert_eq!(
            compile_visibility_state.compile_monitor_refresh_schedule(),
            CompileMonitorRefreshSchedule::NotScheduled
        );
        assert!(!compile_visibility_state.accepts_generation(CompileMonitorGeneration::default()));
    }

    /// A scope that authorizes no classification owes no dispatch deadline.
    /// The cycle it would dispatch requests nothing, so nothing would record
    /// the dispatch or the settle: the deadline would stay in the past and wake
    /// the event loop with a zero timeout on every tick for as long as the row
    /// stayed selected.
    #[test]
    fn a_non_actionable_scope_owes_no_refresh_deadline() -> TestResult {
        let mut scope_fixture = scope_fixture()?;
        scope_fixture.project_list.expand_all(true);
        scope_fixture.project_list.recompute_visibility(true);
        let monitor_scope_update = monitor_scope_update_for_row(
            &mut scope_fixture.project_list,
            &scope_fixture.cargo_workspace_index,
            |visible_row| matches!(visible_row, VisibleRow::Root { node_index: 2 }),
        )?;
        let mut compile_visibility_state = CompileVisibilityState::Off;
        let enabled_at = Instant::now();

        compile_visibility_state.enable(
            monitor_scope_update,
            CompileMonitorGeneration::default(),
            enabled_at,
        );

        assert!(
            compile_visibility_state.is_on(),
            "the non-Rust row is still a selected scope the monitor is enabled over"
        );
        for elapsed_ticks in 0..4_u32 {
            assert_eq!(
                compile_visibility_state.compile_monitor_refresh_schedule(),
                CompileMonitorRefreshSchedule::NotScheduled,
                "tick {elapsed_ticks} over a non-actionable scope owes no deadline"
            );
        }
        Ok(())
    }

    /// An enabled monitor owes its first refresh at once, owes none while one
    /// is with the worker, and owes the next one interval after the one that
    /// went out.
    #[test]
    fn an_enabled_monitor_rearms_one_interval_after_its_refresh_settles() -> TestResult {
        let mut scope_fixture = scope_fixture()?;
        scope_fixture.project_list.expand_all(true);
        scope_fixture.project_list.recompute_visibility(true);
        let monitor_scope_update = monitor_scope_update_for_row(
            &mut scope_fixture.project_list,
            &scope_fixture.cargo_workspace_index,
            |visible_row| matches!(visible_row, VisibleRow::Root { node_index: 0 }),
        )?;
        let mut compile_monitor_generation = CompileMonitorGeneration::default();
        compile_monitor_generation.advance();
        let mut compile_visibility_state = CompileVisibilityState::Off;
        let enabled_at = Instant::now();

        compile_visibility_state.enable(
            monitor_scope_update,
            compile_monitor_generation,
            enabled_at,
        );

        assert_eq!(
            compile_visibility_state.compile_monitor_refresh_schedule(),
            CompileMonitorRefreshSchedule::At(enabled_at)
        );

        compile_visibility_state.record_refresh_dispatch(enabled_at);

        assert_eq!(
            compile_visibility_state.compile_monitor_refresh_schedule(),
            CompileMonitorRefreshSchedule::NotScheduled
        );

        compile_visibility_state.record_refresh_settled(enabled_at + Duration::from_millis(120));

        assert_eq!(
            compile_visibility_state.compile_monitor_refresh_schedule(),
            CompileMonitorRefreshSchedule::At(enabled_at + COMPILE_MONITOR_REFRESH_INTERVAL)
        );
        Ok(())
    }

    /// A refresh that settles several intervals late owes exactly one next
    /// refresh, at the next interval boundary, rather than one queued demand
    /// per interval that elapsed while it was out.
    #[test]
    fn a_late_settlement_coalesces_the_intervals_it_missed() -> TestResult {
        let mut scope_fixture = scope_fixture()?;
        scope_fixture.project_list.expand_all(true);
        scope_fixture.project_list.recompute_visibility(true);
        let monitor_scope_update = monitor_scope_update_for_row(
            &mut scope_fixture.project_list,
            &scope_fixture.cargo_workspace_index,
            |visible_row| matches!(visible_row, VisibleRow::Root { node_index: 0 }),
        )?;
        let mut compile_monitor_generation = CompileMonitorGeneration::default();
        compile_monitor_generation.advance();
        let mut compile_visibility_state = CompileVisibilityState::Off;
        let enabled_at = Instant::now();
        compile_visibility_state.enable(
            monitor_scope_update,
            compile_monitor_generation,
            enabled_at,
        );
        compile_visibility_state.record_refresh_dispatch(enabled_at);

        compile_visibility_state.record_refresh_settled(
            enabled_at + COMPILE_MONITOR_REFRESH_INTERVAL * 3 + Duration::from_millis(200),
        );

        assert_eq!(
            compile_visibility_state.compile_monitor_refresh_schedule(),
            CompileMonitorRefreshSchedule::At(enabled_at + COMPILE_MONITOR_REFRESH_INTERVAL * 4)
        );
        Ok(())
    }
}
