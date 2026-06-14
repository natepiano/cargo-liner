use super::AbsolutePath;
use super::LintConfig;
use super::Path;
use super::ProjectLanguage;
use super::supervisor;

#[derive(Clone)]
pub struct RegisterProjectRequest {
    pub project_label:       String,
    pub abs_path:            AbsolutePath,
    pub project_language:    ProjectLanguage,
    pub linked_primary_root: Option<AbsolutePath>,
}

impl RegisterProjectRequest {
    pub fn new(
        project_label: impl Into<String>,
        abs_path: AbsolutePath,
        project_language: impl Into<ProjectLanguage>,
    ) -> Self {
        Self {
            project_label: project_label.into(),
            abs_path,
            project_language: project_language.into(),
            linked_primary_root: None,
        }
    }

    pub const fn is_rust(&self) -> bool { self.project_language.is_rust() }

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
    supervisor::should_watch_project(
        lint,
        &RegisterProjectRequest::new(project_label, AbsolutePath::from(abs_path), is_rust),
    )
}
