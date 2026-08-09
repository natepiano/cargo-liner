use crate::project::AbsolutePath;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::tui) enum CargoPortToastAction {
    OpenPath(AbsolutePath),
}

impl From<AbsolutePath> for CargoPortToastAction {
    fn from(path: AbsolutePath) -> Self { Self::OpenPath(path) }
}
