use std::collections::HashSet;

use crate::ci::OwnerRepo;
use crate::scan;
use crate::scan::RepoCache;

pub(super) struct ServiceAvailability {
    unreachable:  bool,
    retry_active: bool,
}

impl ServiceAvailability {
    pub(super) const fn new() -> Self {
        Self {
            unreachable:  false,
            retry_active: false,
        }
    }

    pub(super) const fn is_unreachable(&self) -> bool { self.unreachable }

    pub(super) const fn mark_reachable(&mut self) { self.unreachable = false; }

    /// Marks the service unreachable. Returns `true` when `retry_active`
    /// transitions from false to true, i.e. the caller should spawn the
    /// retry loop. Subsequent calls while a retry is already running
    /// return `false`, so the loop is not respawned.
    pub(super) const fn mark_unreachable(&mut self) -> bool {
        self.unreachable = true;
        let newly_active = !self.retry_active;
        self.retry_active = true;
        newly_active
    }

    pub(super) const fn mark_recovered(&mut self) {
        self.unreachable = false;
        self.retry_active = false;
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
