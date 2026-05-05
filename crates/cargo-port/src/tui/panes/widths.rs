//! Column-fit width math for the project list.
//!
//! The math mirrors the renderer in `panes::project_list` — every
//! visible row's width is `display_width(prefix) +
//! display_width(label)`. Whoever owns the prefix strings has to
//! own this math, otherwise the reserved column width and the
//! rendered content can drift. Co-locating the two in `panes/`
//! keeps the `PREFIX_*` constants private to the renderer module.
//!
//! Single entry: [`compute_project_list_widths`]. Internal
//! observers walk the project tree, accumulating per-column
//! width observations into a [`ProjectListWidths`] which the
//! Selection subsystem stores.

use super::constants::PREFIX_GROUP_COLLAPSED;
use super::constants::PREFIX_MEMBER_INLINE;
use super::constants::PREFIX_MEMBER_NAMED;
use super::constants::PREFIX_ROOT_COLLAPSED;
use super::constants::PREFIX_SUBMODULE;
use super::constants::PREFIX_VENDORED;
use super::constants::PREFIX_WT_COLLAPSED;
use super::constants::PREFIX_WT_FLAT;
use super::constants::PREFIX_WT_GROUP_COLLAPSED;
use super::constants::PREFIX_WT_MEMBER_INLINE;
use super::constants::PREFIX_WT_MEMBER_NAMED;
use super::constants::PREFIX_WT_VENDORED;
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
use crate::project::Workspace;
use crate::project::WorktreeGroup;
use crate::tui::columns;
use crate::tui::columns::COL_DISK;
use crate::tui::columns::COL_MAIN;
use crate::tui::columns::COL_NAME;
use crate::tui::columns::COL_SYNC;
use crate::tui::columns::ProjectListWidths;
use crate::tui::project_list::ProjectList;
use crate::tui::render;

/// Walk the project tree and produce a `ProjectListWidths`.
/// Single entry point used by `App::ensure_fit_widths`.
pub fn compute_project_list_widths(
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
pub const fn name_width_with_gutter(content_width: usize) -> usize {
    content_width.saturating_add(1)
}

fn observe_name_width(widths: &mut ProjectListWidths, content_width: usize) {
    widths.observe(COL_NAME, name_width_with_gutter(content_width));
}

fn observe_item_fit_widths(widths: &mut ProjectListWidths, entry: &ProjectEntry, root_label: &str) {
    let dw = columns::display_width;
    let item = &entry.item;
    let repo_info = entry
        .git_repo
        .as_ref()
        .and_then(|repo| repo.repo_info.as_ref());

    observe_name_width(widths, dw(PREFIX_ROOT_COLLAPSED) + dw(root_label));
    widths.observe(COL_DISK, dw(&formatted_disk(item.disk_usage_bytes())));
    widths.observe(COL_SYNC, dw(&git_sync_label(item.git_info(), repo_info)));
    widths.observe(COL_MAIN, dw(&git_main_sync_label(item.git_info())));

    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            observe_new_member_group_fit_widths(widths, ws.groups(), false);
            observe_typed_vendored_fit_widths(widths, ws.vendored(), PREFIX_VENDORED);
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            observe_typed_vendored_fit_widths(widths, pkg.vendored(), PREFIX_VENDORED);
        },
        RootItem::NonRust(_) => {},
        RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. }) => {
            observe_workspace_worktree_group_fit_widths(widths, wtg, repo_info);
        },
        RootItem::Worktrees(wtg @ WorktreeGroup::Packages { .. }) => {
            observe_package_worktree_group_fit_widths(widths, wtg, repo_info);
        },
    }
    for submodule in item.submodules() {
        let label = format!("{} (s)", submodule.name);
        observe_path_only_entry_fit_widths(widths, PREFIX_SUBMODULE, &label, submodule);
    }
}

fn observe_path_only_entry_fit_widths(
    widths: &mut ProjectListWidths,
    prefix: &str,
    label: &str,
    entry: &impl ProjectFields,
) {
    let dw = columns::display_width;
    observe_name_width(widths, dw(prefix) + dw(label));
    widths.observe(COL_DISK, dw(&formatted_disk(entry.info().disk_usage_bytes)));
}

