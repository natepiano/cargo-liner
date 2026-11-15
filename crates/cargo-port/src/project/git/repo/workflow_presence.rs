use std::path::Path;

use serde::Serialize;

/// Whether `.github/workflows/` contains any `.yml` or `.yaml` files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) enum WorkflowPresence {
    /// At least one workflow YAML file exists.
    Present,
    /// No workflow files found (or no `.github/workflows/` directory).
    #[default]
    Missing,
}

impl WorkflowPresence {
    pub const fn is_present(self) -> bool { matches!(self, Self::Present) }
}

pub(super) fn get_workflow_presence(repo_root: &Path) -> WorkflowPresence {
    let workflows_dir = repo_root.join(".github").join("workflows");
    let has_yaml = std::fs::read_dir(workflows_dir).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".yml") || name.ends_with(".yaml")
        })
    });
    if has_yaml {
        WorkflowPresence::Present
    } else {
        WorkflowPresence::Missing
    }
}
