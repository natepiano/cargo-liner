use std::collections::HashMap;
use std::path::Path;

use crate::project::AbsolutePath;
use crate::tui::project_list_state::ProjectList;

/// Last accepted metadata outcome for one project's `Cargo.toml`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(in crate::tui) enum ProjectMetadataState {
    #[default]
    NotLoaded,
    Available,
    Failed {
        first_error_line: String,
    },
}

impl ProjectMetadataState {
    pub(in crate::tui) const fn name_marker(&self) -> &'static str {
        match self {
            Self::Failed { .. } => " !",
            Self::NotLoaded | Self::Available => "",
        }
    }
}

/// Per-manifest outcomes owned by `Scan`, independently of replaceable rows.
/// Each project root identifies its single `Cargo.toml`. An outcome remains
/// through rescans while the root remains a Rust project, until another
/// accepted metadata reply replaces it.
#[derive(Default)]
pub(in crate::tui) struct ManifestMetadataStates {
    by_project_root: HashMap<AbsolutePath, ProjectMetadataState>,
}

impl ManifestMetadataStates {
    pub(in crate::tui) fn retain_rust_projects(&mut self, projects: &ProjectList) {
        self.by_project_root
            .retain(|root, _| projects.is_rust_at_path(root));
    }

    pub(in crate::tui) fn for_project(&self, root: &Path) -> &ProjectMetadataState {
        self.by_project_root
            .get(root)
            .unwrap_or(&ProjectMetadataState::NotLoaded)
    }

    pub(in crate::tui) fn record_failure(&mut self, root: &AbsolutePath, message: &str) {
        self.by_project_root.insert(
            root.clone(),
            ProjectMetadataState::Failed {
                first_error_line: message.to_string(),
            },
        );
    }

    pub(in crate::tui) fn record_success(&mut self, root: &AbsolutePath) {
        self.by_project_root
            .insert(root.clone(), ProjectMetadataState::Available);
    }
}
