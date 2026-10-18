//! Column-fit width math for the project list.
//!
//! The math mirrors the renderer in `panes::project_list` — every
//! visible row's width is `display_width(prefix) +
//! display_width(label)`. Whoever owns the prefix strings has to
//! own this math, otherwise the reserved column width and the
//! rendered content can drift. Co-locating the two in `panes/`
//! shares complete tree-prefix endings and their measured widths.
//! The Disk column measures the size text plus the row's suffix,
//! shared with the renderer through the same constants.
//!
//! Single entry: [`compute_project_list_widths`]. Internal
//! observers walk the project tree, accumulating per-column
//! width observations into a [`ProjectListWidths`] which the
//! Selection subsystem stores.

use super::constants;
use super::constants::BROKEN_WORKTREE_DISK_SUFFIX;
use super::constants::PREFIX_ROOT_COLLAPSED;
use super::constants::TreeRowDepth;
use crate::constants::IN_SYNC;
use crate::constants::NO_REMOTE_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::project::CheckoutInfo;
use crate::project::GitOrigin;
use crate::project::GitStatus;
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
    root_labels: &[String],
    lint_enabled: bool,
    generation: u64,
) -> ProjectListWidths {
    let mut widths = ProjectListWidths::new(lint_enabled);
    for (index, entry) in entries.iter().enumerate() {
        observe_item_fit_widths(&mut widths, entry, &root_labels[index]);
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

fn observe_disk_width(widths: &mut ProjectListWidths, disk: &str, health: WorktreeHealth) {
    let suffix_width = match health {
        WorktreeHealth::Normal => 0,
        WorktreeHealth::Broken => columns::display_width(BROKEN_WORKTREE_DISK_SUFFIX),
    };
    widths.observe(COL_DISK, columns::display_width(disk) + suffix_width);
}

fn observe_item_fit_widths(widths: &mut ProjectListWidths, entry: &ProjectEntry, root_label: &str) {
    let dw = columns::display_width;
    let item = &entry.root_item;
    let repo_info = entry
        .git_repo
        .as_ref()
        .and_then(|repo| repo.repo_info.as_ref());

    observe_name_width(widths, dw(PREFIX_ROOT_COLLAPSED) + dw(root_label));
    observe_disk_width(
        widths,
        &formatted_disk(item.disk_usage_bytes()),
        item.worktree_health(),
    );
    widths.observe(COL_SYNC, dw(&git_sync_label(item.git_info(), repo_info)));
    widths.observe(COL_MAIN, dw(&git_main_sync_label(item.git_info())));

    match item {
        RootItem::Rust(project) => observe_rust_children_fit_widths(widths, project),
        RootItem::NonRust(_) => {},
        RootItem::Worktrees(worktree_group) => {
            observe_worktree_group_fit_widths(widths, worktree_group, repo_info);
        },
    }
    for submodule in item.submodules() {
        let label = format!("{} (s)", submodule.name);
        observe_path_only_entry_fit_widths(
            widths,
            constants::tree_prefix_width(TreeRowDepth::root_child()),
            &label,
            submodule,
        );
    }
}

fn observe_rust_children_fit_widths(widths: &mut ProjectListWidths, project: &RustProject) {
    if let RustProject::Workspace(ws) = project {
        observe_new_member_group_fit_widths(widths, ws.groups(), TreeRowDepth::root_child());
    }
    observe_typed_vendored_fit_widths(
        widths,
        &project.rust_info().vendored,
        TreeRowDepth::root_child(),
    );
}

fn observe_path_only_entry_fit_widths(
    widths: &mut ProjectListWidths,
    prefix_width: usize,
    label: &str,
    entry: &impl ProjectFields,
) {
    let dw = columns::display_width;
    observe_name_width(widths, prefix_width + dw(label));
    observe_disk_width(
        widths,
        &formatted_disk(entry.info().disk_usage_bytes),
        entry.info().worktree_health,
    );
}

fn observe_new_member_group_fit_widths(
    widths: &mut ProjectListWidths,
    groups: &[MemberGroup],
    group_depth: TreeRowDepth,
) {
    let dw = columns::display_width;
    for group in groups {
        let member_depth = group_depth.member_of(group);
        for member in group.members() {
            observe_name_width(
                widths,
                constants::tree_prefix_width(member_depth) + dw(member.package_name().as_str()),
            );
            widths.observe(COL_DISK, dw(&formatted_disk(member.disk_usage_bytes())));
            observe_typed_vendored_fit_widths(widths, member.vendored(), member_depth.child());
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
) {
    let dw = columns::display_width;
    for project in vendored {
        let label = constants::vendored_row_label(project.package_name().as_str());
        observe_name_width(widths, constants::tree_prefix_width(depth) + dw(&label));
        widths.observe(COL_DISK, dw(&formatted_disk(project.disk_usage_bytes())));
    }
}

fn observe_workspace_worktree_entry_fit_widths(
    widths: &mut ProjectListWidths,
    ws: &Workspace,
    group: &WorktreeGroup,
    worktree_index: usize,
    repo_info: Option<&RepoInfo>,
) {
    let dw = columns::display_width;
    let worktree_name = worktree_entry_label(group, worktree_index);
    observe_name_width(
        widths,
        constants::tree_prefix_width(TreeRowDepth::root_child()) + dw(&worktree_name),
    );
    observe_disk_width(
        widths,
        &formatted_disk(ws.disk_usage_bytes()),
        ws.worktree_health(),
    );
    widths.observe(COL_SYNC, dw(&git_sync_label(ws.git_info(), repo_info)));
    widths.observe(COL_MAIN, dw(&git_main_sync_label(ws.git_info())));
    let child_depth = TreeRowDepth::root_child().child();
    observe_new_member_group_fit_widths(widths, ws.groups(), child_depth);
    observe_typed_vendored_fit_widths(widths, ws.vendored(), child_depth);
}

fn observe_package_worktree_entry_fit_widths(
    widths: &mut ProjectListWidths,
    pkg: &Package,
    group: &WorktreeGroup,
    worktree_index: usize,
    repo_info: Option<&RepoInfo>,
) {
    let dw = columns::display_width;
    let worktree_name = worktree_entry_label(group, worktree_index);
    observe_name_width(
        widths,
        constants::tree_prefix_width(TreeRowDepth::root_child()) + dw(&worktree_name),
    );
    observe_disk_width(
        widths,
        &formatted_disk(pkg.disk_usage_bytes()),
        pkg.worktree_health(),
    );
    widths.observe(COL_SYNC, dw(&git_sync_label(pkg.git_info(), repo_info)));
    widths.observe(COL_MAIN, dw(&git_main_sync_label(pkg.git_info())));
    observe_typed_vendored_fit_widths(widths, pkg.vendored(), TreeRowDepth::root_child().child());
}

fn observe_worktree_group_fit_widths(
    widths: &mut ProjectListWidths,
    worktree_group: &WorktreeGroup,
    repo_info: Option<&RepoInfo>,
) {
    // Use the renderer's grouping decision: a single live checkout is the
    // root itself, so its children have no intervening worktree ancestor.
    if !worktree_group.renders_as_group() {
        if let Some(entry) = worktree_group.single_live() {
            observe_rust_children_fit_widths(widths, entry);
        }
        return;
    }
    for (worktree_index, entry) in worktree_group.iter_entries().enumerate() {
        match entry {
            RustProject::Workspace(ws) => {
                observe_workspace_worktree_entry_fit_widths(
                    widths,
                    ws,
                    worktree_group,
                    worktree_index,
                    repo_info,
                );
            },
            RustProject::Package(pkg) => {
                observe_package_worktree_entry_fit_widths(
                    widths,
                    pkg,
                    worktree_group,
                    worktree_index,
                    repo_info,
                );
            },
        }
    }
}

fn worktree_entry_label(group: &WorktreeGroup, worktree_index: usize) -> String {
    let Some(entry) = group.entry(worktree_index) else {
        return String::new();
    };
    let mut label = entry.root_directory_name().into_string();
    if renders_primary_marker(group, worktree_index) {
        label.push_str(" (p)");
    }
    label
}

fn renders_primary_marker(group: &WorktreeGroup, worktree_index: usize) -> bool {
    worktree_index == 0
        && group.visible_entry_count() > 2
        && group
            .entry(worktree_index)
            .is_some_and(|entry| entry.visibility() == Visibility::Visible)
}

fn formatted_disk(bytes: Option<u64>) -> String {
    bytes.map_or_else(|| render::format_bytes(0), render::format_bytes)
}

fn git_sync_label(checkout: Option<&CheckoutInfo>, repo: Option<&RepoInfo>) -> String {
    let Some(info) = checkout else {
        return String::new();
    };
    if matches!(info.status, GitStatus::Untracked | GitStatus::Ignored) {
        return String::new();
    }
    let primary_ab = repo.and_then(|r| info.primary_ahead_behind(r));
    let origin = repo.map_or(GitOrigin::Local, RepoInfo::origin_kind);
    match primary_ab {
        Some((0, 0)) => IN_SYNC.to_string(),
        Some((a, 0)) => format!("{SYNC_UP}{a}"),
        Some((0, b)) => format!("{SYNC_DOWN}{b}"),
        Some((a, b)) => format!("{SYNC_UP}{a}{SYNC_DOWN}{b}"),
        None if origin != GitOrigin::Local => "-".to_string(),
        None => NO_REMOTE_SYNC.to_string(),
    }
}

fn git_main_sync_label(checkout: Option<&CheckoutInfo>) -> String {
    let Some(info) = checkout else {
        return String::new();
    };
    if matches!(info.status, GitStatus::Untracked | GitStatus::Ignored) {
        return String::new();
    }
    match info.ahead_behind_local {
        Some((0, 0)) => IN_SYNC.to_string(),
        Some((a, 0)) => format!("{SYNC_UP}{a}"),
        Some((0, b)) => format!("{SYNC_DOWN}{b}"),
        Some((a, b)) => format!("{SYNC_UP}{a}{SYNC_DOWN}{b}"),
        None => String::new(),
    }
}
