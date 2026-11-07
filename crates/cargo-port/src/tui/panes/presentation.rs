/// Whether the selected project still exists on disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::tui) enum ProjectPresentation {
    #[default]
    Current,
    Deleted,
}
