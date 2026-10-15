//! Optional startup milestones and census stage timings.

use std::fs::OpenOptions;
use std::io::Stdout;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Instant;

use tui_pane::FramePhase;
use tui_pane::FrameProbe;

use crate::census::AgentRow;
use crate::constants::FRAME_LOG_VARIABLE;

/// Whether startup timings are requested, and where they are recorded.
enum StartupTiming {
    Disabled,
    Recording { started: Instant, path: PathBuf },
}

static STARTUP_TIMING: OnceLock<StartupTiming> = OnceLock::new();
static LOG_WRITE: Mutex<()> = Mutex::new(());
static FIRST_FRAME: AtomicBool = AtomicBool::new(false);
static ANSWER_MILESTONES: StartupAnswerMilestones = StartupAnswerMilestones {
    first_local: AtomicBool::new(false),
    first_agent: AtomicBool::new(false),
};

/// One-time milestones for local answers and the first visible agents.
struct StartupAnswerMilestones {
    first_local: AtomicBool,
    first_agent: AtomicBool,
}

impl StartupAnswerMilestones {
    /// Emit milestones without counting transport-only carriers as agents.
    fn record(&self, rows: &[AgentRow], mut emit: impl FnMut(&str)) {
        if !self.first_local.swap(true, Ordering::Relaxed) {
            emit(&format!("first_local_answer rows={}", rows.len()));
        }
        let visible = rows.iter().filter(|row| row.is_visible()).count();
        if visible > 0 && !self.first_agent.swap(true, Ordering::Relaxed) {
            emit(&format!("first_agent_rows rows={visible}"));
        }
    }
}

/// Record a startup milestone in milliseconds since the app began starting.
pub(crate) fn startup_event(event: &str) {
    let Some(StartupTiming::Recording { started, path }) = STARTUP_TIMING.get() else {
        return;
    };
    let elapsed = started.elapsed().as_millis();
    let _guard = LOG_WRITE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{elapsed}ms {event}");
    }
}

/// Time a census stage only when the startup log is requested.
pub(crate) fn startup_stage<T>(stage: &str, body: impl FnOnce() -> T) -> T {
    if !matches!(STARTUP_TIMING.get(), Some(StartupTiming::Recording { .. })) {
        return body();
    }
    startup_event(&format!("scan_stage_start {stage}"));
    let started = Instant::now();
    let result = body();
    startup_event(&format!(
        "scan_stage_end {stage} elapsed={}ms",
        started.elapsed().as_millis()
    ));
    result
}

/// Record when a local answer is ready, independently of when the UI drains it.
pub(crate) fn startup_local_answer(rows: &[AgentRow]) {
    ANSWER_MILESTONES.record(rows, startup_event);
}

/// Startup milestones supplied by the terminal runner's existing probe hooks.
#[derive(Clone, Copy, Debug)]
pub(crate) enum StartupProbe {}

impl FrameProbe for StartupProbe {
    type Output = Stdout;

    fn output(stdout: Stdout) -> Self::Output {
        startup_event("alternate_screen_entered");
        stdout
    }

    fn timed<T>(phase: FramePhase, body: impl FnOnce() -> T) -> T {
        let result = body();
        if phase == FramePhase::Draw && !FIRST_FRAME.swap(true, Ordering::Relaxed) {
            startup_event("first_frame_flushed");
        }
        result
    }

    fn note(event: &str) { startup_event(event); }
}

/// Start the optional log before configuration is loaded.
pub(crate) fn init() {
    let started = Instant::now();
    let timing = std::env::var_os(FRAME_LOG_VARIABLE).map_or(StartupTiming::Disabled, |path| {
        StartupTiming::Recording {
            started,
            path: PathBuf::from(path),
        }
    });
    let _ = STARTUP_TIMING.set(timing);
    startup_event("process_start");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transport-only answer must not consume the first visible-agent milestone.
    #[test]
    fn transport_hosts_do_not_record_the_first_agent_milestone() {
        let milestones = StartupAnswerMilestones {
            first_local: AtomicBool::new(false),
            first_agent: AtomicBool::new(false),
        };
        let (manager, worker) = crate::census::mesh_worker_fixture();
        let mut events = Vec::new();
        milestones.record(worker.rows(), |event| events.push(event.to_string()));
        assert_eq!(events, ["first_local_answer rows=1"]);
        let mut mixed = worker.rows().to_vec();
        mixed.extend_from_slice(manager.rows());
        milestones.record(&mixed, |event| events.push(event.to_string()));
        milestones.record(manager.rows(), |event| events.push(event.to_string()));
        assert_eq!(
            events,
            ["first_local_answer rows=1", "first_agent_rows rows=1"]
        );
    }
}