fn observe_new_member_group_fit_widths(
    widths: &mut ProjectListWidths,
    groups: &[MemberGroup],
    is_worktree: bool,
) {
    let dw = columns::display_width;
    for group in groups {
        let (inline_prefix, named_prefix, group_prefix) = if is_worktree {
            (
                PREFIX_WT_MEMBER_INLINE,
                PREFIX_WT_MEMBER_NAMED,
                PREFIX_WT_GROUP_COLLAPSED,
            )
        } else {
            (
                PREFIX_MEMBER_INLINE,
                PREFIX_MEMBER_NAMED,
                PREFIX_GROUP_COLLAPSED,
            )
        };
        for member in group.members() {
            let prefix = if group.is_named() {
                named_prefix
            } else {
                inline_prefix
            };
            observe_name_width(widths, dw(prefix) + dw(member.package_name().as_str()));
            widths.observe(COL_DISK, dw(&formatted_disk(member.disk_usage_bytes())));
        }
        if group.is_named() {
            let label = format!("{} ({})", group.group_name(), group.members().len());
            observe_name_width(widths, dw(group_prefix) + dw(&label));
        }
    }
}

fn observe_typed_vendored_fit_widths(
    widths: &mut ProjectListWidths,
    vendored: &[VendoredPackage],
    prefix: &str,
) {
    let dw = columns::display_width;
    for project in vendored {
        let label = format!("{} (vendored)", project.package_name());
        observe_name_width(widths, dw(prefix) + dw(&label));
        widths.observe(COL_DISK, dw(&formatted_disk(project.disk_usage_bytes())));
    }
}

fn observe_workspace_worktree_entry_fit_widths(
    widths: &mut ProjectListWidths,
    ws: &Workspace,
    repo_info: Option<&RepoInfo>,
) {
    let dw = columns::display_width;
    let wt_name = ws.root_directory_name().into_string();
    let prefix = if ws.has_members() {
        PREFIX_WT_COLLAPSED
    } else {
        PREFIX_WT_FLAT
    };
    observe_name_width(widths, dw(prefix) + dw(&wt_name));
    widths.observe(COL_DISK, dw(&formatted_disk(ws.disk_usage_bytes())));
    widths.observe(COL_SYNC, dw(&git_sync_label(ws.git_info(), repo_info)));
    widths.observe(COL_MAIN, dw(&git_main_sync_label(ws.git_info())));
    observe_new_member_group_fit_widths(widths, ws.groups(), true);
    observe_typed_vendored_fit_widths(widths, ws.vendored(), PREFIX_WT_VENDORED);
}

fn observe_package_worktree_entry_fit_widths(
    widths: &mut ProjectListWidths,
    pkg: &Package,
    repo_info: Option<&RepoInfo>,
) {
    let dw = columns::display_width;
    let wt_name = pkg.root_directory_name().into_string();
    observe_name_width(widths, dw(PREFIX_WT_FLAT) + dw(&wt_name));
    widths.observe(COL_DISK, dw(&formatted_disk(pkg.disk_usage_bytes())));
    widths.observe(COL_SYNC, dw(&git_sync_label(pkg.git_info(), repo_info)));
    widths.observe(COL_MAIN, dw(&git_main_sync_label(pkg.git_info())));
    observe_typed_vendored_fit_widths(widths, pkg.vendored(), PREFIX_WT_VENDORED);
}

fn observe_workspace_worktree_group_fit_widths(
    widths: &mut ProjectListWidths,
    wtg: &WorktreeGroup,
    repo_info: Option<&RepoInfo>,
) {
    let WorktreeGroup::Workspaces {
        primary, linked, ..
    } = wtg
    else {
        return;
    };
    observe_workspace_worktree_entry_fit_widths(widths, primary, repo_info);
    for ws in linked {
        observe_workspace_worktree_entry_fit_widths(widths, ws, repo_info);
    }
}

fn observe_package_worktree_group_fit_widths(
    widths: &mut ProjectListWidths,
    wtg: &WorktreeGroup,
    repo_info: Option<&RepoInfo>,
) {
    let WorktreeGroup::Packages {
        primary, linked, ..
    } = wtg
    else {
        return;
    };
    observe_package_worktree_entry_fit_widths(widths, primary, repo_info);
    for pkg in linked {
        observe_package_worktree_entry_fit_widths(widths, pkg, repo_info);
    }
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
