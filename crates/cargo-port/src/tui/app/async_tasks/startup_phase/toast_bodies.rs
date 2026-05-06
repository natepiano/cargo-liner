use std::collections::HashSet;
use std::time::Instant;

use crate::project;
use crate::project::AbsolutePath;
use crate::tui::app::App;
use crate::tui::app::Startup;
use crate::tui::toasts;
use crate::tui::toasts::TrackedItem;

impl Startup {
    pub(super) fn disk_toast_body(&self) -> String {
        let empty = HashSet::new();
        let expected = self.disk.expected.as_ref().unwrap_or(&empty);
        remaining_toast_body(expected, &self.disk.seen)
    }
    pub(super) fn git_toast_body(&self) -> String {
        let empty = HashSet::new();
        let expected = self.git.expected.as_ref().unwrap_or(&empty);
        remaining_toast_body(expected, &self.git.seen)
    }
    pub(super) fn metadata_toast_body(&self) -> String {
        let empty = HashSet::new();
        let expected = self.metadata.expected.as_ref().unwrap_or(&empty);
        remaining_toast_body(expected, &self.metadata.seen)
    }
}

impl App {
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
    #[cfg(test)]
    pub fn startup_lint_toast_body_for(
        expected: &HashSet<AbsolutePath>,
        seen: &HashSet<AbsolutePath>,
    ) -> String {
        remaining_toast_body(expected, seen)
    }
}

pub(super) fn remaining_toast_body(
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
