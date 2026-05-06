use std::collections::HashSet;

use crate::project;
use crate::project::AbsolutePath;
use crate::tui::app::Startup;
use crate::tui::toasts;

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
