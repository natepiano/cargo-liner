//! Startup-time helpers that produce seed paths for background
//! fetches. App's startup orchestration
//! (`schedule_startup_metadata`, `schedule_startup_disk_usage`)
//! calls them once at boot. `StartupPlanEffects` records which startup
//! work runs for real when App builds its startup plan.

use std::collections::HashSet;

use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::tui::project_list_state::ProjectList;
use crate::tui::startup_services::StartupEffect;
use crate::tui::startup_services::StartupServices;

/// Which startup work runs for real, and which the startup services
/// suppress, when App builds its startup plan.
#[derive(Clone, Copy)]
pub(super) struct StartupPlanEffects {
    pub(super) streaming_scan:  StartupEffect,
    pub(super) project_details: StartupEffect,
    pub(super) lint_history:    StartupEffect,
}

impl StartupPlanEffects {
    pub(super) const fn from_startup_services(startup_services: &StartupServices) -> Self {
        Self {
            streaming_scan:  startup_services.streaming_scan_effect(),
            project_details: startup_services.startup_project_details_effect(),
            lint_history:    startup_services.lint_history_hydration_effect(),
        }
    }

    #[cfg(test)]
    pub(super) const fn production() -> Self {
        Self {
            streaming_scan:  StartupEffect::Real,
            project_details: StartupEffect::Real,
            lint_history:    StartupEffect::Real,
        }
    }
}

/// Workspace roots that should receive an initial `cargo metadata`
/// dispatch: every Rust leaf project (workspace or standalone
/// package), including each worktree in a group. Non-Rust
/// projects are skipped.
pub(super) fn initial_metadata_roots(projects: &ProjectList) -> HashSet<AbsolutePath> {
    let mut roots = HashSet::new();
    projects.for_each_leaf(|entry| {
        if let RootItem::Rust(rust) = &entry.root_item {
            roots.insert(rust.path().clone());
        }
    });
    roots
}

/// Top-level paths for the initial disk-usage scan. Nested
/// projects are folded under their nearest ancestor so the
/// background scanner doesn't double-walk a tree.
pub(super) fn initial_disk_roots(projects: &ProjectList) -> HashSet<AbsolutePath> {
    let mut abs_paths: Vec<&AbsolutePath> = projects
        .iter()
        .map(|entry| entry.root_item.path())
        .collect();
    abs_paths.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });

    let mut roots: Vec<&AbsolutePath> = Vec::new();
    for abs_path in abs_paths {
        if roots.iter().any(|root| abs_path.starts_with(root)) {
            continue;
        }
        roots.push(abs_path);
    }

    roots.into_iter().cloned().collect()
}
