use std::path::Path;

use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::tui::app::App;
use crate::tui::project_list::VisibleRow;
use crate::tui::startup_services::StartupEffect;
use crate::tui::terminal;

/// The project a priority fetch loads: the path its results are written
/// back to, and the crates.io name it queries for. Both name one project,
/// so a `BackgroundMsg::CratesIoVersion` can never store one crate's
/// version on another crate's row.
struct PriorityFetchTarget {
    path:           AbsolutePath,
    crates_io_name: Option<String>,
}

impl App {
    pub(super) fn detail_path_is_affected(&self, path: &Path) -> bool {
        let Some(selected_path) = self.project_list.selected_project_path() else {
            return false;
        };
        if selected_path == path {
            return true;
        }
        if self.selected_worktree_group_contains(path) {
            return true;
        }
        // Check if both paths resolve to the same lint-owning node.
        // This covers shared owner rows such as workspace members
        // without widening unrelated watcher invalidations.
        self.project_list
            .lint_at_path(selected_path)
            .zip(self.project_list.lint_at_path(path))
            .is_some_and(|(a, b)| std::ptr::eq(a, b))
    }

    fn selected_worktree_group_contains(&self, path: &Path) -> bool {
        let Some(VisibleRow::Root { node_index }) = self.project_list.selected_row() else {
            return false;
        };
        let Some(entry) = self.project_list.get(node_index) else {
            return false;
        };
        let RootItem::Worktrees(group) = &entry.root_item else {
            return false;
        };
        group
            .iter_paths()
            .any(|group_path| group_path.as_path() == path)
    }

    /// The selected project paired with the crates.io name to query for it.
    /// Both come from the project list at one path. The detail pane is not
    /// a source here: it keeps rendering the previously selected row until
    /// the next draw, and this runs from the background poll on a tree
    /// rebuild, so a pane-sourced name queries one crate and stores its
    /// version on another crate's row.
    fn priority_fetch_target(&self) -> Option<PriorityFetchTarget> {
        let path: AbsolutePath = self.project_list.selected_project_path()?.into();
        let crates_io_name = self
            .collect_crates_io_fetch_plan()
            .name_for_path(path.as_path())
            .map(String::from);
        Some(PriorityFetchTarget {
            path,
            crates_io_name,
        })
    }

    /// Spawn a priority fetch for the selected project if it hasn't been loaded yet.
    pub(in crate::tui) fn maybe_priority_fetch(&mut self) {
        let Some(target) = self.priority_fetch_target() else {
            return;
        };
        let abs_key = target.path;
        let display_path = self
            .selected_display_path()
            .unwrap_or_else(|| abs_key.display_path());
        if self
            .project_list
            .at_path(abs_key.as_path())
            .is_none_or(|p| p.disk_usage_bytes.is_none())
            && self.scan.priority_fetch_path() != Some(&abs_key)
        {
            let effect = self.startup_services.priority_detail_fetch_effect();
            self.startup_services.record_priority_detail_fetch(effect);
            if effect == StartupEffect::Suppressed {
                return;
            }
            let abs_str = abs_key.as_path().display().to_string();
            self.scan.set_priority_fetch_path(Some(abs_key));
            terminal::spawn_priority_fetch(
                self,
                display_path.as_str(),
                &abs_str,
                target.crates_io_name.as_ref(),
            );
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::project::Package;
    use crate::project::RustProject;
    use crate::tui::test_support;

    fn package(name: &str, path: &str) -> RootItem {
        RootItem::Rust(RustProject::Package(Package {
            path: AbsolutePath::from(path),
            name: Some(name.to_string()),
            ..Package::default()
        }))
    }

    /// The name a priority fetch queries and the path it writes the answer
    /// to must name one project. The detail pane keeps rendering the
    /// previously selected row until the next draw, so sourcing the name
    /// from it stores one crate's crates.io version on another crate's row
    /// — which then reads as a release the next background refresh
    /// "discovers".
    #[test]
    fn priority_fetch_names_the_selected_project_not_the_stale_detail_pane() {
        let mut app = test_support::make_app(&[
            package("alpha_crate", "/tmp/alpha_crate"),
            package("beta_crate", "/tmp/beta_crate"),
        ]);

        app.project_list.select_root_row(0);
        app.ensure_detail_cached();
        assert_eq!(
            app.panes.package.content().map(|d| d.name.as_str()),
            Some("alpha_crate"),
            "the pane holds the first row's package before the selection moves"
        );

        app.project_list.select_root_row(1);

        let target = app.priority_fetch_target().expect("a row is selected");
        assert_eq!(target.path.as_path(), Path::new("/tmp/beta_crate"));
        assert_eq!(
            target.crates_io_name.as_deref(),
            Some("beta_crate"),
            "the queried name follows the selected path, not the unredrawn pane"
        );
    }
}
