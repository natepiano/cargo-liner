use std::collections::HashSet;
use std::path::Path;

use ratatui::style::Color;
use tui_pane::title_color;

use crate::ci;
use crate::ci::OwnerRepo;
use crate::project::AbsolutePath;
use crate::project::CheckoutInfo;
use crate::project::ExampleGroup;
use crate::project::Package;
use crate::project::ProjectEntry;
use crate::project::ProjectFields;
use crate::project::ProjectType;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::VendoredPackage;
use crate::project::Visibility;
use crate::project::Workspace;
use crate::tui::panes::RunTargetKind;
use crate::tui::project_list::ProjectList;

/// A searchable item in the universal finder.
#[derive(Clone)]
pub struct FinderItem {
    /// Display name shown in the results list.
    pub display_name:  String,
    /// Search tokens derived from visible fields and path segments.
    pub search_tokens: Vec<String>,
    /// What kind of item this is.
    pub kind:          FinderKind,
    /// Path of the project this item belongs to (for navigation).
    pub project_path:  AbsolutePath,
    /// For targets: the cargo target name (used with --example/--bench).
    pub target_name:   Option<String>,
    /// Parent project display name (shown dimmed for non-project items).
    pub parent_label:  String,
    /// Git branch, if known. Distinguishes worktrees.
    pub branch:        String,
    /// Directory name (last path component).
    pub dir:           String,
    pub pr_target:     Option<PullRequestTarget>,
}

#[derive(Clone)]
pub struct PullRequestTarget {
    pub owner_repo: OwnerRepo,
    pub number:     u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FinderKind {
    Project,
    Binary,
    Example,
    Bench,
    PullRequest,
}

impl FinderKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Binary => "bin",
            Self::Example => "example",
            Self::Bench => "bench",
            Self::PullRequest => "pr",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Self::Project | Self::PullRequest => title_color(),
            Self::Binary => RunTargetKind::Binary.color(),
            Self::Example => RunTargetKind::Example.color(),
            Self::Bench => RunTargetKind::Bench.color(),
        }
    }
}

/// Column width metrics cached at index build time so the popup renders at a
/// stable size regardless of the current query results.
pub const FINDER_COLUMN_COUNT: usize = 5;
pub const FINDER_HEADERS: [&str; FINDER_COLUMN_COUNT] =
    ["Name", "Project", "Branch", "Dir", "Type"];

/// Build a flat index of all searchable items from the project list.
/// Uses the tree structure so workspace members inherit the branch
/// from their workspace root (members don't have their own `.git`).
/// Only `Visible` entries are indexed: `Deleted` (gone from disk) and
/// `Dismissed` ones are skipped so the finder only returns live,
/// navigable targets. Returns `(items, col_widths)` where `col_widths` is
/// the max display width of each column across the entire index.
pub fn build_finder_index(
    entries: &ProjectList,
) -> (Vec<FinderItem>, [usize; FINDER_COLUMN_COUNT]) {
    let mut items = Vec::new();
    let mut seen_pull_requests = HashSet::new();

    for entry in entries {
        if entry.item.visibility() != Visibility::Visible {
            continue;
        }
        match &entry.item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                add_workspace_items(&mut items, ws);
            },
            RootItem::Rust(RustProject::Package(pkg)) => {
                add_package_items(&mut items, pkg);
            },
            RootItem::NonRust(nr) => {
                let dp = nr.display_path().into_string();
                let abs = nr.path();
                let branch = branch_for(nr.git_info());
                let root_name = nr.root_directory_name().into_string();
                let context = TypedProjectContext {
                    project_name: &root_name,
                    cargo_name:   None,
                    abs_path:     abs,
                    display_path: &dp,
                    branch:       &branch,
                };
                add_project_items_from_typed(&mut items, &context, &[], &[], &[]);
            },
            RootItem::Worktrees(group) => {
                let primary_path = group.primary.path().clone();
                let mut emit = |entry: &RustProject| match entry {
                    RustProject::Workspace(ws) => add_workspace_items(&mut items, ws),
                    RustProject::Package(pkg) => add_package_items(&mut items, pkg),
                };
                if group.primary.visibility() == Visibility::Visible {
                    emit(&group.primary);
                }
                for l in &group.linked {
                    if l.path() == &primary_path {
                        continue;
                    }
                    if l.visibility() != Visibility::Visible {
                        continue;
                    }
                    emit(l);
                }
            },
        }
        add_pull_request_items(entries, &mut items, &mut seen_pull_requests, entry);
    }

    // Pre-compute column widths from the full index
    let mut col_widths: [usize; FINDER_COLUMN_COUNT] = FINDER_HEADERS.map(str::len);
    for item in &items {
        col_widths[0] = col_widths[0].max(item.display_name.len());
        col_widths[1] = col_widths[1].max(if item.kind == FinderKind::Project {
            0
        } else {
            item.parent_label.len()
        });
        col_widths[2] = col_widths[2].max(item.branch.len());
        col_widths[3] = col_widths[3].max(item.dir.len());
        col_widths[4] = col_widths[4].max(item.kind.label().len());
    }

    (items, col_widths)
}

