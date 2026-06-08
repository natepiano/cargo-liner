use tui_pane::RunningTracker;
use tui_pane::ToastTaskId;

use crate::ci::OwnerRepo;

/// The standalone GitHub / crates.io running-toast slots. One sticky toast
/// per service, created only in steady state. This value exists exclusively
/// inside `NetworkToastStage::SteadyState`: during startup the consolidated
/// panel owns those rows, so there is no slot here to populate and the
/// standalone toast cannot be created.
#[derive(Default)]
pub struct NetworkRunningToasts {
    /// "Fetching crates.io info" toast slot.
    pub crates_io: Option<ToastTaskId>,
    /// "Retrieving GitHub repo details" toast slot.
    pub github:    Option<ToastTaskId>,
}

#[derive(Default)]
pub(super) struct NetworkRunningTrackers {
    pub(super) github:    RunningTracker<OwnerRepo>,
    pub(super) crates_io: RunningTracker<String>,
}

#[derive(Default)]
pub(super) struct SteadyStateNetworkToasts {
    pub(super) running: NetworkRunningTrackers,
    pub(super) toasts:  NetworkRunningToasts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StartupServiceExit {
    Drained,
    Abandoned,
}

/// Count of startup-owned network items still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupNetworkPending {
    pub github:    usize,
    pub crates_io: usize,
}

/// Proof that startup-owned network work is terminal and can hand off to
/// steady state. Its fields are private so callers cannot manufacture it
/// without going through `Net::startup_network_readiness`.
#[derive(Debug, PartialEq, Eq)]
pub struct StartupNetworkReady {
    pub(super) github:    StartupServiceExit,
    pub(super) crates_io: StartupServiceExit,
}

/// Whether the startup-owned network trackers may hand off to steady state.
#[derive(Debug, PartialEq, Eq)]
pub enum StartupNetworkReadiness {
    Ready(StartupNetworkReady),
    Pending(StartupNetworkPending),
}
