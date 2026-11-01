use tui_pane::ToastActivation;
use tui_pane::ToastId;
use tui_pane::ToastStyle::Warning;

use crate::http::GithubAuthGap;
use crate::http::ServiceKind;
use crate::http::ServiceSignal;
use crate::tui::app::App;
use crate::tui::app::phase_state::FailureReason;
use crate::tui::state::AvailabilityStatus;
use crate::tui::state::RecoveryOutcome;

impl App {
    /// One-shot startup check: when `gh auth token` yielded no token,
    /// every authenticated GitHub call silently no-ops (see
    /// [`crate::http::HttpClient`]), so CI runs and rate-limit buckets
    /// never load. Mark GitHub unavailable — the git-pane rate-limit rows
    /// read this to surface the remediation hint — and push a one-time
    /// persistent warning toast whose copy depends on whether `gh` is
    /// missing (install it) or merely logged out (`gh auth login`).
    ///
    /// Skipped under `cfg(test)`: the gap comes from a real `gh auth
    /// token` subprocess, so honoring it would make toast and render
    /// state depend on the host's gh login.
    pub(in crate::tui::app) fn warn_if_github_unauthenticated(&mut self) {
        if cfg!(test) {
            return;
        }
        let Some(gap) = self.net.http_client.github_auth_gap() else {
            return;
        };
        match gap {
            GithubAuthGap::NotInstalled => {
                self.net
                    .availability_for(ServiceKind::GitHub)
                    .mark_not_installed();
                self.framework.toasts.push_persistent(
                    "GitHub CLI not found",
                    "Install gh (https://cli.github.com), run `gh auth login`, then restart cargo-port. CI runs and rate limits are unavailable.",
                    Warning,
                    ToastActivation::Informational,
                    1,
                );
            },
            GithubAuthGap::Unauthenticated => {
                self.net
                    .availability_for(ServiceKind::GitHub)
                    .mark_unauthenticated();
                self.framework.toasts.push_persistent(
                    "GitHub not authenticated",
                    "CI runs and rate limits are unavailable. Run `gh auth login`, then restart cargo-port.",
                    Warning,
                    ToastActivation::Informational,
                    1,
                );
            },
        }
    }

