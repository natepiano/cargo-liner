//! Column-fit width math for the project list.
//!
//! The math mirrors the renderer in `panes::project_list` — every
//! visible row's width is `display_width(prefix) +
//! display_width(label)`. Whoever owns the prefix strings has to
//! own this math, otherwise the reserved column width and the
//! rendered content can drift. Co-locating the two in `panes/`
//! shares complete tree-prefix endings and their measured widths.
//! The Disk column measures the size text plus the row's suffix,
//! shared with the renderer through `DiskCell`.
//!
//! Single entry: [`compute_project_list_widths`]. Internal
//! observers walk the project tree, accumulating per-column
//! width observations into a [`ProjectListWidths`] which the
//! Selection subsystem stores.

use std::path::Path;

use super::constants;
use super::constants::DiskCell;
use super::constants::PREFIX_ROOT_COLLAPSED;
use super::constants::TreeRowDepth;
use crate::project::CheckoutInfo;
use crate::project::MemberGroup;
use crate::project::Package;
use crate::project::ProjectEntry;
use crate::project::ProjectFields;
use crate::project::RepoInfo;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::VendoredPackage;
use crate::project::Visibility;
use crate::project::Workspace;
use crate::project::WorktreeGroup;
use crate::project::WorktreeHealth;
use crate::tui::app::ManifestMetadataStates;
use crate::tui::columns;
use crate::tui::columns::COL_DISK;
use crate::tui::columns::COL_MAIN;
use crate::tui::columns::COL_NAME;
use crate::tui::columns::COL_SYNC;
use crate::tui::columns::ProjectListWidths;
use crate::tui::project_list_state::ProjectList;
use crate::tui::render;

/// Walk the project tree and produce a `ProjectListWidths`.
/// Single entry point used by `App::ensure_fit_widths`.
pub(in crate::tui) fn compute_project_list_widths(
    entries: &ProjectList,
    metadata: &ManifestMetadataStates,
    root_labels: &[String],
    lint_enabled: bool,
    generation: u64,
) -> ProjectListWidths {
    let mut widths = ProjectListWidths::new(lint_enabled);
    for (index, entry) in entries.iter().enumerate() {
        observe_item_fit_widths(&mut widths, entries, metadata, entry, &root_labels[index]);
    }
    widths.generation = generation;
    widths
}

/// Tests reach into width math through a minimal surface; this
/// is the gutter-aware name-column width helper. Public via
/// `panes/mod.rs` for `app/tests/rows.rs` and `app/tests/panes.rs`.
pub(in crate::tui) const fn name_width_with_gutter(content_width: usize) -> usize {
    content_width.saturating_add(1)
}

fn observe_name_width(widths: &mut ProjectListWidths, content_width: usize) {
    widths.observe(COL_NAME, name_width_with_gutter(content_width));
}

/// Project state shared by the name-marker and deletion-aware disk measurements.
#[derive(Clone, Copy)]
struct ProjectFitContext<'a> {
    projects: &'a ProjectList,
    metadata: &'a ManifestMetadataStates,
    ancestor: &'a Path,
}

impl ProjectFitContext<'_> {
    fn cell(&self, entry: &impl ProjectFields) -> DiskCell {
        let deleted =
            self.projects.is_deleted(self.ancestor) || self.projects.is_deleted(entry.path());
        DiskCell::for_row(
            formatted_disk(entry.info().disk_usage_bytes),
            columns::RowLifecycle::from(deleted),
            entry.info().worktree_health,
        )
    }
}

fn observe_disk_width(widths: &mut ProjectListWidths, cell: &DiskCell) {
    widths.observe(COL_DISK, cell.width());
}

