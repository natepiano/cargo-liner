//! Compile-monitor enablement and selected-scope lifetime.

mod constants;
mod scope;

use std::time::Instant;

use constants::COMPILE_MONITOR_REFRESH_INTERVAL;
pub(crate) use scope::MonitorScopeActionability;
pub(crate) use scope::MonitorScopeKey;
pub(crate) use scope::MonitorScopeResolution;
#[cfg(test)]
pub(crate) use scope::MonitorScopeResolutionRevision;
pub(crate) use scope::MonitorScopeUpdate;
pub(crate) use scope::MonitorSelectedRow;
pub(crate) use scope::MonitorWorkspaceIndexReadiness;
pub(crate) use scope::monitor_scope_input;
pub(crate) use scope::resolve_monitor_scope;

use crate::build_monitor::BuildScopeActionability;
use crate::build_monitor::BuildScopeKey;
pub(crate) use crate::build_monitor::CompileMonitorGeneration;
use crate::process_observation::CompileMonitorRefreshSchedule;

/// When the enabled monitor next owes a refresh, and whether the one it asked
/// for is still out.
///
/// This is the monitor's own scheduling intent. The executor's
/// [`CompileMonitorRefreshSchedule`] is the dispatch deadline derived from it
/// and is never read back as truth, so the two cannot disagree about whether a
/// refresh is owed. A disabled monitor owns no schedule at all, because
/// [`CompileVisibilityState::Off`] drops this whole aggregate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MonitorRefreshSchedule {
    /// Nothing is out; the next refresh is owed at this instant.
    DueAt(Instant),
    /// A cycle for this generation is with the worker; when it settles the
    /// next refresh is owed from this interval boundary.
    InFlight {
        generation: CompileMonitorGeneration,
        rearm_at:   Instant,
    },
}

/// Whether a cycle is with the worker, and the generation it was dispatched
/// under.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InFlightMonitorGeneration {
    InFlight(CompileMonitorGeneration),
    NoneInFlight,
}

/// Compile-monitor state owned while visibility is enabled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveMonitorState {
    monitor_selected_row:       MonitorSelectedRow,
    monitor_scope_resolution:   MonitorScopeResolution,
    compile_monitor_generation: CompileMonitorGeneration,
    monitor_refresh_schedule:   MonitorRefreshSchedule,
}

impl ActiveMonitorState {
    fn new(
        monitor_scope_update: MonitorScopeUpdate,
        compile_monitor_generation: CompileMonitorGeneration,
        now: Instant,
    ) -> Self {
        let (monitor_selected_row, monitor_scope_resolution) = monitor_scope_update.into_parts();
        Self {
            monitor_selected_row,
            monitor_scope_resolution,
            compile_monitor_generation,
            monitor_refresh_schedule: MonitorRefreshSchedule::DueAt(now),
        }
    }

    /// The dispatch deadline the shared executor should hold for this monitor.
    ///
    /// A scope that authorizes no classification owes no deadline: the cycle it
    /// would dispatch produces [`crate::build_monitor::CompileClassificationDemand::NotRequested`],
    /// so nothing would record the dispatch or the settle and the deadline would
    /// stay in the past, waking the event loop with a zero timeout every tick.
    pub(crate) fn compile_monitor_refresh_schedule(&self) -> CompileMonitorRefreshSchedule {
        if self.build_scope_actionability() == BuildScopeActionability::NotActionable {
            return CompileMonitorRefreshSchedule::NotScheduled;
        }
        match self.monitor_refresh_schedule {
            MonitorRefreshSchedule::DueAt(due_at) => CompileMonitorRefreshSchedule::At(due_at),
            MonitorRefreshSchedule::InFlight { .. } => CompileMonitorRefreshSchedule::NotScheduled,
        }
    }

    /// The generation of the cycle currently with the worker.
    ///
    /// A whole-cycle failure carries no classification to name the generation it
    /// ran under, so this is what tells that failure apart from one belonging to
    /// a generation the monitor has already moved past — enabling and every
    /// scope replacement reset the schedule to [`MonitorRefreshSchedule::DueAt`].
    const fn in_flight_generation(&self) -> InFlightMonitorGeneration {
        match self.monitor_refresh_schedule {
            MonitorRefreshSchedule::InFlight { generation, .. } => {
                InFlightMonitorGeneration::InFlight(generation)
            },
            MonitorRefreshSchedule::DueAt(_) => InFlightMonitorGeneration::NoneInFlight,
        }
    }

