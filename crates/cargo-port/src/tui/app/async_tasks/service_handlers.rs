use std::thread;
use std::time::Duration;

use tui_pane::ToastId;
use tui_pane::ToastStyle::Warning;

use crate::constants::SERVICE_RETRY_SECS;
use crate::constants::SERVICE_UNAVAILABLE_GRACE;
use crate::http::ServiceKind;
use crate::http::ServiceSignal;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::tui::app::App;
use crate::tui::app::phase_state::FailureReason;
use crate::tui::state::AvailabilityStatus;
use crate::tui::state::RecoveryOutcome;

impl App {
    /// One-shot startup check: when `gh auth token` returned nothing,
    /// every authenticated GitHub call silently no-ops (see
    /// [`crate::http::HttpClient`]), so CI runs and rate-limit buckets
    /// never load. Mark GitHub unauthenticated — the git-pane
    /// rate-limit rows read this to surface a `gh auth login` hint — and
    /// push a one-time persistent warning toast.
    ///
    /// Skipped under `cfg(test)`: the token comes from a real `gh auth
    /// token` subprocess, so honoring it would make toast and render
    /// state depend on the host's gh login.
    pub fn warn_if_github_unauthenticated(&mut self) {
        if cfg!(test) {
            return;
        }
        if self.net.http_client.has_github_token() {
            return;
        }
        self.net
            .availability_for(ServiceKind::GitHub)
            .mark_unauthenticated();
        self.framework.toasts.push_persistent(
            "GitHub not authenticated",
            "CI runs and rate limits are unavailable. Run `gh auth login`, then restart cargo-port.",
            Warning,
            None,
            1,
        );
    }

