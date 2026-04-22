use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Instant;

use crate::ci::OwnerRepo;
use crate::scan;
use crate::scan::RepoCache;
use crate::tui::toasts::ToastTaskId;

/// Three-way availability for a single service. `Unreachable` means
/// the network layer can't talk to the service at all; `RateLimited`
/// means the service is reachable but refusing our requests for quota
/// reasons. Recovery, display text, and toast copy all diverge
/// between the two — hence the explicit enum.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AvailabilityStatus {
    #[default]
    Reachable,
    Unreachable,
    RateLimited,
}

impl AvailabilityStatus {
    pub const fn is_available(self) -> bool { matches!(self, Self::Reachable) }
}

pub(super) struct ServiceAvailability {
    status:            AvailabilityStatus,
    retry_active:      bool,
    unavailable_toast: Option<u64>,
}

impl ServiceAvailability {
    pub(super) const fn new() -> Self {
        Self {
            status:            AvailabilityStatus::Reachable,
            retry_active:      false,
            unavailable_toast: None,
        }
    }

    pub(super) const fn status(&self) -> AvailabilityStatus { self.status }

    #[cfg(test)]
    pub(super) const fn is_unavailable(&self) -> bool { !self.status.is_available() }

    /// Mark the service reachable. Returns the tracked toast id iff
    /// this call is the transition out of an unavailable state —
    /// caller should dismiss that toast and fire the recovery message.
    /// Subsequent `Reachable` signals while already reachable return
    /// `None`, so the recovery toast only fires once per outage.
    pub(super) const fn mark_reachable(&mut self) -> Option<u64> {
        let was_unavailable = !matches!(self.status, AvailabilityStatus::Reachable);
        self.status = AvailabilityStatus::Reachable;
        if was_unavailable {
            self.retry_active = false;
            self.unavailable_toast.take()
        } else {
            None
        }
    }

    /// Marks the service unreachable (network failure). Returns `true`
    /// iff `retry_active` transitioned from false to true — caller
    /// spawns the retry loop. Subsequent `Unreachable`/`RateLimited`
    /// signals while a retry is already running return `false` so the
    /// loop is not respawned. `Reachable` signals during flip-flopping
    /// bursts leave `retry_active` and the toast slot untouched — only
    /// `mark_recovered` clears them.
    pub(super) const fn mark_unreachable(&mut self) -> bool {
        self.status = AvailabilityStatus::Unreachable;
        let newly_active = !self.retry_active;
        self.retry_active = true;
        newly_active
    }

    /// Marks the service rate-limited. Same retry-spawn semantics as
    /// `mark_unreachable`.
    pub(super) const fn mark_rate_limited(&mut self) -> bool {
        self.status = AvailabilityStatus::RateLimited;
        let newly_active = !self.retry_active;
        self.retry_active = true;
        newly_active
    }

    /// The id of the tracked unavailability toast, if one was ever
    /// pushed. Callers must verify liveness against the toast manager
    /// before assuming a toast is still visible — the user may have
    /// dismissed it out-of-band, in which case the id refers to a
    /// toast that no longer exists.
    pub(super) const fn toast_id(&self) -> Option<u64> { self.unavailable_toast }

    pub(super) const fn set_toast(&mut self, id: u64) { self.unavailable_toast = Some(id); }

    /// Clear all unavailability state and consume the stored toast id
    /// if any. `Some(id)` signals the caller to dismiss the error
    /// toast and push a transient "available" info toast; `None`
    /// means the recovery was for a service we never toast-signalled
    /// as down, so the caller should stay silent.
    pub(super) const fn mark_recovered(&mut self) -> Option<u64> {
        self.status = AvailabilityStatus::Reachable;
        self.retry_active = false;
        self.unavailable_toast.take()
    }
}

pub(super) struct GitHubState {
    pub(super) availability:         ServiceAvailability,
    pub(super) fetch_cache:          RepoCache,
    pub(super) repo_fetch_in_flight: HashSet<OwnerRepo>,
    /// Live cache-missed repo fetches, keyed by `OwnerRepo` and
    /// started-at. Drives the "Retrieving GitHub repo details" lint-
    /// style toast. Populated on `RepoFetchQueued` (which only fires
    /// on cache miss) and cleared on `RepoFetchComplete`. Cache-hit
    /// fetches never appear here, so the toast doesn't flicker for
    /// work that never touches the network.
    pub(super) running_fetches:      HashMap<OwnerRepo, Instant>,
    pub(super) running_fetch_toast:  Option<ToastTaskId>,
}

impl GitHubState {
    pub(super) fn new() -> Self {
        Self {
            availability:         ServiceAvailability::new(),
            fetch_cache:          scan::new_repo_cache(),
            repo_fetch_in_flight: HashSet::new(),
            running_fetches:      HashMap::new(),
            running_fetch_toast:  None,
        }
    }
}

pub(super) struct CratesIoState {
    pub(super) availability: ServiceAvailability,
}

impl CratesIoState {
    pub(super) const fn new() -> Self {
        Self {
            availability: ServiceAvailability::new(),
        }
    }
}