fn add_pull_request_items(
    entries: &ProjectList,
    items: &mut Vec<FinderItem>,
    seen: &mut HashSet<(OwnerRepo, u32)>,
    entry: &ProjectEntry,
) {
    let Some(url) = entries.fetch_url_for(entry.item.path()) else {
        return;
    };
    if ci::parse_owner_repo(&url).is_none() {
        return;
    }
    let Some(info) = entry.git_repo.as_ref().and_then(|repo| repo.pr_data.info()) else {
        return;
    };
    let project_label = entry.item.root_directory_name().into_string();
    let dir = entry.item.display_path().into_string();
    for pull_request in &info.open {
        if !seen.insert((info.owner_repo.clone(), pull_request.number)) {
            continue;
        }
        let display_name = format!("#{} {}", pull_request.number, pull_request.title);
        let branch = pull_request.branch_label(&info.default_branch);
        let number = pull_request.number.to_string();
        let state_label = pull_request.state.label();
        let head_owner = pull_request.head_owner.as_deref().unwrap_or("");
        let head_repo = pull_request.head_repo.as_deref().unwrap_or("");
        items.push(FinderItem {
            search_tokens: build_search_tokens(&[
                &number,
                &pull_request.title,
                &branch,
                &pull_request.head,
                &pull_request.base,
                state_label,
                head_owner,
                head_repo,
                &info.viewer_login,
                FinderKind::PullRequest.label(),
            ]),
            display_name,
            kind: FinderKind::PullRequest,
            project_path: entry.item.path().clone(),
            target_name: None,
            parent_label: project_label.clone(),
            branch,
            dir: dir.clone(),
            pr_target: Some(PullRequestTarget {
                owner_repo: info.owner_repo.clone(),
                number:     pull_request.number,
            }),
        });
    }
}

fn branch_for(git_info: Option<&CheckoutInfo>) -> String {
    git_info.map_or_else(String::new, |g| g.head.display_label())
}

fn add_workspace_items(items: &mut Vec<FinderItem>, ws: &Workspace) {
    let root_path = ws.display_path().into_string();
    let root_abs_path = ws.path();
    let root_branch = branch_for(ws.git_info());
    let cargo = &ws.cargo;
    let root_name = ws.root_directory_name().into_string();
    let cargo_name = ws.package_name().into_string();
    let cargo_name = (cargo_name != root_name).then_some(cargo_name);
    let root_context = TypedProjectContext {
        project_name: &root_name,
        cargo_name:   cargo_name.as_deref(),
        abs_path:     root_abs_path,
        display_path: &root_path,
        branch:       &root_branch,
    };

    add_project_items_from_typed(
        items,
        &root_context,
        cargo.types(),
        cargo.examples(),
        cargo.benches(),
    );

    for group in ws.groups() {
        for member in group.members() {
            let member_cargo = &member.cargo;
            let member_display_path = member.display_path();
            let member_abs_path = member.path();
            let member_name = member.package_name().into_string();
            let member_context = TypedProjectContext {
                project_name: &member_name,
                cargo_name:   None,
                abs_path:     member_abs_path,
                display_path: member_display_path.as_str(),
                branch:       &root_branch,
            };
            add_project_items_from_typed(
                items,
                &member_context,
                member_cargo.types(),
                member_cargo.examples(),
                member_cargo.benches(),
            );
            for vendored in member.vendored() {
                add_vendored_items_typed(items, vendored, &member_name);
            }
        }
    }

    let ws_package_name = ws.package_name().into_string();
    for vendored in ws.vendored() {
        add_vendored_items_typed(items, vendored, &ws_package_name);
    }
}