    pub(super) fn apply_service_signal(&mut self, signal: ServiceSignal) {
        match signal {
            ServiceSignal::Reachable(service) => self.handle_service_reachable(service),
            ServiceSignal::Unreachable(service) => {
                self.apply_unavailability(service, AvailabilityKind::Unreachable);
            },
            ServiceSignal::RateLimited(service) => {
                self.apply_unavailability(service, AvailabilityKind::RateLimited);
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
    /// while still triggering the missing-data refetch.
    pub(super) fn handle_service_reachable(&mut self, service: ServiceKind) {
        let outcome = self.net.availability_for(service).mark_reachable();
        self.apply_recovery_outcome(service, outcome);
    }
    /// Record the unavailability transition and spawn the retry
    /// thread. The user-visible toast is **not** pushed here — it's
    /// deferred to the [`Self::confirm_service_unreachable`] handler
    /// which only fires after the [`SERVICE_UNAVAILABLE_GRACE`]
    /// window elapses without recovery. Single transient timeouts
    /// in a sea of successful fetches never reach the UI.
    pub(super) fn apply_unavailability(&mut self, service: ServiceKind, kind: AvailabilityKind) {
        let spawn_retry = {
            let avail = self.net.availability_for(service);
            match kind {
                AvailabilityKind::Unreachable => avail.mark_unreachable(),
                AvailabilityKind::RateLimited => avail.mark_rate_limited(),
            }
        };
        if spawn_retry {
            self.spawn_service_retry(service);
        }
    }
    /// Surface the persistent "service unavailable" toast. Called
    /// from the dispatch path when [`BackgroundMsg::ServiceUnreachableConfirmed`]
    /// arrives — i.e. after the retry thread waited
    /// [`SERVICE_UNAVAILABLE_GRACE`] and confirmed the service is
    /// still down. No-op if the state has flipped back to reachable
    /// during the grace window (a real fetch landed) or a live toast
    /// is already showing.
    pub(super) fn confirm_service_unreachable(&mut self, service: ServiceKind) {
        let (kind, prior_toast) = {
            let avail = self.net.availability_for(service);
            let kind = match avail.status() {
                AvailabilityStatus::Unreachable => AvailabilityKind::Unreachable,
                AvailabilityStatus::RateLimited => AvailabilityKind::RateLimited,
                // Unauthenticated never spawns a retry (token is fixed for
                // the process), so this confirm path can't reach it.
                AvailabilityStatus::Reachable | AvailabilityStatus::Unauthenticated => return,
            };
            (kind, avail.toast_id())
        };
        let alive = prior_toast.is_some_and(|id| self.framework.toasts.is_alive(id));
        if alive {
            return;
        }
        let toast_id = self.push_service_unavailable_toast(service, kind);
        self.net.availability_for(service).set_toast(toast_id);
        // A confirmed-down GitHub means startup repo fetches will never
        // complete; fail the startup panel's repo row so it finishes
        // instead of waiting out the timeout. The toast above names the
        // reason, so the row failure adds none of its own.
        if service == ServiceKind::GitHub {
            let reason = match kind {
                AvailabilityKind::RateLimited => FailureReason::RateLimited,
                AvailabilityKind::Unreachable => FailureReason::FetchError,
            };
            self.fail_startup_repo_phase(reason);
        }
    }
    pub(super) fn push_service_unavailable_toast(
        &mut self,
        service: ServiceKind,
        kind: AvailabilityKind,
    ) -> ToastId {
        let (title, body) = service_unavailable_message(service, kind);
        self.framework
            .toasts
            .push_persistent(title, body, Warning, None, 1)
    }
    /// Spawn the retry / grace probe thread.
    ///
    /// The thread sleeps for [`SERVICE_UNAVAILABLE_GRACE`] before its
    /// first probe. If the service has recovered by then, emit a
    /// silent recovery (no "back online" toast — none was pushed).
    /// Otherwise emit [`BackgroundMsg::ServiceUnreachableConfirmed`]
    /// to push the user-visible toast, then enter the 1Hz retry loop
    /// until probe succeeds.
    pub(super) fn spawn_service_retry(&self, service: ServiceKind) {
        #[cfg(test)]
        if !self.scan.retry_spawn_mode().is_enabled() {
            return;
        }

        let tx = self.background.background_sender();
        let client = self.net.http_client();
        thread::spawn(move || {
            thread::sleep(SERVICE_UNAVAILABLE_GRACE);
            if client.probe_service(service) {
                scan::emit_service_recovered(&tx, service);
                return;
            }
            let _ = tx.send(BackgroundMsg::ServiceUnreachableConfirmed { service });
            loop {
                if client.probe_service(service) {
                    scan::emit_service_recovered(&tx, service);
                    break;
                }
                thread::sleep(Duration::from_secs(SERVICE_RETRY_SECS));
            }
        });
    }
    /// Apply a `ServiceRecovered` message from the retry probe.
    /// Routes through the shared [`Self::apply_recovery_outcome`]
    /// helper so the toast handling and refetch hook stay in lockstep
    /// with the `handle_service_reachable` path.
    pub(super) fn mark_service_recovered(&mut self, service: ServiceKind) {
        let outcome = self.net.availability_for(service).mark_recovered();
        self.apply_recovery_outcome(service, outcome);
    }
    /// Unified post-recovery dispatch: dismiss / push the back-online
    /// toast on the `WithToast` variant, then fire
    /// [`Self::refetch_missing_after_recovery`] on every transition
    /// (silent or not) so rows that failed to fetch during the outage
    /// fill in once the service is reachable again.
    fn apply_recovery_outcome(&mut self, service: ServiceKind, outcome: RecoveryOutcome) {
        match outcome {
            RecoveryOutcome::NoTransition => return,
            RecoveryOutcome::Silent => {},
            RecoveryOutcome::WithToast(toast_id) => {
                self.framework.toasts.dismiss(toast_id);
                let (title, body) = service_recovered_message(service);
                self.show_timed_toast(title, body);
            },
        }
        self.refetch_missing_after_recovery(service);
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

const fn service_recovered_message(service: ServiceKind) -> (&'static str, &'static str) {
    match service {
        ServiceKind::GitHub => ("GitHub available", "Back online."),
        ServiceKind::CratesIo => ("crates.io available", "Back online."),
    }
}
