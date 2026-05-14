use tui_pane::ToastId;

use crate::project::AbsolutePath;

/// Identifies what is being dismissed by a `GlobalAction::Dismiss`.
#[derive(Clone, Debug)]
pub enum DismissTarget {
    Toast(ToastId),
    DeletedProject(AbsolutePath),
}
