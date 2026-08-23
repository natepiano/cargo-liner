//! App-facing access to the accepted Cargo workspace index.

use std::sync::Arc;

use super::app::App;
use crate::project::CargoWorkspaceIndex;
use crate::project::CargoWorkspaceIndexRevisionState;
use crate::project::WorkspaceIndexRebuild;

/// Readiness of the App-owned workspace index for one consumer decision.
///
/// `RetainedLastAccepted` means the metadata store was unavailable for this
/// decision, so consumers keep using the immutable accepted index instead of
/// replacing it with incomplete ownership data.
///
/// The payload variants borrow the App-owned handle rather than owning an
/// [`Arc`] clone: owning one would drop `Copy`, and callers pass the same
/// readiness value more than once. A consumer that needs the index across a
/// thread boundary clones the handle explicitly at that call site.
#[derive(Clone, Copy, Debug)]
pub(super) enum WorkspaceIndexReadiness<'a> {
    Current {
        cargo_workspace_index: &'a Arc<CargoWorkspaceIndex>,
    },
    RetainedLastAccepted {
        cargo_workspace_index: &'a Arc<CargoWorkspaceIndex>,
    },
    Uninitialized,
}

impl App {
    /// Refresh the immutable index only when the metadata store is available,
    /// then expose the current or retained accepted index to one consumer.
    pub(super) fn workspace_index_readiness(&mut self) -> WorkspaceIndexReadiness<'_> {
        {
            let Ok(metadata_store) = self.scan.metadata_store().lock() else {
                return self.retained_workspace_index_readiness();
            };
            if let WorkspaceIndexRebuild::Replaced(cargo_workspace_index) = self
                .cargo_workspace_index
                .rebuild_if_changed(&metadata_store, self.project_list.revision())
            {
                self.cargo_workspace_index = cargo_workspace_index;
            }
        }
        WorkspaceIndexReadiness::Current {
            cargo_workspace_index: &self.cargo_workspace_index,
        }
    }

    /// Readiness reported when the metadata store could not be locked for this
    /// decision, so no rebuild was attempted.
    fn retained_workspace_index_readiness(&self) -> WorkspaceIndexReadiness<'_> {
        match self.cargo_workspace_index.revision() {
            CargoWorkspaceIndexRevisionState::Accepted(_) => {
                WorkspaceIndexReadiness::RetainedLastAccepted {
                    cargo_workspace_index: &self.cargo_workspace_index,
                }
            },
            CargoWorkspaceIndexRevisionState::Uninitialized => {
                WorkspaceIndexReadiness::Uninitialized
            },
        }
    }
}
