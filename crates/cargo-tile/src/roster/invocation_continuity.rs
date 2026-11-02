//! One row owner retains process aliases, lifecycle data, and work readings.

use std::time::Instant;

use super::invocation_owners::InvocationKey;
use crate::census::CargoProcess;
use crate::census::InvocationId;
use crate::census::Measurement;
use crate::census::invocation_cpu_accounting::LiveResources;
use crate::census::scan::InvocationExecution;
use crate::census::scan::OriginStart;

/// The display borrows the snapshot owned by the lifecycle state.
#[derive(Clone)]
pub(crate) struct InvocationSnapshot {
    pub(crate) process: CargoProcess,
}

/// Accepted observations freeze between scans; retained clocks continue between scans.
#[derive(Clone, Copy)]
enum ClockObservation {
    Incoming(Instant),
    Retained(Instant),
}

impl ClockObservation {
    const fn time(self) -> Instant {
        match self {
            Self::Incoming(time) | Self::Retained(time) => time,
        }
    }
}

/// A live state owns the origin clock and the last valid work readings.
#[derive(Clone)]
struct LiveInvocation {
    snapshot: InvocationSnapshot,
    readings: WorkReadingHistory,
    observed: ClockObservation,
}

/// A draining origin owns the retained executor snapshot.
#[derive(Clone)]
struct DrainingInvocation {
    snapshot: InvocationSnapshot,
    observed: ClockObservation,
}

/// A finished invocation owns its final snapshot and its fade origin.
#[derive(Clone)]
struct FinishedInvocation {
    snapshot: InvocationSnapshot,
    ended:    Instant,
}

#[derive(Clone)]
enum InvocationState {
    Starting(LiveInvocation),
    Running(LiveInvocation),
    Draining(DrainingInvocation),
    Finished(FinishedInvocation),
}

/// The lifecycle state owns every retained value for one immutable row key.
#[derive(Clone)]
pub(super) struct InvocationContinuity {
    key:   InvocationKey,
    state: InvocationState,
}

#[derive(Clone, Copy)]
pub(super) enum InvocationCompletion {
    Live,
    Finished(Instant),
}

impl InvocationContinuity {
    pub(super) fn new(process: CargoProcess, now: Instant) -> Self {
        let key = InvocationKey(process.invocation_id.clone());
        let starting = matches!(
            process.execution,
            InvocationExecution::AwaitingExecutor(_) | InvocationExecution::RegistrationOnly
        );
        let live = LiveInvocation {
            readings: WorkReadingHistory::new(),
            snapshot: InvocationSnapshot { process },
            observed: ClockObservation::Incoming(now),
        };
        Self {
            key,
            state: if starting {
                InvocationState::Starting(live)
            } else {
                InvocationState::Running(live)
            },
        }
    }

    pub(super) const fn key(&self) -> &InvocationId { &self.key.0 }

    pub(super) const fn snapshot(&self) -> &InvocationSnapshot {
        match &self.state {
            InvocationState::Starting(live) | InvocationState::Running(live) => &live.snapshot,
            InvocationState::Draining(draining) => &draining.snapshot,
            InvocationState::Finished(finished) => &finished.snapshot,
        }
    }

    pub(super) const fn snapshot_mut(&mut self) -> &mut InvocationSnapshot {
        match &mut self.state {
            InvocationState::Starting(live) | InvocationState::Running(live) => &mut live.snapshot,
            InvocationState::Draining(draining) => &mut draining.snapshot,
            InvocationState::Finished(finished) => &mut finished.snapshot,
        }
    }

    pub(super) const fn completion(&self) -> InvocationCompletion {
        match &self.state {
            InvocationState::Finished(finished) => InvocationCompletion::Finished(finished.ended),
            InvocationState::Starting(_)
            | InvocationState::Running(_)
            | InvocationState::Draining(_) => InvocationCompletion::Live,
        }
    }

    pub(super) fn refresh(&mut self, mut process: CargoProcess, now: Instant) {
        let previous = &self.snapshot().process;
        let (start, observed) = match &self.state {
            InvocationState::Starting(live) | InvocationState::Running(live) => {
                (live.snapshot.process.origin_start, live.observed.time())
            },
            InvocationState::Draining(draining) => (
                draining.snapshot.process.origin_start,
                draining.observed.time(),
            ),
            InvocationState::Finished(_) => (process.origin_start, now),
        };
        let elapsed = now.saturating_duration_since(observed);
        let retaining = previous.pid == process.pid
            && !matches!(process.execution, InvocationExecution::Command)
            && start.retains(process.origin_start);
        let observed = if retaining {
            ClockObservation::Retained(now)
        } else {
            ClockObservation::Incoming(now)
        };
        let start = if retaining {
            start.advance(process.origin_start, elapsed)
        } else {
            process.origin_start
        };
        if !matches!(process.execution, InvocationExecution::Command) {
            process.origin_start = start;
        }
        process.invocation_id.clone_from(&self.key.0);
        let draining = matches!(process.execution, InvocationExecution::AwaitingExecutor(_))
            && matches!(
                self.state,
                InvocationState::Running(_) | InvocationState::Draining(_)
            );
        let mut readings = match &self.state {
            InvocationState::Starting(live) | InvocationState::Running(live) => {
                live.readings.clone()
            },
            InvocationState::Draining(_) | InvocationState::Finished(_) => {
                WorkReadingHistory::new()
            },
        };
        if draining {
            process.resource_use.clone_from(&previous.resource_use);
            process
                .subtree_resource_use
                .clone_from(&previous.subtree_resource_use);
            readings.restore(&mut process);
            self.state = InvocationState::Draining(DrainingInvocation {
                snapshot: InvocationSnapshot { process },
                observed,
            });
        } else {
            readings.observe(previous, &process);
            let starting = matches!(
                process.execution,
                InvocationExecution::AwaitingExecutor(_) | InvocationExecution::RegistrationOnly
            );
            let live = LiveInvocation {
                snapshot: InvocationSnapshot { process },
                readings,
                observed,
            };
            self.state = if starting {
                InvocationState::Starting(live)
            } else {
                InvocationState::Running(live)
            };
        }
    }

