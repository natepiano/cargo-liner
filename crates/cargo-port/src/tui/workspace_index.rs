//! App-facing access to the accepted Cargo workspace index.

use crate::project::CargoWorkspaceIndex;
use crate::project::CargoWorkspaceIndexRevisionState;
use crate::tui::app::App;

/// Readiness of the App-owned workspace index for one consumer decision.
///
/// `RetainedLastAccepted` means the metadata store was unavailable for this
/// decision, so consumers keep using the immutable accepted index instead of
/// replacing it with incomplete ownership data.
#[derive(Clone, Copy, Debug)]
pub(crate) enum WorkspaceIndexReadiness<'a> {
    Current {
        cargo_workspace_index: &'a CargoWorkspaceIndex,
    },
    RetainedLastAccepted {
        cargo_workspace_index: &'a CargoWorkspaceIndex,
    },
    Uninitialized,
}

impl App {
    /// Refresh the immutable index only when the metadata store is available,
    /// then expose the current or retained accepted index to one consumer.
    pub(crate) fn workspace_index_readiness(&mut self) -> WorkspaceIndexReadiness<'_> {
        {
            let Ok(metadata_store) = self.scan.metadata_store().lock() else {
                return self.retained_workspace_index_readiness();
            };
            self.cargo_workspace_index
                .rebuild_if_changed(&metadata_store, self.project_list.revision());
        }
        WorkspaceIndexReadiness::Current {
            cargo_workspace_index: &self.cargo_workspace_index,
        }
    }

    /// Readiness reported when the metadata store could not be locked for this
    /// decision, so no rebuild was attempted.
    const fn retained_workspace_index_readiness(&self) -> WorkspaceIndexReadiness<'_> {
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
