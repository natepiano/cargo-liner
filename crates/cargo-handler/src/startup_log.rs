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
static FIRST_LOCAL: AtomicBool = AtomicBool::new(false);
static FIRST_AGENT: AtomicBool = AtomicBool::new(false);

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
    if !FIRST_LOCAL.swap(true, Ordering::Relaxed) {
        startup_event(&format!("first_local_answer rows={}", rows.len()));
    }
    if !rows.is_empty() && !FIRST_AGENT.swap(true, Ordering::Relaxed) {
        startup_event(&format!("first_agent_rows rows={}", rows.len()));
    }
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
