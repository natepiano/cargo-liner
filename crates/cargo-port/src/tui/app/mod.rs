mod async_tasks;
mod ci_state;
mod construct;
mod dismiss;
mod focus;
mod lint;
mod navigation;
mod query;
mod snapshots;
mod types;

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests;

pub(super) use dismiss::DismissTarget;
pub(super) use types::App;
pub(super) use types::CiState;
pub(super) use types::ConfirmAction;
pub(super) use types::DiscoveryRowKind;
pub(super) use types::ExpandKey;
pub(super) use types::PendingClean;
pub(super) use types::PollBackgroundStats;
#[cfg(test)]
pub(super) use types::SearchHit;
#[cfg(test)]
pub(super) use types::SearchMode;
pub(super) use types::VisibleRow;

pub(super) use super::columns::ResolvedWidths;

impl App {
    pub(super) fn dismiss_target_for_row(&self, row: VisibleRow) -> Option<DismissTarget> {
        self.dismiss_target_for_row_inner(row)
    }
}
