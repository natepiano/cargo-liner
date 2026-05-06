use std::thread;
use std::time::Duration;

use crate::constants::SERVICE_RETRY_SECS;
use crate::http::ServiceKind;
use crate::http::ServiceSignal;
use crate::scan;
use crate::tui::app::App;
use crate::tui::toasts::ToastStyle::Warning;

impl App {
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
    /// steady-state success signals stay silent.
    pub(super) fn handle_service_reachable(&mut self, service: ServiceKind) {
        let Some(toast_id) = self.net.availability_for(service).mark_reachable() else {
            return;
        };
        self.toasts.dismiss(toast_id);
        let (title, body) = service_recovered_message(service);
        self.show_timed_toast(title, body);
    }
    pub(super) fn apply_unavailability(&mut self, service: ServiceKind, kind: AvailabilityKind) {
        let (spawn_retry, prior_toast) = {
            let avail = self.net.availability_for(service);
            let spawn_retry = match kind {
                AvailabilityKind::Unreachable => avail.mark_unreachable(),
                AvailabilityKind::RateLimited => avail.mark_rate_limited(),
            };
            (spawn_retry, avail.toast_id())
        };
        if spawn_retry {
            self.spawn_service_retry(service);
        }
        // The tracked toast id can go stale if the user dismissed the
        // toast while the service was still unavailable — the toast
        // manager evicts it after its exit animation, but the
        // `ServiceAvailability` still holds the id. Recheck aliveness
        // so the next unavailability signal re-pushes a fresh toast
        // instead of silently assuming one is visible.
        let alive = prior_toast.is_some_and(|id| self.toasts.is_alive(id));
        if !alive {
            let toast_id = self.push_service_unavailable_toast(service, kind);
            self.net.availability_for(service).set_toast(toast_id);
        }
    }
    pub(super) fn push_service_unavailable_toast(
        &mut self,
        service: ServiceKind,
        kind: AvailabilityKind,
    ) -> u64 {
        let (title, body) = service_unavailable_message(service, kind);
        let id = self.toasts.push_persistent(title, body, Warning, None, 1);
        let toast_len = self.toasts.active_now().len();
        self.toasts.viewport_mut().set_len(toast_len);
        id
    }
    pub(super) fn spawn_service_retry(&self, service: ServiceKind) {
        #[cfg(test)]
        if !self.scan.retry_spawn_mode().is_enabled() {
            return;
        }

        let tx = self.background.bg_sender();
        let client = self.net.http_client();
        thread::spawn(move || {
            loop {
                if client.probe_service(service) {
                    scan::emit_service_recovered(&tx, service);
                    break;
                }
                thread::sleep(Duration::from_secs(SERVICE_RETRY_SECS));
            }
        });
    }
    pub(super) fn mark_service_recovered(&mut self, service: ServiceKind) {
        let Some(toast_id) = self.net.availability_for(service).mark_recovered() else {
            return;
        };
        self.toasts.dismiss(toast_id);
        let (title, body) = service_recovered_message(service);
        self.show_timed_toast(title, body);
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