    /// Identical scans advance only clocks retained from an earlier observation.
    pub(super) fn advance_retained_clock(&mut self, now: Instant) -> bool {
        let (snapshot, observed) = match &mut self.state {
            InvocationState::Starting(live) | InvocationState::Running(live) => {
                (&mut live.snapshot, &mut live.observed)
            },
            InvocationState::Draining(draining) => (&mut draining.snapshot, &mut draining.observed),
            InvocationState::Finished(_) => return false,
        };
        let ClockObservation::Retained(previous) = *observed else {
            return false;
        };
        let start = snapshot.process.origin_start;
        let advanced = start.advance(
            OriginStart::Unavailable,
            now.saturating_duration_since(previous),
        );
        snapshot.process.origin_start = advanced;
        *observed = ClockObservation::Retained(now);
        !start.same_duration(advanced)
    }

    pub(super) fn finish(&mut self, now: Instant) {
        if matches!(self.completion(), InvocationCompletion::Finished(_)) {
            return;
        }
        let mut snapshot = self.snapshot().clone();
        if let InvocationState::Starting(live) | InvocationState::Running(live) = &self.state {
            live.readings.restore(&mut snapshot.process);
        }
        self.state = InvocationState::Finished(FinishedInvocation {
            snapshot,
            ended: now,
        });
    }
}

/// Only a valid work reading replaces retained work evidence.
#[derive(Clone)]
struct WorkReadingHistory {
    cpu:            LiveResources<f32>,
    memory:         LiveResources<u64>,
    subtree_cpu:    LiveResources<f32>,
    subtree_memory: LiveResources<u64>,
}

impl WorkReadingHistory {
    const fn new() -> Self {
        Self {
            cpu:            LiveResources::Sampling,
            memory:         LiveResources::Sampling,
            subtree_cpu:    LiveResources::Sampling,
            subtree_memory: LiveResources::Sampling,
        }
    }

    const fn observe(&mut self, previous: &CargoProcess, incoming: &CargoProcess) {
        retain_reading(
            &mut self.cpu,
            previous.resource_use.cpu,
            incoming.resource_use.cpu,
        );
        retain_reading(
            &mut self.memory,
            previous.resource_use.memory,
            incoming.resource_use.memory,
        );
        retain_reading(
            &mut self.subtree_cpu,
            previous.subtree_resource_use.cpu,
            incoming.subtree_resource_use.cpu,
        );
        retain_reading(
            &mut self.subtree_memory,
            previous.subtree_resource_use.memory,
            incoming.subtree_resource_use.memory,
        );
    }

    fn restore(&self, process: &mut CargoProcess) {
        restore_reading(self.cpu, &mut process.resource_use.cpu);
        restore_reading(self.memory, &mut process.resource_use.memory);
        restore_reading(self.subtree_cpu, &mut process.subtree_resource_use.cpu);
        restore_reading(
            self.subtree_memory,
            &mut process.subtree_resource_use.memory,
        );
    }
}

const fn retain_reading<T: Copy>(
    retained: &mut LiveResources<T>,
    previous: Measurement<T>,
    incoming: Measurement<T>,
) {
    match (previous, incoming) {
        (_, Measurement::Reading(_)) => *retained = LiveResources::Sampling,
        (Measurement::Reading(value), Measurement::Unavailable(_)) => {
            *retained = LiveResources::Reading(value);
        },
        (Measurement::Unavailable(_), Measurement::Unavailable(_)) => {},
    }
}

fn restore_reading<T>(retained: LiveResources<T>, current: &mut Measurement<T>) {
    if let LiveResources::Reading(reading) = retained {
        *current = Measurement::Reading(reading);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::census::scan::CensusSequence;
    use crate::census::scan::tests::FixtureExecution;
    use crate::census::scan::tests::ShellRun;

    #[test]
    fn a_retained_registration_clock_accumulates_fractional_refresh_time()
    -> Result<(), Box<dyn std::error::Error>> {
        let run = ShellRun::new();
        let root = tempfile::tempdir()?;
        let capture = ShellRun::capture(root.path());
        let mut sequence = CensusSequence::default();
        let mut groups = sequence.sample_capture(&run.records(FixtureExecution::Working), &capture);
        let mut process = groups.remove(0).lead;
        process.origin_start = OriginStart::Registration {
            epoch:   100,
            elapsed: Duration::from_secs(0),
        };
        let now = Instant::now();
        let mut continuity = InvocationContinuity::new(process.clone(), now);
        assert_eq!(continuity.snapshot().process.duration(), "00:00");
        process.origin_start = OriginStart::Unavailable;
        for refresh in 1..=12 {
            process.resource_use.memory = Measurement::Reading(refresh);
            continuity.refresh(process.clone(), now + Duration::from_millis(refresh * 100));
        }
        assert_eq!(continuity.snapshot().process.duration(), "00:01");
        Ok(())
    }
}