fn observe_item_fit_widths(
    widths: &mut ProjectListWidths,
    projects: &ProjectList,
    metadata: &ManifestMetadataStates,
    entry: &ProjectEntry,
    root_label: &str,
) {
    let dw = columns::display_width;
    let item = &entry.root_item;
    let disk_context = ProjectFitContext {
        projects,
        metadata,
        ancestor: item.path(),
    };
    let repo_info = entry
        .git_repo
        .as_ref()
        .and_then(|repo| repo.repo_info.as_ref());

    observe_name_width(
        widths,
        dw(PREFIX_ROOT_COLLAPSED)
            + dw(root_label)
            + dw(metadata.for_project(item.path()).name_marker()),
    );
    let cell = DiskCell::for_row(
        formatted_disk(item.disk_usage_bytes()),
        columns::RowLifecycle::from(projects.is_deleted(item.path())),
        item.worktree_health(),
    );
    observe_disk_width(widths, &cell);
    widths.observe(COL_SYNC, dw(&git_sync_label(item.git_info(), repo_info)));
    widths.observe(
        COL_MAIN,
        dw(&git_main_sync_label(item.git_info(), repo_info)),
    );

    match item {
        RootItem::Rust(project) => observe_rust_children_fit_widths(widths, disk_context, project),
        RootItem::NonRust(_) => {},
        RootItem::Worktrees(worktree_group) => {
            observe_worktree_group_fit_widths(
                widths,
                projects,
                metadata,
                worktree_group,
                repo_info,
            );
        },
    }
    for submodule in item.submodules() {
        let label = format!("{} (s)", submodule.name);
        observe_path_only_entry_fit_widths(
            widths,
            constants::tree_prefix_width(TreeRowDepth::root_child()),
            &label,
            submodule,
            disk_context,
        );
    }
}

fn observe_rust_children_fit_widths(
    widths: &mut ProjectListWidths,
    disk_context: ProjectFitContext<'_>,
    project: &RustProject,
) {
    if let RustProject::Workspace(ws) = project {
        observe_new_member_group_fit_widths(
            widths,
            ws.groups(),
            TreeRowDepth::root_child(),
            disk_context,
        );
    }
    observe_typed_vendored_fit_widths(
        widths,
        &project.rust_info().vendored,
        TreeRowDepth::root_child(),
        disk_context,
    );
}

fn observe_path_only_entry_fit_widths(
    widths: &mut ProjectListWidths,
    prefix_width: usize,
    label: &str,
    entry: &impl ProjectFields,
    disk_context: ProjectFitContext<'_>,
) {
    let dw = columns::display_width;
    observe_name_width(
        widths,
        prefix_width
            + dw(label)
            + dw(disk_context
                .metadata
                .for_project(entry.path())
                .name_marker()),
    );
    observe_disk_width(widths, &disk_context.cell(entry));
}

fn observe_new_member_group_fit_widths(
    widths: &mut ProjectListWidths,
    groups: &[MemberGroup],
    group_depth: TreeRowDepth,
    disk_context: ProjectFitContext<'_>,
) {
    let dw = columns::display_width;
    for group in groups {
        let member_depth = group_depth.member_of(group);
        for member in group.members() {
            observe_name_width(
                widths,
                constants::tree_prefix_width(member_depth)
                    + dw(member.package_name().as_str())
                    + dw(disk_context
                        .metadata
                        .for_project(member.path())
                        .name_marker()),
            );
            let cell = DiskCell::for_row(
                formatted_disk(member.disk_usage_bytes()),
                columns::RowLifecycle::from(
                    disk_context.projects.is_deleted(disk_context.ancestor)
                        || disk_context.projects.is_deleted(member.path()),
                ),
                WorktreeHealth::Normal,
            );
            observe_disk_width(widths, &cell);
            observe_typed_vendored_fit_widths(
                widths,
                member.vendored(),
                member_depth.child(),
                disk_context,
            );
        }
        if group.is_named() {
            let label = format!("{} ({})", group.group_name(), group.members().len());
            observe_name_width(
                widths,
                constants::tree_prefix_width(group_depth) + dw(&label),
            );
        }
    }
}

fn observe_typed_vendored_fit_widths(
    widths: &mut ProjectListWidths,
    vendored: &[VendoredPackage],
    depth: TreeRowDepth,
    disk_context: ProjectFitContext<'_>,
) {
    let dw = columns::display_width;
    for project in vendored {
        let label = constants::vendored_row_label(project.package_name().as_str());
        observe_name_width(
            widths,
            constants::tree_prefix_width(depth)
                + dw(&label)
                + dw(disk_context
                    .metadata
                    .for_project(project.path())
                    .name_marker()),
        );
        let cell = DiskCell::for_row(
            formatted_disk(project.disk_usage_bytes()),
            columns::RowLifecycle::from(
                disk_context.projects.is_deleted(disk_context.ancestor)
                    || disk_context.projects.is_deleted(project.path()),
            ),
            WorktreeHealth::Normal,
        );
        observe_disk_width(widths, &cell);
    }
}