fn add_package_items(items: &mut Vec<FinderItem>, pkg: &Package) {
    let root_path = pkg.display_path().into_string();
    let root_abs_path = pkg.path();
    let root_branch = branch_for(pkg.git_info());
    let cargo = &pkg.cargo;
    let root_name = pkg.root_directory_name().into_string();
    let pkg_name = pkg.package_name().into_string();
    let cargo_name = (pkg_name != root_name).then_some(pkg_name);
    let root_context = TypedProjectContext {
        project_name: &root_name,
        cargo_name:   cargo_name.as_deref(),
        abs_path:     root_abs_path,
        display_path: &root_path,
        branch:       &root_branch,
    };

    add_project_items_from_typed(
        items,
        &root_context,
        cargo.types(),
        cargo.examples(),
        cargo.benches(),
    );

    let pkg_parent_name = pkg.package_name().into_string();
    for vendored in pkg.vendored() {
        add_vendored_items_typed(items, vendored, &pkg_parent_name);
    }
}

fn add_vendored_items_typed(
    items: &mut Vec<FinderItem>,
    project: &VendoredPackage,
    parent_name: &str,
) {
    let project_name = project.package_name().into_string();
    let dir = project.display_path().into_string();
    let project_path: AbsolutePath = project.path().clone();
    let branch = String::new();
    let display_name = format!("{project_name} (vendored)");

    items.push(FinderItem {
        search_tokens: build_search_tokens(&[
            &display_name,
            &project_name,
            parent_name,
            &dir,
            "vendored",
            FinderKind::Project.label(),
        ]),
        display_name,
        kind: FinderKind::Project,
        project_path: project_path.clone(),
        target_name: None,
        parent_label: parent_name.to_string(),
        branch: branch.clone(),
        dir: dir.clone(),
        pr_target: None,
    });

    let cargo = &project.cargo;

    if cargo.types().contains(&ProjectType::Binary) {
        let kind = FinderKind::Binary;
        items.push(FinderItem {
            search_tokens: build_search_tokens(&[
                &project_name,
                &project_name,
                parent_name,
                &dir,
                "vendored",
                kind.label(),
            ]),
            display_name: project_name.clone(),
            kind,
            project_path: project_path.clone(),
            target_name: Some(project_name.clone()),
            parent_label: project_name.clone(),
            branch: branch.clone(),
            dir: dir.clone(),
            pr_target: None,
        });
    }

    for group in cargo.examples() {
        for name in &group.names {
            let display = if group.category.is_empty() {
                name.clone()
            } else {
                format!("{}/{name}", group.category)
            };
            let kind = FinderKind::Example;
            items.push(FinderItem {
                search_tokens: build_search_tokens(&[
                    &display,
                    &project_name,
                    parent_name,
                    &dir,
                    "vendored",
                    kind.label(),
                ]),
                display_name: display,
                kind,
                project_path: project_path.clone(),
                target_name: Some(name.clone()),
                parent_label: project_name.clone(),
                branch: branch.clone(),
                dir: dir.clone(),
                pr_target: None,
            });
        }
    }

    for name in cargo.benches() {
        let kind = FinderKind::Bench;
        items.push(FinderItem {
            search_tokens: build_search_tokens(&[
                name,
                &project_name,
                parent_name,
                &dir,
                "vendored",
                kind.label(),
            ]),
            display_name: name.clone(),
            kind,
            project_path: project_path.clone(),
            target_name: Some(name.clone()),
            parent_label: project_name.clone(),
            branch: branch.clone(),
            dir: dir.clone(),
            pr_target: None,
        });
    }
}

