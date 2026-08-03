//! Compile-monitor enablement and selected-scope lifetime.

mod scope;

pub(crate) use scope::MonitorScopeResolution;
pub(crate) use scope::MonitorScopeUpdate;
pub(crate) use scope::MonitorSelectedRow;
pub(crate) use scope::monitor_scope_input;
pub(crate) use scope::resolve_monitor_scope;

/// Monotonic identity for one compile-monitor scope lifetime.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct CompileMonitorGeneration(u64);

impl CompileMonitorGeneration {
    pub(crate) const fn advance(&mut self) { self.0 = self.0.saturating_add(1); }
}

/// Compile-monitor state owned while visibility is enabled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveMonitorState {
    monitor_selected_row:       MonitorSelectedRow,
    monitor_scope_resolution:   MonitorScopeResolution,
    compile_monitor_generation: CompileMonitorGeneration,
}

impl ActiveMonitorState {
    #[allow(
        dead_code,
        reason = "Reserved for Build Monitor snapshot and deadline state"
    )]
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

    #[allow(
        dead_code,
        reason = "Reserved for Build Monitor snapshot and deadline state"
    )]
    pub(crate) const fn monitor_scope_resolution(&self) -> &MonitorScopeResolution {
        &self.monitor_scope_resolution
    }

    #[allow(
        dead_code,
        reason = "Reserved for Build Monitor snapshot and deadline state"
    )]
    pub(crate) const fn compile_monitor_generation(&self) -> CompileMonitorGeneration {
        self.compile_monitor_generation
    }
}

/// Whether compile-monitor visibility owns an active selected-scope aggregate.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum CompileVisibilityState {
    #[default]
    Off,
    #[allow(dead_code, reason = "Reserved for Build Monitor enablement lifecycle")]
    On(ActiveMonitorState),
}

impl CompileVisibilityState {
    pub(crate) const fn is_on(&self) -> bool { matches!(self, Self::On(_)) }

    #[allow(dead_code, reason = "Reserved for Build Monitor enablement lifecycle")]
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

    #[allow(dead_code, reason = "Reserved for Build Monitor enablement lifecycle")]
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
