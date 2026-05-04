use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use crate::project::AbsolutePath;
use crate::project::GitStatus;
use crate::project::Package;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::Workspace;
use crate::project::WorktreeGroup;
use crate::tui::app::App;
use crate::tui::app::types::DiscoveryRowKind;
use crate::tui::app::types::DiscoveryShimmer;
use crate::tui::columns;

impl App {
    pub fn animation_elapsed(&self) -> Duration { self.animation_started.elapsed() }

    pub fn register_discovery_shimmer(&mut self, path: &Path) {
        if !self.is_scan_complete() || !self.config().discovery_shimmer_enabled() {
            return;
        }
        let shimmer =
            DiscoveryShimmer::new(Instant::now(), self.config().discovery_shimmer_duration());
        self.scan
            .discovery_shimmers_mut()
            .insert(AbsolutePath::from(path), shimmer);
    }

    pub fn prune_discovery_shimmers(&mut self, now: Instant) {
        self.scan
            .discovery_shimmers_mut()
            .retain(|_, shimmer| shimmer.is_active_at(now));
    }

    pub fn discovery_name_segments_for_path(
        &self,
        row_path: &Path,
        name: &str,
        git_status: Option<GitStatus>,
        row_kind: DiscoveryRowKind,
    ) -> Option<Vec<columns::StyledSegment>> {
        if !self.config().discovery_shimmer_enabled() {
            return None;
        }
        let now = Instant::now();
        let (session_path, shimmer) =
            self.discovery_shimmer_session_for_path(row_path, now, row_kind)?;
        let char_count = name.chars().count();
        if char_count == 0 {
            return None;
        }

        let base_style = columns::project_name_style(git_status);
        let accent_style = columns::project_name_shimmer_style(git_status);
        let window = discovery_shimmer_window_len(char_count);
        let elapsed_ms = usize::try_from(now.duration_since(shimmer.started_at).as_millis())
            .unwrap_or(usize::MAX);
        let step = elapsed_ms / discovery_shimmer_step_millis();
        let head = (step
            + discovery_shimmer_phase_offset(
                session_path.as_path(),
                row_path,
                row_kind,
                char_count,
            ))
            % char_count;

        Some(columns::build_shimmer_segments(
            name,
            base_style,
            accent_style,
            head,
            window,
        ))
    }

    fn discovery_shimmer_session_for_path(
        &self,
        row_path: &Path,
        now: Instant,
        row_kind: DiscoveryRowKind,
    ) -> Option<(AbsolutePath, DiscoveryShimmer)> {
        self.scan
            .discovery_shimmers()
            .iter()
            .filter(|(session_path, shimmer)| {
                shimmer.is_active_at(now)
                    && self.discovery_shimmer_session_matches(
                        session_path.as_path(),
                        row_path,
                        row_kind,
                    )
            })
            .max_by_key(|(_, shimmer)| shimmer.started_at)
            .map(|(session_path, shimmer)| (session_path.clone(), *shimmer))
    }

    fn discovery_shimmer_session_matches(
        &self,
        session_path: &Path,
        row_path: &Path,
        row_kind: DiscoveryRowKind,
    ) -> bool {
        self.discovery_scope_contains(session_path, row_path)
            || self
                .discovery_parent_row(session_path)
                .is_some_and(|parent| {
                    parent.path.as_path() == row_path && row_kind.allows_parent_kind(parent.kind)
                })
    }

    fn discovery_scope_contains(&self, session_path: &Path, row_path: &Path) -> bool {
        self.projects()
            .iter()
            .any(|item| root_item_scope_contains(item, session_path, row_path))
    }

    fn discovery_parent_row(&self, session_path: &Path) -> Option<DiscoveryParentRow> {
        self.projects()
            .iter()
            .find_map(|item| root_item_parent_row(item, session_path))
    }
}

const fn discovery_shimmer_window_len(char_count: usize) -> usize {
    match char_count {
        0 => 0,
        1..=2 => 1,
        3..=5 => 2,
        6..=8 => 3,
        _ => 4,
    }
}

const fn discovery_shimmer_step_millis() -> usize { 85 }

