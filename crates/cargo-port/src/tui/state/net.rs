//! The `Net` subsystem.
//!
//! Owns every "talks-to-the-network" field: the shared
//! [`HttpClient`], the GitHub sub-state (availability, repo-fetch
//! cache, in-flight set, running tracker + toast), and the
//! crates.io sub-state (availability). App orchestration reaches
//! in via the public [`Net::github`] and [`Net::crates_io`] fields.
//!
//! The standalone GitHub / crates.io running toasts are gated by
//! [`NetworkToastStage`]: their toast slots live only in
//! [`NetworkToastStage::SteadyState`], so while a scan's "Startup" panel owns
//! those rows there is no slot to populate and the standalone toast cannot
//! fire.
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

use tui_pane::RunningTracker;
use tui_pane::ToastId;
use tui_pane::ToastTaskId;

use crate::ci::OwnerRepo;
use crate::http::GitHubRateLimit;
use crate::http::HttpClient;
use crate::http::ServiceKind;
use crate::scan;
use crate::scan::RepoCache;

/// Availability for a single service. `Unreachable` means the network
/// layer can't talk to the service at all; `RateLimited` means the
/// service is reachable but refusing our requests for quota reasons;
/// `Unauthenticated` and `NotInstalled` (GitHub only) both mean `gh auth
/// token` produced no token at startup — `Unauthenticated` when `gh` is
/// installed but logged out, `NotInstalled` when the `gh` binary is
/// absent — so authenticated calls silently no-op. Recovery, display
/// text, and toast copy diverge between them — hence the explicit enum.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AvailabilityStatus {
    #[default]
    Reachable,
    Unreachable,
    RateLimited,
    /// GitHub only: `gh` is installed but returned no token. Authenticated
    /// REST / GraphQL calls short-circuit, so CI + rate-limit data never
    /// load.
    Unauthenticated,
    /// GitHub only: the `gh` binary was not found on `PATH`, so no token
    /// could be obtained. Same no-op effect as `Unauthenticated`; the
    /// remediation differs (install `gh` rather than `gh auth login`).
    NotInstalled,
}

impl AvailabilityStatus {
    pub const fn is_available(self) -> bool { matches!(self, Self::Reachable) }

    /// True when GitHub has no usable auth token — whether `gh` is logged
    /// out (`Unauthenticated`) or absent (`NotInstalled`). Both render the
    /// same actionable-warning state; only the remediation copy differs.
    pub const fn is_unauthenticated(self) -> bool {
        matches!(self, Self::Unauthenticated | Self::NotInstalled)
    }
}

/// Outcome of a "service became reachable" call. The retry / recovery
/// paths converge on this so callers handle every case the same way:
/// `NoTransition` no-ops, `Silent` triggers refetch without a toast,
/// `WithToast(id)` triggers refetch and surfaces the back-online toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// The service was already reachable — nothing to do.
    NoTransition,
    /// State transitioned from unavailable → reachable, but no
    /// user-visible toast was ever pushed (the grace window absorbed
    /// the outage). Refetch missing data; stay silent on toasts.
    Silent,
    /// State transitioned from unavailable → reachable and a toast
    /// was up. Dismiss it, push the back-online message, refetch.
    WithToast(ToastId),
}

/// Render-side snapshot of service availability — collapses
/// [`AvailabilityStatus`]'s three-way state to a binary "render the
/// placeholder, or render the real value." UI code carries this on
/// per-row data so the rendering function stays a pure read.
/// Applies equally to GitHub and crates.io.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ServiceStatus {
    #[default]
    Available,
    Unreachable,
}

pub struct ServiceAvailability {
    status:            AvailabilityStatus,
    retry_active:      bool,
    unavailable_toast: Option<ToastId>,
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

