//! Compile-monitor enablement and selected-scope lifetime.

mod scope;

pub(crate) use scope::MonitorScopeActionability;
pub(crate) use scope::MonitorScopeKey;
pub(crate) use scope::MonitorScopeResolution;
pub(crate) use scope::MonitorScopeUpdate;
pub(crate) use scope::MonitorSelectedRow;
pub(crate) use scope::monitor_scope_input;
pub(crate) use scope::resolve_monitor_scope;

use crate::build_monitor::BuildScopeActionability;
use crate::build_monitor::BuildScopeKey;
pub(crate) use crate::build_monitor::CompileMonitorGeneration;

/// Compile-monitor state owned while visibility is enabled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveMonitorState {
    monitor_selected_row:       MonitorSelectedRow,
    monitor_scope_resolution:   MonitorScopeResolution,
    compile_monitor_generation: CompileMonitorGeneration,
}

impl ActiveMonitorState {
    fn new(
        monitor_scope_update: MonitorScopeUpdate,
        compile_monitor_generation: CompileMonitorGeneration,
    ) -> Self {
        let (monitor_selected_row, monitor_scope_resolution) = monitor_scope_update.into_parts();
        Self {
            monitor_selected_row,
            monitor_scope_resolution,
            compile_monitor_generation,
        }
    }

    /// Whether this scope authorizes build classification, and the roots and
    /// revisions it authorizes it over.
    pub(crate) fn build_scope_actionability(&self) -> BuildScopeActionability {
        build_scope_actionability(&self.monitor_scope_resolution)
    }

    fn requires_replacement(&self, monitor_scope_update: &MonitorScopeUpdate) -> bool {
        self.monitor_selected_row != *monitor_scope_update.monitor_selected_row()
            || self.monitor_scope_resolution != *monitor_scope_update.monitor_scope_resolution()
    }

    fn replace_scope(
        &mut self,
        monitor_scope_update: MonitorScopeUpdate,
        compile_monitor_generation: CompileMonitorGeneration,
    ) {
        let (monitor_selected_row, monitor_scope_resolution) = monitor_scope_update.into_parts();
        self.monitor_selected_row = monitor_selected_row;
        self.monitor_scope_resolution = monitor_scope_resolution;
        self.compile_monitor_generation = compile_monitor_generation;
    }

    #[allow(
        dead_code,
        reason = "Reserved for Build Monitor snapshot and deadline state"
    )]
    pub(crate) const fn monitor_selected_row(&self) -> &MonitorSelectedRow {
        &self.monitor_selected_row
    }

    #[cfg(test)]
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
    On(ActiveMonitorState),
}

impl CompileVisibilityState {
    pub(crate) const fn is_on(&self) -> bool { matches!(self, Self::On(_)) }

    pub(crate) fn enable(
        &mut self,
        monitor_scope_update: MonitorScopeUpdate,
        compile_monitor_generation: CompileMonitorGeneration,
    ) {
        *self = Self::On(ActiveMonitorState::new(
            monitor_scope_update,
            compile_monitor_generation,
        ));
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
    ) {
        if let Self::On(active_monitor_state) = self {
            active_monitor_state.replace_scope(monitor_scope_update, compile_monitor_generation);
        }
    }

    #[allow(dead_code, reason = "Reserved for Build Monitor enablement lifecycle")]
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
        )
    }
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
