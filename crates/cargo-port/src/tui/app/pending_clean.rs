use crate::project::AbsolutePath;

#[derive(Clone)]
pub(crate) struct PendingClean {
    pub(in crate::tui) abs_path: AbsolutePath,
}
