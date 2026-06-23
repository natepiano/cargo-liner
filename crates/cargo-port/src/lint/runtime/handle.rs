use super::AbsolutePath;
use super::Arc;
use super::Child;
#[cfg(test)]
use super::JoinHandle;
use super::LintEventKind;
use super::LintTriggerEvent;
use super::LintTriggerKind;
use super::Mutex;
use super::RegisterProjectRequest;
use super::StdSender;

#[derive(Clone)]
pub struct RuntimeHandle {
    pub(super) supervisor_sender: StdSender<SupervisorMsg>,
}

impl RuntimeHandle {
    pub fn sync_projects(&self, projects: Vec<RegisterProjectRequest>) {
        let _ = self
            .supervisor_sender
            .send(SupervisorMsg::SyncProjects { projects });
    }

    pub fn register_project(&self, project: RegisterProjectRequest) {
        let _ = self
            .supervisor_sender
            .send(SupervisorMsg::RegisterProject { project });
    }

    pub fn unregister_project(&self, abs_path: AbsolutePath) {
        let _ = self
            .supervisor_sender
            .send(SupervisorMsg::UnregisterProject { abs_path });
    }

    pub fn lint_trigger(&self, event: LintTriggerEvent) {
        let _ = self
            .supervisor_sender
            .send(SupervisorMsg::LintTriggered { event });
    }

    /// Pause all lint work: kill every in-flight run and hold new runs until
    /// [`Self::resume`]. Projects whose runs are killed or whose triggers
    /// arrive while paused are remembered and re-linted on resume.
    pub fn pause(&self) { let _ = self.supervisor_sender.send(SupervisorMsg::Pause); }

    /// Resume lint work and re-dispatch the catch-up runs accumulated while
    /// paused (same `CatchUp` origin as the startup staleness sweep).
    pub fn resume(&self) { let _ = self.supervisor_sender.send(SupervisorMsg::Resume); }

    /// Schedule a lint run for a project the app's post-startup staleness
    /// check flagged (source newer than the last run, or never linted under
    /// immediate discovery). Routed through the same `LintTriggered` path as
    /// watcher events so the worker debounces and coalesces it normally.
    pub fn request_startup_lint(&self, project_root: AbsolutePath) {
        let _ = self.supervisor_sender.send(SupervisorMsg::LintTriggered {
            event: LintTriggerEvent {
                project_root,
                trigger: LintTriggerKind::Startup,
                event_kind: LintEventKind::CreateOrModify,
            },
        });
    }
}

pub struct SpawnResult {
    pub handle:            Option<RuntimeHandle>,
    pub warning:           Option<String>,
    #[cfg(test)]
    pub(crate) supervisor: Option<JoinHandle<()>>,
}

pub(super) enum SupervisorMsg {
    SyncProjects {
        projects: Vec<RegisterProjectRequest>,
    },
    RegisterProject {
        project: RegisterProjectRequest,
    },
    UnregisterProject {
        abs_path: AbsolutePath,
    },
    LintTriggered {
        event: LintTriggerEvent,
    },
    Pause,
    Resume,
}

pub(super) type ChildSlot = Arc<Mutex<Option<Child>>>;