fn observe_workspace_worktree_entry_fit_widths(
    widths: &mut ProjectListWidths,
    ws: &Workspace,
    worktree_name: &str,
    repo_info: Option<&RepoInfo>,
    projects: &ProjectList,
    metadata: &ManifestMetadataStates,
) {
    let disk_context = ProjectFitContext {
        projects,
        metadata,
        ancestor: ws.path(),
    };
    let dw = columns::display_width;
    observe_name_width(
        widths,
        constants::tree_prefix_width(TreeRowDepth::root_child())
            + dw(worktree_name)
            + dw(metadata.for_project(disk_context.ancestor).name_marker()),
    );
    observe_disk_width(widths, &disk_context.cell(ws));
    widths.observe(COL_SYNC, dw(&git_sync_label(ws.git_info(), repo_info)));
    widths.observe(COL_MAIN, dw(&git_main_sync_label(ws.git_info(), repo_info)));
    let child_depth = TreeRowDepth::root_child().child();
    observe_new_member_group_fit_widths(widths, ws.groups(), child_depth, disk_context);
    observe_typed_vendored_fit_widths(widths, ws.vendored(), child_depth, disk_context);
}

fn observe_package_worktree_entry_fit_widths(
    widths: &mut ProjectListWidths,
    pkg: &Package,
    worktree_name: &str,
    repo_info: Option<&RepoInfo>,
    projects: &ProjectList,
    metadata: &ManifestMetadataStates,
) {
    let disk_context = ProjectFitContext {
        projects,
        metadata,
        ancestor: pkg.path(),
    };
    let dw = columns::display_width;
    observe_name_width(
        widths,
        constants::tree_prefix_width(TreeRowDepth::root_child())
            + dw(worktree_name)
            + dw(metadata.for_project(disk_context.ancestor).name_marker()),
    );
    observe_disk_width(widths, &disk_context.cell(pkg));
    widths.observe(COL_SYNC, dw(&git_sync_label(pkg.git_info(), repo_info)));
    widths.observe(
        COL_MAIN,
        dw(&git_main_sync_label(pkg.git_info(), repo_info)),
    );
    observe_typed_vendored_fit_widths(
        widths,
        pkg.vendored(),
        TreeRowDepth::root_child().child(),
        disk_context,
    );
}

/// The worktree name drawn in the list, including its primary marker.
pub(super) fn worktree_entry_label(
    entry: &RustProject,
    visible_entry_count: usize,
    is_first_entry: bool,
) -> String {
    let mut name = entry.root_directory_name().into_string();
    if is_first_entry && visible_entry_count > 2 && entry.visibility() == Visibility::Visible {
        name.push_str(" (p)");
    }
    name
}

fn observe_worktree_group_fit_widths(
    widths: &mut ProjectListWidths,
    projects: &ProjectList,
    metadata: &ManifestMetadataStates,
    worktree_group: &WorktreeGroup,
    repo_info: Option<&RepoInfo>,
) {
    // Use `worktree_group.renders_as_group()` just as rendering does: with one live checkout,
    // `single_live()` is the root itself, so its children have no intervening worktree ancestor.
    if !worktree_group.renders_as_group() {
        if let Some(entry) = worktree_group.single_live() {
            observe_rust_children_fit_widths(
                widths,
                ProjectFitContext {
                    projects,
                    metadata,
                    ancestor: entry.path(),
                },
                entry,
            );
        }
        return;
    }
    for (worktree_index, entry) in worktree_group.iter_entries().enumerate() {
        let worktree_name = worktree_entry_label(
            entry,
            worktree_group.visible_entry_count(),
            worktree_index == 0,
        );
        match entry {
            RustProject::Workspace(ws) => {
                observe_workspace_worktree_entry_fit_widths(
                    widths,
                    ws,
                    &worktree_name,
                    repo_info,
                    projects,
                    metadata,
                );
            },
            RustProject::Package(pkg) => {
                observe_package_worktree_entry_fit_widths(
                    widths,
                    pkg,
                    &worktree_name,
                    repo_info,
                    projects,
                    metadata,
                );
            },
        }
    }
}

fn formatted_disk(bytes: Option<u64>) -> String {
    bytes.map_or_else(|| render::format_bytes(0), render::format_bytes)
}

fn git_sync_label(checkout: Option<&CheckoutInfo>, repo: Option<&RepoInfo>) -> String {
    super::GitSyncLabels::of(checkout, repo).origin
}

fn git_main_sync_label(checkout: Option<&CheckoutInfo>, repo: Option<&RepoInfo>) -> String {
    super::GitSyncLabels::of(checkout, repo).main
}