fn discovery_shimmer_phase_offset(
    session_path: &Path,
    row_path: &Path,
    row_kind: DiscoveryRowKind,
    char_count: usize,
) -> usize {
    if char_count == 0 {
        return 0;
    }
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let key = format!(
        "{}|{}|{}",
        session_path.to_string_lossy(),
        row_path.to_string_lossy(),
        row_kind.discriminant()
    );
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    usize::try_from(hash % u64::try_from(char_count).unwrap_or(1)).unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoveryParentRow {
    path: AbsolutePath,
    kind: DiscoveryRowKind,
}

impl DiscoveryRowKind {
    const fn allows_parent_kind(self, kind: Self) -> bool {
        matches!(
            (self, kind),
            (Self::Root, Self::Root)
                | (Self::WorktreeEntry, Self::WorktreeEntry)
                | (Self::PathOnly, Self::PathOnly)
        )
    }

    const fn discriminant(self) -> u8 {
        match self {
            Self::Root => 0,
            Self::WorktreeEntry => 1,
            Self::PathOnly => 2,
        }
    }
}

fn package_contains_path(pkg: &Package, row_path: &Path) -> bool {
    pkg.path() == row_path
        || pkg
            .vendored()
            .iter()
            .any(|vendored| vendored.path() == row_path)
}

fn workspace_contains_path(ws: &Workspace, row_path: &Path) -> bool {
    ws.path() == row_path
        || ws.groups().iter().any(|group| {
            group
                .members()
                .iter()
                .any(|member| package_contains_path(member, row_path))
        })
        || ws
            .vendored()
            .iter()
            .any(|vendored| vendored.path() == row_path)
}

fn root_item_scope_contains(item: &RootItem, session_path: &Path, row_path: &Path) -> bool {
    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            workspace_scope_contains(ws, session_path, row_path)
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            package_scope_contains(pkg, session_path, row_path)
        },
        RootItem::NonRust(project) => project.path() == session_path && project.path() == row_path,
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            workspace_scope_contains(primary, session_path, row_path)
                || linked
                    .iter()
                    .any(|l| workspace_scope_contains(l, session_path, row_path))
        },
        RootItem::Worktrees(WorktreeGroup::Packages {
            primary, linked, ..
        }) => {
            package_scope_contains(primary, session_path, row_path)
                || linked
                    .iter()
                    .any(|l| package_scope_contains(l, session_path, row_path))
        },
    }
}

fn workspace_scope_contains(ws: &Workspace, session_path: &Path, row_path: &Path) -> bool {
    if ws.path() == session_path {
        return workspace_contains_path(ws, row_path);
    }
    if ws
        .vendored()
        .iter()
        .any(|vendored| vendored.path() == session_path && vendored.path() == row_path)
    {
        return true;
    }
    ws.groups().iter().any(|group| {
        group
            .members()
            .iter()
            .any(|member| package_scope_contains(member, session_path, row_path))
    })
}

fn package_scope_contains(pkg: &Package, session_path: &Path, row_path: &Path) -> bool {
    if pkg.path() == session_path {
        return package_contains_path(pkg, row_path);
    }
    pkg.vendored()
        .iter()
        .any(|vendored| vendored.path() == session_path && vendored.path() == row_path)
}

fn root_item_parent_row(item: &RootItem, session_path: &Path) -> Option<DiscoveryParentRow> {
    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            workspace_parent_row(ws, session_path, DiscoveryRowKind::Root)
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            package_parent_row(pkg, session_path, DiscoveryRowKind::Root)
        },
        RootItem::NonRust(_) => None,
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            if primary.path() == session_path {
                return None;
            }
            if linked.iter().any(|l| l.path() == session_path) {
                return Some(DiscoveryParentRow {
                    path: primary.path().clone(),
                    kind: DiscoveryRowKind::Root,
                });
            }
            workspace_parent_row(primary, session_path, DiscoveryRowKind::WorktreeEntry).or_else(
                || {
                    linked.iter().find_map(|l| {
                        workspace_parent_row(l, session_path, DiscoveryRowKind::WorktreeEntry)
                    })
                },
            )
        },
        RootItem::Worktrees(WorktreeGroup::Packages {
            primary, linked, ..
        }) => {
            if primary.path() == session_path {
                return None;
            }
            if linked.iter().any(|l| l.path() == session_path) {
                return Some(DiscoveryParentRow {
                    path: primary.path().clone(),
                    kind: DiscoveryRowKind::Root,
                });
            }
            package_parent_row(primary, session_path, DiscoveryRowKind::WorktreeEntry).or_else(
                || {
                    linked.iter().find_map(|l| {
                        package_parent_row(l, session_path, DiscoveryRowKind::WorktreeEntry)
                    })
                },
            )
        },
    }
}

fn workspace_parent_row(
    ws: &Workspace,
    session_path: &Path,
    parent_kind: DiscoveryRowKind,
) -> Option<DiscoveryParentRow> {
    if ws.path() == session_path {
        return None;
    }
    if ws
        .vendored()
        .iter()
        .any(|vendored| vendored.path() == session_path)
    {
        return Some(DiscoveryParentRow {
            path: ws.path().clone(),
            kind: parent_kind,
        });
    }
    for group in ws.groups() {
        for member in group.members() {
            if member.path() == session_path {
                return Some(DiscoveryParentRow {
                    path: ws.path().clone(),
                    kind: parent_kind,
                });
            }
            if let Some(parent) =
                package_parent_row(member, session_path, DiscoveryRowKind::PathOnly)
            {
                return Some(parent);
            }
        }
    }
    None
}

fn package_parent_row(
    pkg: &Package,
    session_path: &Path,
    parent_kind: DiscoveryRowKind,
) -> Option<DiscoveryParentRow> {
    if pkg.path() == session_path {
        return None;
    }
    pkg.vendored()
        .iter()
        .any(|vendored| vendored.path() == session_path)
        .then(|| DiscoveryParentRow {
            path: pkg.path().clone(),
            kind: parent_kind,
        })
}
