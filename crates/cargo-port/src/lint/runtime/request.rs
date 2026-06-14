use super::AbsolutePath;
use super::LintConfig;
use super::Path;
use super::supervisor;

#[derive(Clone)]
pub struct RegisterProjectRequest {
    pub project_label:       String,
    pub abs_path:            AbsolutePath,
    pub linked_primary_root: Option<AbsolutePath>,
}

impl RegisterProjectRequest {
    pub fn new(project_label: impl Into<String>, abs_path: AbsolutePath) -> Self {
        Self {
            project_label: project_label.into(),
            abs_path,
            linked_primary_root: None,
        }
    }

    pub fn with_linked_primary_root(mut self, primary_root: Option<AbsolutePath>) -> Self {
        self.linked_primary_root = primary_root;
        self
    }
}

pub fn project_is_eligible(
    lint: &LintConfig,
    project_label: &str,
    abs_path: &Path,
    is_rust: bool,
) -> bool {
    if !is_rust {
        return false;
    }
    supervisor::should_watch_project(
        lint,
        &RegisterProjectRequest::new(project_label, AbsolutePath::from(abs_path)),
    )
}
