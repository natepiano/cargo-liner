//! The `Net` subsystem.
//!
//! Owns every "talks-to-the-network" field: the shared
//! [`HttpClient`], the GitHub sub-state (availability, repo-fetch
//! cache, in-flight set, running tracker + toast), and the
//! crates.io sub-state (availability). App orchestration reaches
//! in via [`Net::github`] / [`Net::github_mut`] and
//! [`Net::crates_io`] / [`Net::crates_io_mut`].
//!
//! Cross-subsystem orchestration that touches Net plus other
//! subsystems (toast push/dismiss, background spawn, scan reset)
//! stays on `App` — see `App::apply_unavailability`,
//! `App::sync_running_repo_fetch_toast`,
//! `App::spawn_repo_fetch_for_git_info`,
//! `App::handle_repo_fetch_queued`,
//! `App::handle_repo_fetch_complete`,
//! `App::spawn_rate_limit_prime`. This matches the Lint
//! pattern, where lookup / reset live on the subsystem and toast /
//! runtime orchestration live on App.

use std::collections::HashSet;

use super::running_tracker::RunningTracker;
use crate::ci::OwnerRepo;
use crate::http::GitHubRateLimit;
use crate::http::HttpClient;
use crate::scan;
use crate::scan::RepoCache;

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

pub struct ServiceAvailability {
    status:            AvailabilityStatus,
    retry_active:      bool,
    unavailable_toast: Option<u64>,
}

impl ServiceAvailability {
    pub const fn new() -> Self {
        Self {
            status:            AvailabilityStatus::Reachable,
            retry_active:      false,
            unavailable_toast: None,
        }
    }

    pub const fn status(&self) -> AvailabilityStatus { self.status }

    #[cfg(test)]
    pub const fn is_unavailable(&self) -> bool { !self.status.is_available() }

    /// Mark the service reachable. Returns the tracked toast id iff
    /// this call is the transition out of an unavailable state —
    /// caller should dismiss that toast and fire the recovery message.
    /// Subsequent `Reachable` signals while already reachable return
    /// `None`, so the recovery toast only fires once per outage.
    pub const fn mark_reachable(&mut self) -> Option<u64> {
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
    pub const fn mark_unreachable(&mut self) -> bool {
        self.status = AvailabilityStatus::Unreachable;
        let newly_active = !self.retry_active;
        self.retry_active = true;
        newly_active
    }

    /// Marks the service rate-limited. Same retry-spawn semantics as
    /// `mark_unreachable`.
    pub const fn mark_rate_limited(&mut self) -> bool {
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
    pub const fn toast_id(&self) -> Option<u64> { self.unavailable_toast }

    pub const fn set_toast(&mut self, id: u64) { self.unavailable_toast = Some(id); }

    /// Clear all unavailability state and consume the stored toast id
    /// if any. `Some(id)` signals the caller to dismiss the error
    /// toast and push a transient "available" info toast; `None`
    /// means the recovery was for a service we never toast-signalled
    /// as down, so the caller should stay silent.
    pub const fn mark_recovered(&mut self) -> Option<u64> {
        self.status = AvailabilityStatus::Reachable;
        self.retry_active = false;
        self.unavailable_toast.take()
    }
}

pub struct Github {
    availability:         ServiceAvailability,
    fetch_cache:          RepoCache,
    repo_fetch_in_flight: HashSet<OwnerRepo>,
    /// Live cache-missed repo fetches plus the single sticky
    /// "Retrieving GitHub repo details" toast slot.
    running:              RunningTracker<OwnerRepo>,
}

impl Github {
    fn new() -> Self {
        Self {
            availability:         ServiceAvailability::new(),
            fetch_cache:          scan::new_repo_cache(),
            repo_fetch_in_flight: HashSet::new(),
            running:              RunningTracker::new(),
        }
    }

    pub const fn fetch_cache(&self) -> &RepoCache { &self.fetch_cache }

    pub const fn repo_fetch_in_flight_mut(&mut self) -> &mut HashSet<OwnerRepo> {
        &mut self.repo_fetch_in_flight
    }

    pub fn contains_in_flight(&self, repo: &OwnerRepo) -> bool {
        self.repo_fetch_in_flight.contains(repo)
    }

    pub const fn running(&self) -> &RunningTracker<OwnerRepo> { &self.running }

    pub const fn running_mut(&mut self) -> &mut RunningTracker<OwnerRepo> { &mut self.running }

    #[cfg(test)]
    pub const fn availability(&self) -> &ServiceAvailability { &self.availability }

    pub const fn availability_mut(&mut self) -> &mut ServiceAvailability { &mut self.availability }

    /// Reset every GitHub field to its post-construction state.
    /// Called by `Net::clear_for_tree_change` on rescan; replaces
    /// the four inline field writes that used to live in
    /// `App::rescan`.
    fn clear_for_tree_change(&mut self) {
        self.fetch_cache = scan::new_repo_cache();
        self.repo_fetch_in_flight.clear();
        self.running.clear();
    }
}

pub struct CratesIo {
    availability: ServiceAvailability,
}

impl CratesIo {
    const fn new() -> Self {
        Self {
            availability: ServiceAvailability::new(),
        }
    }

    #[cfg(test)]
    pub const fn availability(&self) -> &ServiceAvailability { &self.availability }

    pub const fn availability_mut(&mut self) -> &mut ServiceAvailability { &mut self.availability }
}

pub struct Net {
    http_client: HttpClient,
    github:      Github,
    crates_io:   CratesIo,
}

impl Net {
    pub fn new(http_client: HttpClient) -> Self {
        Self {
            http_client,
            github: Github::new(),
            crates_io: CratesIo::new(),
        }
    }

    pub fn http_client(&self) -> HttpClient { self.http_client.clone() }

    pub const fn http_client_ref(&self) -> &HttpClient { &self.http_client }

    pub fn rate_limit(&self) -> GitHubRateLimit { self.http_client.rate_limit() }

    pub fn set_force_github_rate_limit(&self, on: bool) {
        self.http_client.set_force_github_rate_limit(on);
    }

    pub const fn github(&self) -> &Github { &self.github }

    pub const fn github_mut(&mut self) -> &mut Github { &mut self.github }

    #[cfg(test)]
    pub const fn crates_io(&self) -> &CratesIo { &self.crates_io }

    pub const fn crates_io_mut(&mut self) -> &mut CratesIo { &mut self.crates_io }

    pub const fn github_status(&self) -> AvailabilityStatus { self.github.availability.status() }

    /// Clear the GitHub sub-state on rescan: drop the repo-fetch
    /// cache, the in-flight set, and the running tracker (running
    /// fetches map + toast slot). Crates.io and the `HttpClient`
    /// keep their state across rescans.
    pub fn clear_for_tree_change(&mut self) { self.github.clear_for_tree_change(); }
}
