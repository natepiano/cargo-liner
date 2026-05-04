use std::path::Path;

use crate::project::ProjectCiData;
use crate::tui::app::App;

impl App {
    pub(super) fn ci_is_exhausted(&self, path: &Path) -> bool {
        self.projects()
            .ci_data_for(path)
            .is_some_and(ProjectCiData::is_exhausted)
    }
}