    /// Mark the service reachable. Returns [`RecoveryOutcome`]:
    /// - `NoTransition` if already reachable (subsequent successes are silent).
    /// - `Silent` on the transition edge when no toast was ever surfaced (the grace window absorbed
    ///   the outage). Caller refetches missing data without showing a toast.
    /// - `WithToast(id)` on the transition edge with a live toast slot to dismiss; caller also
    ///   pushes the recovery toast and refetches.
    pub const fn mark_reachable(&mut self) -> RecoveryOutcome {
        let was_unavailable = !matches!(self.status, AvailabilityStatus::Reachable);
        self.status = AvailabilityStatus::Reachable;
        if !was_unavailable {
            return RecoveryOutcome::NoTransition;
        }
        self.retry_active = false;
        match self.unavailable_toast.take() {
            Some(id) => RecoveryOutcome::WithToast(id),
            None => RecoveryOutcome::Silent,
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

    /// Marks the service unauthenticated (no `gh` token at startup).
    /// Unlike the reachability transitions this spawns no retry loop —
    /// the token is fixed for the process lifetime, so recovery needs a
    /// restart. Set once from `App::warn_if_github_unauthenticated`.
    pub const fn mark_unauthenticated(&mut self) {
        self.status = AvailabilityStatus::Unauthenticated;
    }

    /// Marks GitHub unavailable because the `gh` binary was not found at
    /// startup. Like [`Self::mark_unauthenticated`] this spawns no retry
    /// loop — installing `gh` needs a restart. Set once from
    /// `App::warn_if_github_unauthenticated`.
    pub const fn mark_not_installed(&mut self) { self.status = AvailabilityStatus::NotInstalled; }

    /// The id of the tracked unavailability toast, if one was ever
    /// pushed. Callers must verify liveness against the toast manager
    /// before assuming a toast is still visible — the user may have
    /// dismissed it out-of-band, in which case the id refers to a
    /// toast that no longer exists.
    pub const fn toast_id(&self) -> Option<ToastId> { self.unavailable_toast }

    pub const fn set_toast(&mut self, id: ToastId) { self.unavailable_toast = Some(id); }

    /// Convenience for the retry-probe path: identical semantics to
    /// [`Self::mark_reachable`]. Kept as a distinct name so the
    /// retry-thread caller reads cleanly.
    pub const fn mark_recovered(&mut self) -> RecoveryOutcome { self.mark_reachable() }
}

pub struct Github {
    pub availability:     ServiceAvailability,
    pub fetch_cache:      RepoCache,
    repo_fetch_in_flight: HashSet<OwnerRepo>,
    pr_check_polls:       HashSet<(OwnerRepo, u32)>,
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
            pr_check_polls:       HashSet::new(),
            running:              RunningTracker::new(),
        }
    }

    pub const fn repo_fetch_in_flight_mut(&mut self) -> &mut HashSet<OwnerRepo> {
        &mut self.repo_fetch_in_flight
    }

    pub fn contains_in_flight(&self, repo: &OwnerRepo) -> bool {
        self.repo_fetch_in_flight.contains(repo)
    }

    pub fn insert_pr_check_poll(&mut self, repo: OwnerRepo, number: u32) -> bool {
        self.pr_check_polls.insert((repo, number))
    }

    pub fn remove_pr_check_poll(&mut self, repo: &OwnerRepo, number: u32) -> bool {
        self.pr_check_polls.remove(&(repo.clone(), number))
    }

    pub fn pr_check_poll_numbers(&self, repo: &OwnerRepo) -> HashSet<u32> {
        self.pr_check_polls
            .iter()
            .filter_map(|(poll_repo, number)| (poll_repo == repo).then_some(*number))
            .collect()
    }

    pub fn has_pr_check_polls(&self) -> bool { !self.pr_check_polls.is_empty() }

    pub fn retain_pr_check_polls_for_repo(
        &mut self,
        repo: &OwnerRepo,
        active_numbers: &HashSet<u32>,
    ) -> bool {
        let before = self.pr_check_polls.len();
        self.pr_check_polls
            .retain(|(poll_repo, number)| poll_repo != repo || active_numbers.contains(number));
        before != self.pr_check_polls.len()
    }

    pub const fn running(&self) -> &RunningTracker<OwnerRepo> { &self.running }

    pub const fn running_mut(&mut self) -> &mut RunningTracker<OwnerRepo> { &mut self.running }

    /// Reset every GitHub field to its post-construction state.
    /// Called by `Net::clear_for_tree_change` on rescan; replaces
    /// the four inline field writes that used to live in
    /// `App::rescan`.
    fn clear_for_tree_change(&mut self) {
        self.fetch_cache = scan::new_repo_cache();
        self.repo_fetch_in_flight.clear();
        self.pr_check_polls.clear();
        self.running.clear();
    }
}

pub struct CratesIo {
    pub availability: ServiceAvailability,
    /// Live in-flight crates.io fetches keyed by crate name, paired
    /// with the single sticky "Fetching crates.io info" toast slot.
    /// Synced each tick by `App::sync_running_crates_io_toast`. Mirrors
    /// the GitHub repo-fetch tracker.
    running:          RunningTracker<String>,
}

impl CratesIo {
    fn new() -> Self {
        Self {
            availability: ServiceAvailability::new(),
            running:      RunningTracker::new(),
        }
    }

    pub const fn running(&self) -> &RunningTracker<String> { &self.running }

    pub const fn running_mut(&mut self) -> &mut RunningTracker<String> { &mut self.running }
}

/// The standalone GitHub / crates.io running-toast slots. One sticky toast
/// per service, created only in steady state. This value exists exclusively
/// inside [`NetworkToastStage::SteadyState`]: during startup the consolidated
/// panel owns those rows, so there is no slot here to populate and the
/// standalone toast cannot be created.
#[derive(Default)]
pub struct NetworkRunningToasts {
    /// "Fetching crates.io info" toast slot.
    pub crates_io: Option<ToastTaskId>,
    /// "Retrieving GitHub repo details" toast slot.
    pub github:    Option<ToastTaskId>,
}

