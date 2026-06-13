use crate::project::AbsolutePath;

#[derive(Clone)]
pub(crate) struct PendingClean {
    pub(crate) abs_path: AbsolutePath,
}
