use std::collections::HashSet;

use crate::ci::OwnerRepo;
use crate::scan;
use crate::scan::RepoCache;

pub(super) struct ServiceAvailability {
    unreachable:       bool,
    retry_active:      bool,
    unreachable_toast: Option<u64>,
}

impl ServiceAvailability {
    pub(super) const fn new() -> Self {
        Self {
            unreachable:       false,
            retry_active:      false,
            unreachable_toast: None,
        }
    }

    pub(super) const fn is_unreachable(&self) -> bool { self.unreachable }

    pub(super) const fn mark_reachable(&mut self) { self.unreachable = false; }

    /// Marks the service unreachable. Returns `true` when `retry_active`
    /// transitions from false to true, i.e. the caller should spawn the
    /// retry loop. Subsequent calls while a retry is already running
    /// return `false`, so the loop is not respawned. Note that `Reachable`
    /// signals during flip-flopping bursts leave `retry_active` and the
    /// toast slot untouched — only `mark_recovered` clears them.
    pub(super) const fn mark_unreachable(&mut self) -> bool {
        self.unreachable = true;
        let newly_active = !self.retry_active;
        self.retry_active = true;
        newly_active
    }

    /// Whether this service needs a fresh unreachable toast pushed. Only
    /// `true` when no toast is currently tracked. Caller pushes the
    /// toast, then records its id via `set_toast`. Split from
    /// `mark_unreachable` so the caller can act on the two concerns
    /// independently (retry spawn vs. toast dedup).
    pub(super) const fn needs_toast(&self) -> bool { self.unreachable_toast.is_none() }

    pub(super) const fn set_toast(&mut self, id: u64) { self.unreachable_toast = Some(id); }

    /// Clear unreachable + retry state and consume the stored toast id
    /// if any. `Some(id)` signals the caller to dismiss the error toast
    /// and push a transient "available" info toast; `None` means the
    /// recovery was for a service we never toast-signalled as down, so
    /// the caller should stay silent.
    pub(super) const fn mark_recovered(&mut self) -> Option<u64> {
        self.unreachable = false;
        self.retry_active = false;
        self.unreachable_toast.take()
    }
}

pub(super) struct GitHubState {
    pub(super) availability:         ServiceAvailability,
    pub(super) fetch_cache:          RepoCache,
    pub(super) repo_fetch_in_flight: HashSet<OwnerRepo>,
}

impl GitHubState {
    pub(super) fn new() -> Self {
        Self {
            availability:         ServiceAvailability::new(),
            fetch_cache:          scan::new_repo_cache(),
            repo_fetch_in_flight: HashSet::new(),
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