/// Lifecycle of the GitHub + crates.io progress surface.
///
/// While a scan runs and its consolidated "Startup" panel is open, the panel
/// owns the GitHub and crates.io rows — `StartupOwned` carries no toast slot,
/// so no standalone running toast can be created. When startup completes the
/// stage flips to `SteadyState`, which is the only variant that holds the
/// per-service slots. A rescan returns the stage to `StartupOwned`. Making the
/// slot absent during startup is what prevents the standalone crates.io /
/// GitHub toast from firing while the panel owns the row.
pub enum NetworkToastStage {
    /// Pre-startup and while the panel is open. No standalone-toast slot.
    StartupOwned,
    /// Steady state: standalone running toasts emit from these slots.
    SteadyState(NetworkRunningToasts),
}

pub struct Net {
    pub http_client: HttpClient,
    pub github:      Github,
    pub crates_io:   CratesIo,
    /// Lifecycle gate for the standalone GitHub / crates.io running toasts.
    /// Begins `StartupOwned` so a network fetch processed before the startup
    /// panel even exists cannot leak a standalone toast.
    toast_stage:     NetworkToastStage,
}

impl Net {
    pub fn new(http_client: HttpClient) -> Self {
        Self {
            http_client,
            github: Github::new(),
            crates_io: CratesIo::new(),
            toast_stage: NetworkToastStage::StartupOwned,
        }
    }

    /// The steady-state network-toast slots, or `None` while startup owns the
    /// rows. The standalone-toast sync paths read the slot through this: a
    /// `None` return means there is structurally nowhere to store a toast id,
    /// so they no-op.
    pub const fn network_toasts(&self) -> Option<&NetworkRunningToasts> {
        match &self.toast_stage {
            NetworkToastStage::SteadyState(toasts) => Some(toasts),
            NetworkToastStage::StartupOwned => None,
        }
    }

    /// Mutable view of the steady-state network-toast slots, or `None` while
    /// startup owns the rows.
    pub const fn network_toasts_mut(&mut self) -> Option<&mut NetworkRunningToasts> {
        match &mut self.toast_stage {
            NetworkToastStage::SteadyState(toasts) => Some(toasts),
            NetworkToastStage::StartupOwned => None,
        }
    }

    /// Enter steady state: the panel has closed, so standalone GitHub /
    /// crates.io running toasts may now be created. Installs the (empty) slots.
    pub fn begin_steady_state_network_toasts(&mut self) {
        self.toast_stage = NetworkToastStage::SteadyState(NetworkRunningToasts::default());
    }

    /// Return the stage to `StartupOwned`, discarding the slots. The caller is
    /// responsible for finishing any live toasts first — once the slots are
    /// gone their ids are unrecoverable.
    pub const fn set_network_toasts_startup_owned(&mut self) {
        self.toast_stage = NetworkToastStage::StartupOwned;
    }

    pub fn http_client(&self) -> HttpClient { self.http_client.clone() }

    pub fn rate_limit(&self) -> GitHubRateLimit { self.http_client.rate_limit() }

    pub fn set_force_github_rate_limit(&self, on: bool) {
        self.http_client.set_force_github_rate_limit(on);
    }

    pub const fn github_status(&self) -> AvailabilityStatus { self.github.availability.status() }

    /// Clear the GitHub sub-state on rescan: drop the repo-fetch
    /// cache, the in-flight set, and the running tracker (running
    /// fetches map + toast slot). Crates.io and the `HttpClient`
    /// keep their state across rescans.
    pub fn clear_for_tree_change(&mut self) { self.github.clear_for_tree_change(); }

    pub const fn availability_for(&mut self, service: ServiceKind) -> &mut ServiceAvailability {
        match service {
            ServiceKind::GitHub => &mut self.github.availability,
            ServiceKind::CratesIo => &mut self.crates_io.availability,
        }
    }

    /// One-shot: hit GitHub's `/rate_limit` endpoint so the shared
    /// rate-limit cache is populated before any real request. The endpoint
    /// is quota-exempt, so this is safe to run even when GitHub is
    /// refusing other calls. Logged via `rate_limit_prime_ok` /
    /// `rate_limit_prime_failed`.
    pub fn spawn_rate_limit_prime(&self) {
        let client = self.http_client();
        std::thread::spawn(move || {
            let (rate_limit, _signal) = client.fetch_rate_limit();
            if rate_limit.is_some() {
                tracing::info!("rate_limit_prime_ok");
            } else {
                tracing::info!("rate_limit_prime_failed");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_unauthenticated_sets_status_and_reports_unavailable() {
        let mut avail = ServiceAvailability::new();
        avail.mark_unauthenticated();
        assert_eq!(avail.status(), AvailabilityStatus::Unauthenticated);
        assert!(avail.status().is_unauthenticated());
        // Unauthenticated is not "available" — the git pane reads this
        // to suppress real values and surface the auth hint instead.
        assert!(!avail.status().is_available());
    }

    #[test]
    fn mark_not_installed_sets_status_and_reports_unavailable() {
        let mut avail = ServiceAvailability::new();
        avail.mark_not_installed();
        assert_eq!(avail.status(), AvailabilityStatus::NotInstalled);
        // `gh` missing is still "no auth token" — the predicate the git
        // pane and pane-data readers branch on must cover both gaps.
        assert!(avail.status().is_unauthenticated());
        assert!(!avail.status().is_available());
    }
}
