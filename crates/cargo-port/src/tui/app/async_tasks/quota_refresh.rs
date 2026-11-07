use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::channel;
use crate::channel::Receiver;
use crate::channel::Sender;
use crate::http::GitHubRateLimit;
use crate::http::HttpClient;
use crate::http::ServiceKind;
use crate::http::ServiceSignal;
use crate::tui::app::App;

const RESET_RETRY_INTERVAL: Duration = Duration::from_secs(10);
const RESET_RETRY_COUNT: ResetRetries = ResetRetries::Three;

#[derive(Clone, Copy)]
enum ResetRetries {
    One,
    Two,
    Three,
}

#[derive(Clone, Copy)]
enum ResetRetrySchedule {
    Due {
        at:        SystemTime,
        remaining: ResetRetries,
    },
    Exhausted,
}

impl ResetRetrySchedule {
    fn after_fetch(self, now: SystemTime) -> Self {
        let remaining = match self {
            Self::Due {
                remaining: ResetRetries::Three,
                ..
            } => ResetRetries::Two,
            Self::Due {
                remaining: ResetRetries::Two,
                ..
            } => ResetRetries::One,
            Self::Due {
                remaining: ResetRetries::One,
                ..
            }
            | Self::Exhausted => {
                return Self::Exhausted;
            },
        };
        Self::Due {
            at: now + RESET_RETRY_INTERVAL,
            remaining,
        }
    }
}

/// The reason for a quota request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::tui) enum QuotaFetchReason {
    Startup,
    Reset,
    Recovery,
}

/// The terminal result of a quota fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::tui) enum QuotaFetchCompletion {
    Received(GitHubRateLimit),
    Failed,
}

/// Admission to the shared quota fetch.
pub(in crate::tui) enum QuotaFetchAdmission {
    Start(QuotaFetchPermit),
    InFlight,
    ResetAlreadyRequested,
}

#[derive(Default)]
enum QuotaFetchProgress {
    #[default]
    Idle,
    Fetching,
    Complete(QuotaFetchCompletion),
}

/// Whether the quota fields need new pane data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::tui) enum QuotaScreenUpdate {
    Unchanged,
    Invalidate,
}

/// Whether a reset fetch is due.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ResetExpiry {
    NoNewExpiry,
    NewlyExpired,
}

enum QuotaPollTrigger {
    Unsignaled,
    Completion,
    Expiry,
}

/// A comparison mark. This value never supplies pane data.
#[derive(Default)]
enum QuotaPresentation {
    #[default]
    AwaitingFirstPoll,
    Shown(GitHubRateLimit),
}

#[derive(Default)]
struct QuotaFetchState {
    progress:      QuotaFetchProgress,
    reset_retries: HashMap<u64, ResetRetrySchedule>,
    presentation:  QuotaPresentation,
}

impl QuotaFetchState {
    fn retain_current_resets(&mut self, quota: &GitHubRateLimit) {
        self.reset_retries
            .retain(|reset, _| quota_resets(quota).any(|current| current == *reset));
    }

    fn claim_due_resets(&mut self, quota: &GitHubRateLimit, now: SystemTime) -> ResetExpiry {
        self.retain_current_resets(quota);
        let mut expiry = ResetExpiry::NoNewExpiry;
        for reset in quota_resets(quota).filter(|reset| *reset <= epoch_seconds(now)) {
            let schedule = match self.reset_retries.get(&reset).copied() {
                Some(ResetRetrySchedule::Due { at, .. }) if at > now => continue,
                Some(ResetRetrySchedule::Exhausted) => continue,
                Some(_) if matches!(self.progress, QuotaFetchProgress::Fetching) => {
                    expiry = ResetExpiry::NewlyExpired;
                    continue;
                },
                Some(schedule) => schedule.after_fetch(now),
                None => ResetRetrySchedule::Due {
                    at:        now + RESET_RETRY_INTERVAL,
                    remaining: RESET_RETRY_COUNT,
                },
            };
            self.reset_retries.insert(reset, schedule);
            expiry = ResetExpiry::NewlyExpired;
        }
        expiry
    }
}

#[derive(Default)]
struct QuotaFetchSync {
    state:    Mutex<QuotaFetchState>,
    complete: Condvar,
}

/// Coalesce quota fetches and schedule bounded retries for expired resets.
#[derive(Clone)]
pub(in crate::tui) struct GithubQuotaRefresh {
    shared:   Arc<QuotaFetchSync>,
    sender:   Sender<QuotaFetchCompletion>,
    receiver: Receiver<QuotaFetchCompletion>,
}

/// A permit owns exactly one quota fetch and its completion notification.
pub(in crate::tui) struct QuotaFetchPermit {
    refresh: GithubQuotaRefresh,
}

impl GithubQuotaRefresh {
    pub(in crate::tui) fn new() -> Self {
        let (sender, receiver) = channel::unbounded();
        Self {
            shared: Arc::new(QuotaFetchSync::default()),
            sender,
            receiver,
        }
    }

    fn screen_update(&self, quota: GitHubRateLimit, now: SystemTime) -> QuotaScreenUpdate {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let current = quota_presentation(quota, now);
        let update = match state.presentation {
            QuotaPresentation::Shown(previous) if previous != current => {
                QuotaScreenUpdate::Invalidate
            },
            QuotaPresentation::AwaitingFirstPoll | QuotaPresentation::Shown(_) => {
                QuotaScreenUpdate::Unchanged
            },
        };
        state.presentation = QuotaPresentation::Shown(current);
        update
    }