    /// Record that this generation's refresh went out, and fix the interval
    /// boundary its replacement is owed from.
    fn record_dispatch(&mut self, now: Instant) {
        self.monitor_refresh_schedule = MonitorRefreshSchedule::InFlight {
            generation: self.compile_monitor_generation,
            rearm_at:   next_refresh_boundary(now, now),
        };
    }

    /// Record that the in-flight refresh settled, rearming from the interval
    /// boundary. Missed instants coalesce into one next demand rather than
    /// queueing a catch-up per interval that elapsed.
    fn record_settled(&mut self, now: Instant) {
        if let MonitorRefreshSchedule::InFlight { rearm_at, .. } = self.monitor_refresh_schedule {
            self.monitor_refresh_schedule =
                MonitorRefreshSchedule::DueAt(next_refresh_boundary(rearm_at, now));
        }
    }

    /// Whether this scope authorizes build classification, and the roots and
    /// revisions it authorizes it over.
    pub(crate) fn build_scope_actionability(&self) -> BuildScopeActionability {
        build_scope_actionability(self.monitor_scope_resolution())
    }

    fn requires_replacement(&self, monitor_scope_update: &MonitorScopeUpdate) -> bool {
        self.monitor_selected_row != *monitor_scope_update.monitor_selected_row()
            || self.monitor_scope_resolution != *monitor_scope_update.monitor_scope_resolution()
    }

    fn replace_scope(
        &mut self,
        monitor_scope_update: MonitorScopeUpdate,
        compile_monitor_generation: CompileMonitorGeneration,
        now: Instant,
    ) {
        let (monitor_selected_row, monitor_scope_resolution) = monitor_scope_update.into_parts();
        self.monitor_selected_row = monitor_selected_row;
        self.monitor_scope_resolution = monitor_scope_resolution;
        self.compile_monitor_generation = compile_monitor_generation;
        self.monitor_refresh_schedule = MonitorRefreshSchedule::DueAt(now);
    }

    /// The named resolution this scope produced, and the one input
    /// [`build_scope_actionability`] reads to decide what may be classified.
    pub(crate) const fn monitor_scope_resolution(&self) -> &MonitorScopeResolution {
        &self.monitor_scope_resolution
    }

    pub(crate) const fn compile_monitor_generation(&self) -> CompileMonitorGeneration {
        self.compile_monitor_generation
    }
}

/// Whether compile-monitor visibility owns an active selected-scope aggregate.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum CompileVisibilityState {
    #[default]
    Off,
    /// Boxed because the active aggregate carries the whole resolved scope —
    /// covered roots plus the metadata, project-list, and live target-directory
    /// revisions — and `Off` carries nothing.
    On(Box<ActiveMonitorState>),
}

impl CompileVisibilityState {
    pub(crate) const fn is_on(&self) -> bool { matches!(self, Self::On(_)) }

    /// A monitor switched on over the scope resolution a test names.
    #[cfg(test)]
    pub(crate) fn on_for_test(monitor_scope_resolution: MonitorScopeResolution) -> Self {
        Self::On(Box::new(ActiveMonitorState {
            monitor_selected_row: MonitorSelectedRow::NoVisibleRow,
            monitor_scope_resolution,
            compile_monitor_generation: CompileMonitorGeneration::default(),
            monitor_refresh_schedule: MonitorRefreshSchedule::DueAt(Instant::now()),
        }))
    }

    pub(crate) fn enable(
        &mut self,
        monitor_scope_update: MonitorScopeUpdate,
        compile_monitor_generation: CompileMonitorGeneration,
        now: Instant,
    ) {
        *self = Self::On(Box::new(ActiveMonitorState::new(
            monitor_scope_update,
            compile_monitor_generation,
            now,
        )));
    }

    /// The dispatch deadline the shared executor should hold. A disabled
    /// monitor contributes none, so no compile-specific wakeup exists while it
    /// is off.
    pub(crate) fn compile_monitor_refresh_schedule(&self) -> CompileMonitorRefreshSchedule {
        match self {
            Self::Off => CompileMonitorRefreshSchedule::NotScheduled,
            Self::On(active_monitor_state) => {
                active_monitor_state.compile_monitor_refresh_schedule()
            },
        }
    }