    pub(super) fn apply_service_signal(&mut self, service_signal: ServiceSignal) {
        match service_signal {
            ServiceSignal::Reachable(service_kind) => self.handle_service_reachable(service_kind),
            ServiceSignal::Unreachable(service_kind) => {
                self.apply_unavailability(service_kind, AvailabilityKind::Unreachable);
            },
            ServiceSignal::RateLimited(service_kind) => {
                self.apply_unavailability(service_kind, AvailabilityKind::RateLimited);
            },
        }
    }
    /// A successful request is authoritative evidence the service
    /// works; treat it as recovery. Previously `Reachable` was a
    /// no-op to avoid flicker, but that left the persistent
    /// unavailability toast stuck whenever the retry probe couldn't
    /// complete (tight 1s timeout, graphql quota quirks, etc.). The
    /// recovery work fires only on the actual state transition, so
    /// steady-state success signals stay silent. With the grace
    /// window in place, an `unavailable_toast` id is only set after
    /// the confirm handler fires — so a Reachable signal *inside*
    /// the grace window finds `unavailable_toast == None` and
    /// silently clears state without flashing a "back online" toast,
    /// without triggering the missing-data refetch.
    pub(super) fn handle_service_reachable(&mut self, service_kind: ServiceKind) {
        let recovery_outcome = self.net.availability_for(service_kind).mark_reachable();
        self.apply_recovery_outcome(service_kind, recovery_outcome);
    }
    /// Record the unavailability transition and spawn the retry
    /// thread. The user-visible toast is **not** pushed here — it's
    /// deferred to the [`Self::confirm_service_unreachable`] handler
    /// which only fires after the configured HTTP unavailable grace period
    /// window elapses without recovery. Single transient timeouts
    /// in a sea of successful fetches never reach the UI.
    pub(super) fn apply_unavailability(
        &mut self,
        service: ServiceKind,
        availability_kind: AvailabilityKind,
    ) {
        let spawn_retry = {
            let availability = self.net.availability_for(service);
            match availability_kind {
                AvailabilityKind::Unreachable => availability.mark_unreachable(),
                AvailabilityKind::RateLimited => availability.mark_rate_limited(),
            }
        };
        if spawn_retry {
            self.spawn_service_retry(service);
        }
    }
    /// Surface the persistent "service unavailable" toast. Called
    /// from the dispatch path when
    /// [`crate::scan::BackgroundMsg::ServiceUnreachableConfirmed`]
    /// arrives — i.e. after the retry thread waited
    /// configured HTTP unavailable grace period and confirmed the service is
    /// still down. No-op if the state has flipped back to reachable
    /// during the grace window (a real fetch landed) or a live toast
    /// is already showing.
    pub(super) fn confirm_service_unreachable(&mut self, service_kind: ServiceKind) {
        let (availability_kind, prior_toast) = {
            let availability = self.net.availability_for(service_kind);
            let availability_kind = match availability.status() {
                AvailabilityStatus::Unreachable => AvailabilityKind::Unreachable,
                AvailabilityStatus::RateLimited => AvailabilityKind::RateLimited,
                // The GitHub token is fixed for the process. Missing-token
                // `AvailabilityStatus::NotInstalled` and `Unauthenticated` states never start
                // `spawn_service_retry`, so `confirm_service_unreachable` cannot reach those states
                // through retry.
                AvailabilityStatus::Reachable
                | AvailabilityStatus::Unauthenticated
                | AvailabilityStatus::NotInstalled => return,
            };
            (availability_kind, availability.toast_id())
        };
        let alive = prior_toast.is_some_and(|id| self.framework.toasts.is_alive(id));
        if alive {
            return;
        }
        let toast_id = self.push_service_unavailable_toast(service_kind, availability_kind);
        self.net.availability_for(service_kind).set_toast(toast_id);
        // For `ServiceKind::GitHub`, `fail_startup_repo_phase` with `FailureReason::RateLimited` or
        // `FetchError` finishes `startup.repo` instead of waiting for timeout when confirmed
        // unavailability prevents its fetches completing. `push_service_unavailable_toast` already
        // names the reason, so the row adds no duplicate reason.
        if service_kind == ServiceKind::GitHub {
            let failure_reason = match availability_kind {
                AvailabilityKind::RateLimited => FailureReason::RateLimited,
                AvailabilityKind::Unreachable => FailureReason::FetchError,
            };
            self.fail_startup_repo_phase(failure_reason);
        }
    }
    pub(super) fn push_service_unavailable_toast(
        &mut self,
        service_kind: ServiceKind,
        availability_kind: AvailabilityKind,
    ) -> ToastId {
        let (title, body) = service_unavailable_message(service_kind, availability_kind);
        self.framework.toasts.push_persistent(
            title,
            body,
            Warning,
            ToastActivation::Informational,
            1,
        )
    }
    /// Spawn the retry / grace probe thread.
    ///
    /// The thread sleeps for configured HTTP unavailable grace period before its
    /// first probe. If the service has recovered by then, emit a
    /// silent recovery (no "back online" toast — none was pushed).
    /// Otherwise emit [`crate::scan::BackgroundMsg::ServiceUnreachableConfirmed`]
    /// to push the user-visible toast, then enter the 1Hz retry loop
    /// until probe succeeds.
    pub(super) fn spawn_service_retry(&self, service_kind: ServiceKind) {
        #[cfg(test)]
        if !self.scan.retry_spawn_mode().is_enabled() {
            return;
        }

        self.startup_services.spawn_service_retry_probe(
            self.background.background_sender(),
            self.net.http_client(),
            service_kind,
        );
    }
    /// Apply a `ServiceRecovered` message from the retry probe.
    /// Routes through the shared [`Self::apply_recovery_outcome`]
    /// helper so the toast handling and refetch hook stay in lockstep
    /// with the `handle_service_reachable` path.
    pub(super) fn mark_service_recovered(&mut self, service_kind: ServiceKind) {
        let recovery_outcome = self.net.availability_for(service_kind).mark_recovered();
        self.apply_recovery_outcome(service_kind, recovery_outcome);
    }
    /// Unified post-recovery dispatch: dismiss / push the back-online
    /// toast on the `WithToast` variant, then fire
    /// [`Self::refetch_missing_after_recovery`] — but only for a
    /// *confirmed* outage. A transient blip absorbed by the grace window
    /// recovers `Silent`, and refetching there re-dispatches every fetch
    /// that failed during the blip; at startup those land after the panel
    /// hands the network rows back to steady state and leak the standalone
    /// "Fetching crates.io info" / "Retrieving GitHub repo details" toasts
    /// the startup panel was meant to embed. The `RecoveryOutcome::WithToast`
    /// arm of `apply_recovery_outcome` owns the missing-data refetch for outages
    /// the user actually saw, while `NoTransition` and `Silent` suppress that startup burst.
    fn apply_recovery_outcome(&mut self, service: ServiceKind, recovery_outcome: RecoveryOutcome) {
        match recovery_outcome {
            RecoveryOutcome::NoTransition => (),
            RecoveryOutcome::Silent => {},
            RecoveryOutcome::WithToast(toast_id) => {
                self.framework.toasts.dismiss(toast_id);
                let (title, body) = service_recovered_message(service);
                self.show_timed_toast(title, body);
                self.refetch_missing_after_recovery(service);
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AvailabilityKind {
    Unreachable,
    RateLimited,
}

const fn service_unavailable_message(
    service: ServiceKind,
    kind: AvailabilityKind,
) -> (&'static str, &'static str) {
    match (service, kind) {
        (ServiceKind::GitHub, AvailabilityKind::Unreachable) => (
            "GitHub unreachable",
            "Rate limits and CI data are unavailable until GitHub recovers.",
        ),
        (ServiceKind::GitHub, AvailabilityKind::RateLimited) => (
            "GitHub rate-limited",
            "CI data is paused until the rate-limit bucket refills.",
        ),
        (ServiceKind::CratesIo, AvailabilityKind::Unreachable) => (
            "crates.io unreachable",
            "Crate metadata is unavailable until crates.io recovers.",
        ),
        (ServiceKind::CratesIo, AvailabilityKind::RateLimited) => (
            "crates.io rate-limited",
            "Crate metadata is paused until the rate-limit bucket refills.",
        ),
    }
}

const fn service_recovered_message(service_kind: ServiceKind) -> (&'static str, &'static str) {
    match service_kind {
        ServiceKind::GitHub => ("GitHub available", "Back online."),
        ServiceKind::CratesIo => ("crates.io available", "Back online."),
    }
}

#[cfg(test)]
mod tests {
    use super::RecoveryOutcome;
    use super::ServiceKind;
    use super::ToastId;
    use crate::ci::OwnerRepo;
    use crate::project::ProjectPrData;
    use crate::scan;
    use crate::scan::CachedRepoData;
    use crate::tui::test_support;

    #[test]
    fn only_confirmed_outage_recovery_triggers_refetch() {
        let mut app = test_support::make_app(&[]);
        let owner_repo = OwnerRepo::new("acme", "failed");
        scan::store_cached_repo_data(
            &app.net.github.fetch_cache,
            &owner_repo,
            CachedRepoData {
                runs:         Vec::new(),
                meta:         None,
                github_total: 0,
                pr_data:      ProjectPrData::Unfetched,
            },
        );

        app.apply_recovery_outcome(ServiceKind::GitHub, RecoveryOutcome::NoTransition);
        assert!(
            scan::load_cached_repo_data(&app.net.github.fetch_cache, &owner_repo).is_some(),
            "NoTransition must not refetch failed entries"
        );

        // `Silent` absorbs a grace-window blip: refetching could leak standalone toasts
        // after startup handoff. Only a user-visible `WithToast` outage refetches.
        app.apply_recovery_outcome(ServiceKind::GitHub, RecoveryOutcome::Silent);
        assert!(
            scan::load_cached_repo_data(&app.net.github.fetch_cache, &owner_repo).is_some(),
            "Silent must not refetch failed entries after a grace-window blip"
        );

        app.apply_recovery_outcome(ServiceKind::GitHub, RecoveryOutcome::WithToast(ToastId(1)));
        assert!(
            scan::load_cached_repo_data(&app.net.github.fetch_cache, &owner_repo).is_none(),
            "WithToast must invalidate failed entries for refetch"
        );
    }
}
