use crate::project::AbsolutePath;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CargoPortToastAction {
    OpenPath(AbsolutePath),
}

impl From<AbsolutePath> for CargoPortToastAction {
    fn from(path: AbsolutePath) -> Self { Self::OpenPath(path) }
}