    /// The generation of the cycle the monitor is still waiting on, for a
    /// whole-cycle failure that names no generation of its own.
    pub(crate) const fn in_flight_generation(&self) -> InFlightMonitorGeneration {
        match self {
            Self::Off => InFlightMonitorGeneration::NoneInFlight,
            Self::On(active_monitor_state) => active_monitor_state.in_flight_generation(),
        }
    }

    /// Record that a compile refresh went out for the enabled monitor.
    pub(crate) fn record_refresh_dispatch(&mut self, now: Instant) {
        if let Self::On(active_monitor_state) = self {
            active_monitor_state.record_dispatch(now);
        }
    }

    /// Record that the in-flight compile refresh settled, however it settled.
    pub(crate) fn record_refresh_settled(&mut self, now: Instant) {
        if let Self::On(active_monitor_state) = self {
            active_monitor_state.record_settled(now);
        }
    }

    pub(crate) fn disable(&mut self) { *self = Self::Off; }

    pub(crate) fn requires_scope_replacement(
        &self,
        monitor_scope_update: &MonitorScopeUpdate,
    ) -> bool {
        match self {
            Self::Off => false,
            Self::On(active_monitor_state) => {
                active_monitor_state.requires_replacement(monitor_scope_update)
            },
        }
    }

    pub(crate) fn replace_scope(
        &mut self,
        monitor_scope_update: MonitorScopeUpdate,
        compile_monitor_generation: CompileMonitorGeneration,
        now: Instant,
    ) {
        if let Self::On(active_monitor_state) = self {
            active_monitor_state.replace_scope(
                monitor_scope_update,
                compile_monitor_generation,
                now,
            );
        }
    }

    /// Whether a result that names this generation may still be applied. A
    /// toggle or scope replacement advances past the generation it cancelled,
    /// so a late result from it is refused here.
    pub(crate) fn accepts_generation(
        &self,
        compile_monitor_generation: CompileMonitorGeneration,
    ) -> bool {
        match self {
            Self::Off => false,
            Self::On(active_monitor_state) => {
                active_monitor_state.compile_monitor_generation() == compile_monitor_generation
            },
        }
    }
}

/// Drop the selected-row identity and keep the roots and revisions build
/// classification is allowed to see.
///
/// The conversion lives here rather than beside [`BuildScopeKey`] because
/// [`MonitorScopeKey`] never leaves `crate::tui`. It reads the canonical roots
/// through their accessors, which already carry the sort-and-dedup invariant
/// established when the scope was resolved, so it must not re-sort them.
impl From<&MonitorScopeKey> for BuildScopeKey {
    fn from(monitor_scope_key: &MonitorScopeKey) -> Self {
        Self::from_covered_scope_roots(
            monitor_scope_key.covered_scope_roots().clone(),
            monitor_scope_key.accepted_cargo_metadata_revision(),
            monitor_scope_key.project_list_revision(),
            monitor_scope_key.live_target_directory_revision().clone(),
        )
    }
}

/// The first interval boundary at or after `now`, measured from `boundary`.
///
/// A tick that arrives several intervals late coalesces every missed instant
/// into this one next demand instead of building a queue of catch-up refreshes.
fn next_refresh_boundary(boundary: Instant, now: Instant) -> Instant {
    if boundary > now {
        return boundary;
    }
    let elapsed_intervals = u32::try_from(
        now.duration_since(boundary).as_nanos() / COMPILE_MONITOR_REFRESH_INTERVAL.as_nanos(),
    )
    .unwrap_or(u32::MAX);
    boundary + COMPILE_MONITOR_REFRESH_INTERVAL * elapsed_intervals.saturating_add(1)
}

/// The one entry point through which a monitor scope reaches build
/// classification. Routing through [`MonitorScopeResolution::actionability`]
/// keeps the five resolution states from being restated anywhere downstream.
pub(crate) fn build_scope_actionability(
    monitor_scope_resolution: &MonitorScopeResolution,
) -> BuildScopeActionability {
    match monitor_scope_resolution.actionability() {
        MonitorScopeActionability::Actionable(monitor_scope_key) => {
            BuildScopeActionability::Actionable(BuildScopeKey::from(monitor_scope_key))
        },
        MonitorScopeActionability::NotActionable => BuildScopeActionability::NotActionable,
    }
}
