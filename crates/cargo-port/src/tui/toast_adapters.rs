use tui_pane::TrackedItemKey;

use crate::ci::OwnerRepo;
use crate::project::AbsolutePath;

impl From<AbsolutePath> for TrackedItemKey {
    fn from(value: AbsolutePath) -> Self { Self::new(value.to_string()) }
}

impl From<&AbsolutePath> for TrackedItemKey {
    fn from(value: &AbsolutePath) -> Self { Self::new(value.to_string()) }
}

impl From<OwnerRepo> for TrackedItemKey {
    fn from(value: OwnerRepo) -> Self { Self::new(value.to_string()) }
}

impl From<&OwnerRepo> for TrackedItemKey {
    fn from(value: &OwnerRepo) -> Self { Self::new(value.to_string()) }
}