fn add_project_items_from_typed(
    items: &mut Vec<FinderItem>,
    context: &TypedProjectContext<'_>,
    types: &[ProjectType],
    examples: &[ExampleGroup],
    benches: &[String],
) {
    let project_name = context.project_name.to_string();
    let cargo_name = context.cargo_name.map(str::to_string);
    let branch = context.branch.to_string();
    let dir = context.display_path.to_string();

    // Build base token fields shared by all rows. Cargo name is included so
    // all targets remain findable by Cargo name when the directory differs.
    let base_fields: Vec<&str> = [&project_name as &str, &dir, &branch]
        .into_iter()
        .chain(cargo_name.as_deref())
        .collect();

    // The project itself
    let kind = FinderKind::Project;
    let mut project_tokens = base_fields.clone();
    project_tokens.push(kind.label());
    items.push(FinderItem {
        search_tokens: build_search_tokens(&project_tokens),
        display_name: project_name.clone(),
        kind,
        project_path: context.abs_path.into(),
        target_name: None,
        parent_label: String::new(),
        branch: branch.clone(),
        dir: dir.clone(),
        pr_target: None,
    });

    // Binary
    if types.contains(&ProjectType::Binary) {
        let kind = FinderKind::Binary;
        let mut tokens = base_fields.clone();
        tokens.push(kind.label());
        items.push(FinderItem {
            search_tokens: build_search_tokens(&tokens),
            display_name: project_name.clone(),
            kind,
            project_path: context.abs_path.into(),
            target_name: Some(project_name.clone()),
            parent_label: project_name.clone(),
            branch: branch.clone(),
            dir: dir.clone(),
            pr_target: None,
        });
    }

    // Examples (with category prefix)
    for group in examples {
        for name in &group.names {
            let display = if group.category.is_empty() {
                name.clone()
            } else {
                format!("{}/{name}", group.category)
            };
            let kind = FinderKind::Example;
            let mut tokens = vec![display.as_str()];
            tokens.extend_from_slice(&base_fields);
            tokens.push(kind.label());
            items.push(FinderItem {
                search_tokens: build_search_tokens(&tokens),
                display_name: display,
                kind,
                project_path: context.abs_path.into(),
                target_name: Some(name.clone()),
                parent_label: project_name.clone(),
                branch: branch.clone(),
                dir: dir.clone(),
                pr_target: None,
            });
        }
    }

    // Benches
    for name in benches {
        let kind = FinderKind::Bench;
        let mut tokens = vec![name.as_str()];
        tokens.extend_from_slice(&base_fields);
        tokens.push(kind.label());
        items.push(FinderItem {
            search_tokens: build_search_tokens(&tokens),
            display_name: name.clone(),
            kind,
            project_path: context.abs_path.into(),
            target_name: Some(name.clone()),
            parent_label: project_name.clone(),
            branch: branch.clone(),
            dir: dir.clone(),
            pr_target: None,
        });
    }
}

struct TypedProjectContext<'a> {
    project_name: &'a str,
    /// Cargo package name when it differs from `project_name`. Included in
    /// search tokens so root-level Rust items remain findable by Cargo name.
    cargo_name:   Option<&'a str>,
    abs_path:     &'a Path,
    display_path: &'a str,
    branch:       &'a str,
}

pub fn build_search_tokens(fields: &[&str]) -> Vec<String> {
    let mut tokens = Vec::new();
    for field in fields {
        for segment in field
            .split(|ch: char| ch.is_whitespace() || matches!(ch, '/' | '\\'))
            .filter(|segment| !segment.is_empty())
        {
            push_search_token(&mut tokens, segment);
            for fragment in segment.split(|ch: char| !ch.is_alphanumeric()) {
                push_search_token(&mut tokens, fragment);
            }
        }
    }
    tokens
}

fn push_search_token(tokens: &mut Vec<String>, token: &str) {
    if token.is_empty() || !token.chars().any(char::is_alphanumeric) {
        return;
    }
    if tokens.iter().any(|existing| existing == token) {
        return;
    }
    tokens.push(token.to_string());
}
