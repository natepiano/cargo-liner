use std::collections::HashSet;
use std::time::Instant;

use crate::project;
use crate::project::AbsolutePath;
use crate::tui::app::App;
use crate::tui::toasts;
use crate::tui::toasts::TrackedItem;

impl App {
    pub(super) fn startup_disk_toast_body(&self) -> String {
        let empty = HashSet::new();
        let expected = self.startup.disk.expected.as_ref().unwrap_or(&empty);
        Self::startup_remaining_toast_body(expected, &self.startup.disk.seen)
    }
    pub(super) fn startup_git_toast_body(&self) -> String {
        let empty = HashSet::new();
        let expected = self.startup.git.expected.as_ref().unwrap_or(&empty);
        Self::startup_remaining_toast_body(expected, &self.startup.git.seen)
    }
    pub(super) fn startup_metadata_toast_body(&self) -> String {
        let empty = HashSet::new();
        let expected = self.startup.metadata.expected.as_ref().unwrap_or(&empty);
        Self::startup_remaining_toast_body(expected, &self.startup.metadata.seen)
    }
    /// Build tracked items from expected/seen path sets. Already-seen paths
    /// are pre-marked as completed so the renderer shows them with strikethrough.
    /// Pending items get `started_at = now` so they render with a live
    /// spinner + ticking duration that freezes when the item completes —
    /// matching the GitHub repo-fetch toast.
    pub(super) fn tracked_items_for_startup(
        expected: &HashSet<AbsolutePath>,
        seen: &HashSet<AbsolutePath>,
    ) -> Vec<TrackedItem> {
        let now = Instant::now();
        expected
            .iter()
            .map(|path| {
                let label = project::home_relative_path(path);
                let is_seen = seen.contains(path);
                TrackedItem {
                    label,
                    key: path.into(),
                    started_at: if is_seen { None } else { Some(now) },
                    completed_at: if is_seen { Some(now) } else { None },
                }
            })
            .collect()
    }
    pub(super) fn startup_remaining_toast_body(
        expected: &HashSet<AbsolutePath>,
        seen: &HashSet<AbsolutePath>,
    ) -> String {
        let items: Vec<String> = expected
            .iter()
            .filter(|path| !seen.contains(*path))
            .map(|p| project::home_relative_path(p))
            .collect();
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        if refs.is_empty() {
            return "Complete".to_string();
        }
        toasts::format_toast_items(&refs, toasts::toast_body_width())
    }
    #[cfg(test)]
    pub fn startup_lint_toast_body_for(
        expected: &HashSet<AbsolutePath>,
        seen: &HashSet<AbsolutePath>,
    ) -> String {
        let items: Vec<String> = expected
            .iter()
            .filter(|path| !seen.contains(*path))
            .map(|p| project::home_relative_path(p))
            .collect();
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        if refs.is_empty() {
            return "Complete".to_string();
        }
        toasts::format_toast_items(&refs, toasts::toast_body_width())
    }
}