    pub(in crate::tui) fn request(
        &self,
        reason: QuotaFetchReason,
        quota: GitHubRateLimit,
        now: SystemTime,
    ) -> QuotaFetchAdmission {
        let presentation = quota_presentation(quota, now);
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if matches!(state.presentation, QuotaPresentation::AwaitingFirstPoll) {
            state.presentation = QuotaPresentation::Shown(presentation);
        }
        let expiry = state.claim_due_resets(&quota, now);
        if matches!(reason, QuotaFetchReason::Reset) && expiry == ResetExpiry::NoNewExpiry {
            return QuotaFetchAdmission::ResetAlreadyRequested;
        }
        if matches!(state.progress, QuotaFetchProgress::Fetching) {
            return QuotaFetchAdmission::InFlight;
        }
        state.progress = QuotaFetchProgress::Fetching;
        drop(state);
        QuotaFetchAdmission::Start(QuotaFetchPermit {
            refresh: self.clone(),
        })
    }

    pub(in crate::tui) fn timeout_at(
        &self,
        quota: GitHubRateLimit,
        now: SystemTime,
    ) -> Option<Duration> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.retain_current_resets(&quota);
        let next = quota_resets(&quota)
            .filter_map(|reset| match state.reset_retries.get(&reset) {
                Some(_) if matches!(state.progress, QuotaFetchProgress::Fetching) => None,
                Some(ResetRetrySchedule::Due { at, .. }) => Some(*at),
                Some(ResetRetrySchedule::Exhausted) => None,
                None => UNIX_EPOCH.checked_add(Duration::from_secs(reset)),
            })
            .min();
        drop(state);
        next.map(|reset| reset.duration_since(now).unwrap_or_default())
    }

    pub(in crate::tui) const fn receiver(&self) -> &Receiver<QuotaFetchCompletion> {
        &self.receiver
    }

    pub(in crate::tui) fn probe_recovery(&self, client: &HttpClient) -> bool {
        let completion = match self.request(
            QuotaFetchReason::Recovery,
            client.rate_limit(),
            SystemTime::now(),
        ) {
            QuotaFetchAdmission::Start(permit) => permit.fetch(client),
            QuotaFetchAdmission::InFlight => {
                let mut state = self
                    .shared
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                while matches!(state.progress, QuotaFetchProgress::Fetching) {
                    state = self
                        .shared
                        .complete
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                match state.progress {
                    QuotaFetchProgress::Complete(completion) => completion,
                    QuotaFetchProgress::Idle | QuotaFetchProgress::Fetching => return false,
                }
            },
            QuotaFetchAdmission::ResetAlreadyRequested => return false,
        };
        let quota = match completion {
            QuotaFetchCompletion::Received(quota) => Ok(quota),
            QuotaFetchCompletion::Failed => Err(ServiceSignal::Unreachable(ServiceKind::GitHub)),
        };
        client.github_recovery_after_quota(quota)
    }
}

impl QuotaFetchPermit {
    pub(in crate::tui) fn fetch(self, client: &HttpClient) -> QuotaFetchCompletion {
        let (quota, signal) = client.fetch_rate_limit();
        let completion = quota.map_or(QuotaFetchCompletion::Failed, QuotaFetchCompletion::Received);
        tracing::debug!(?signal, ?completion, "github_quota_fetch_complete");
        self.complete(completion);
        completion
    }

    pub(in crate::tui) fn complete(self, completion: QuotaFetchCompletion) {
        let mut state = self
            .refresh
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.progress = QuotaFetchProgress::Complete(completion);
        drop(state);
        let _ = self.refresh.sender.send(completion);
        self.refresh.shared.complete.notify_all();
    }
}

impl App {
    pub(in crate::tui) fn github_quota_timeout_at(&self, now: SystemTime) -> Option<Duration> {
        self.net
            .quota_refresh
            .timeout_at(self.net.rate_limit(), now)
    }

    /// Process quota completion and request due reset fetches.
    pub(in crate::tui) fn poll_github_quota_at(&mut self, now: SystemTime) -> QuotaScreenUpdate {
        let mut trigger = QuotaPollTrigger::Unsignaled;
        while let Ok(completion) = self.net.quota_refresh.receiver().try_recv() {
            if matches!(completion, QuotaFetchCompletion::Received(_)) {
                trigger = QuotaPollTrigger::Completion;
            }
        }
        if self
            .github_quota_timeout_at(now)
            .is_some_and(|timeout| timeout.is_zero())
        {
            trigger = QuotaPollTrigger::Expiry;
        }
        let quota = self.net.rate_limit();
        let update = match trigger {
            QuotaPollTrigger::Unsignaled => QuotaScreenUpdate::Unchanged,
            QuotaPollTrigger::Completion | QuotaPollTrigger::Expiry => {
                self.net.quota_refresh.screen_update(quota, now)
            },
        };
        if let QuotaFetchAdmission::Start(permit) =
            self.net
                .quota_refresh
                .request(QuotaFetchReason::Reset, quota, now)
        {
            self.startup_services
                .spawn_github_rate_limit_prime(self.net.http_client(), permit);
        }
        if update == QuotaScreenUpdate::Invalidate {
            self.scan.bump_generation();
        }
        update
    }
}

fn epoch_seconds(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn quota_resets(quota: &GitHubRateLimit) -> impl Iterator<Item = u64> + '_ {
    [quota.core, quota.graphql]
        .into_iter()
        .flatten()
        .filter_map(|bucket| bucket.reset_at)
}

fn quota_presentation(mut quota: GitHubRateLimit, now: SystemTime) -> GitHubRateLimit {
    let now = epoch_seconds(now);
    for bucket in [&mut quota.core, &mut quota.graphql].into_iter().flatten() {
        bucket.used = u64::from(bucket.used != 0);
        bucket.reset_at = bucket
            .reset_at
            .filter(|reset| bucket.used != 0 && *reset > now);
    }
    quota
}
