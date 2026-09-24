use std::env;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::io;
use std::io::IsTerminal;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::thread::JoinHandle;

use super::BuildOutputMode;
use crate::compiler::analyzing;
use crate::compiler::constants::CARGO_TERM_PROGRESS_WHEN_ALWAYS;
use crate::compiler::constants::CARGO_TERM_PROGRESS_WHEN_ENV;
use crate::compiler::constants::CARGO_TERM_PROGRESS_WHEN_NEVER;
use crate::compiler::constants::PROGRESS_BAR_FILL;
use crate::compiler::constants::PROGRESS_BAR_HEAD;
use crate::compiler::constants::PROGRESS_BAR_WIDTH;
use crate::compiler::constants::PROGRESS_FRAMES;
use crate::compiler::constants::PROGRESS_INTERVAL;

pub(super) trait ProgressDisplay {
    fn is_active(&self) -> bool;

    fn record_unit_counter(&mut self, unit_counter: UnitCounter);

    fn write_status_notice(&mut self, notice: &str);

    fn stop_for_forwarded_output(&mut self);
}

/// Cargo's count of its build plan, as its progress bar draws it: units finished
/// out of units planned.
///
/// mend analyzes a workspace member inside that member's own `rustc` process, so
/// cargo counts a member's unit as finished only once its analysis is, and the
/// counter covers mend's work as well as the compiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct UnitCounter {
    pub(super) done:  usize,
    pub(super) total: usize,
}

/// Whether the status line is drawn, read from cargo's own
/// `CARGO_TERM_PROGRESS_WHEN` so mend draws its line wherever cargo would draw
/// its bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProgressWhen {
    Always,
    Auto,
    Never,
}

/// How often the status line is redrawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Redraw {
    /// A terminal shows only the latest frame, so the spinner turns on every tick.
    EveryTick,
    /// A pipe keeps every frame it is sent, so a frame is written only when the
    /// status text changes, as cargo writes its own bar.
    OnChange,
}

/// The line the spinner thread draws on. It sits behind the lock every write to
/// stderr takes, so the spinner thread and the stderr reader never interleave.
#[derive(Default)]
struct StatusLine {
    /// The counter from cargo's latest bar frame; `None` until cargo draws one.
    unit_counter: Option<UnitCounter>,
    /// The status text drawn last, without its spinner glyph; `None` once erased.
    /// It names the targets under analysis, so its length changes between frames.
    drawn:        Option<String>,
}

pub(super) struct CargoProgress {
    state: Option<CargoProgressState>,
}

struct CargoProgressState {
    active:      Arc<AtomicBool>,
    status_line: Arc<Mutex<StatusLine>>,
    handle:      Option<JoinHandle<()>>,
}

impl CargoProgress {
    pub(super) fn start(output_mode: BuildOutputMode, analyzing_dir: &Path) -> Self {
        let Some(message) = progress_message_for(output_mode) else {
            return Self { state: None };
        };
        let Some(redraw) = ProgressWhen::from_env().redraw() else {
            return Self { state: None };
        };

        let active = Arc::new(AtomicBool::new(true));
        let status_line = Arc::new(Mutex::new(StatusLine::default()));
        let thread_active = Arc::clone(&active);
        let thread_status_line = Arc::clone(&status_line);
        let thread_analyzing_dir = analyzing_dir.to_path_buf();
        let handle = thread::spawn(move || {
            let mut frame_index = 0;
            while thread_active.load(Ordering::Relaxed) {
                let targets = analyzing::targets_in_flight(&thread_analyzing_dir);
                if let Ok(mut status_line) = thread_status_line.lock() {
                    let status = analyzing_status(&targets, message, status_line.unit_counter);
                    if status_line.needs_redraw(&status, redraw) {
                        status_line.draw(status, frame_index);
                        frame_index = (frame_index + 1) % PROGRESS_FRAMES.len();
                    }
                }
                thread::sleep(PROGRESS_INTERVAL);
            }
        });

        Self {
            state: Some(CargoProgressState {
                active,
                status_line,
                handle: Some(handle),
            }),
        }
    }

    fn stop(&mut self) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        state.active.store(false, Ordering::Relaxed);
        if let Some(handle) = state.handle.take() {
            let _ = handle.join();
        }
        state.clear_line();
        self.state = None;
    }
}

impl Drop for CargoProgress {
    fn drop(&mut self) { self.stop(); }
}

impl ProgressDisplay for CargoProgress {
    fn is_active(&self) -> bool { self.state.is_some() }

    fn record_unit_counter(&mut self, unit_counter: UnitCounter) {
        if let Some(state) = self.state.as_ref()
            && let Ok(mut status_line) = state.status_line.lock()
        {
            status_line.unit_counter = Some(unit_counter);
        }
    }

    fn write_status_notice(&mut self, notice: &str) {
        if let Some(state) = self.state.as_ref() {
            state.write_status_notice(notice);
        } else {
            eprintln!("{notice}");
        }
    }

    fn stop_for_forwarded_output(&mut self) { self.stop(); }
}

impl CargoProgressState {
    fn clear_line(&self) {
        if let Ok(mut status_line) = self.status_line.lock() {
            status_line.erase();
            let _ = io::stderr().flush();
        }
    }

    fn write_status_notice(&self, notice: &str) {
        if let Ok(mut status_line) = self.status_line.lock() {
            status_line.erase();
            eprintln!("{notice}");
            let _ = io::stderr().flush();
        }
    }
}

impl StatusLine {
    fn needs_redraw(&self, status: &str, redraw: Redraw) -> bool {
        match redraw {
            Redraw::EveryTick => true,
            Redraw::OnChange => self.drawn.as_deref() != Some(status),
        }
    }

    /// Draws `status` over the frame drawn last, padding out to that frame's
    /// width so a shorter status does not leave the tail of a longer one behind.
    fn draw(&mut self, status: String, frame_index: usize) {
        let previous = self.drawn.as_deref().map_or(0, progress_line_width);
        let width = progress_line_width(&status);
        eprint!(
            "{}{}",
            progress_frame(&status, frame_index),
            " ".repeat(previous.saturating_sub(width))
        );
        let _ = io::stderr().flush();
        self.drawn = Some(status);
    }

    fn erase(&mut self) {
        if let Some(drawn) = self.drawn.take() {
            eprint!("{}", clear_progress_line(progress_line_width(&drawn)));
        }
    }
}

impl ProgressWhen {
    fn from_env() -> Self {
        match env::var(CARGO_TERM_PROGRESS_WHEN_ENV).as_deref() {
            Ok(CARGO_TERM_PROGRESS_WHEN_ALWAYS) => Self::Always,
            Ok(CARGO_TERM_PROGRESS_WHEN_NEVER) => Self::Never,
            _ => Self::Auto,
        }
    }

    /// How the status line is redrawn under this setting, or `None` when it is
    /// not drawn at all.
    fn redraw(self) -> Option<Redraw> {
        match (self, io::stderr().is_terminal()) {
            (Self::Never, _) | (Self::Auto, false) => None,
            (Self::Always | Self::Auto, true) => Some(Redraw::EveryTick),
            (Self::Always, false) => Some(Redraw::OnChange),
        }
    }
}

/// Draws the counter as cargo draws its own bar, `[=====>     ] 149/403`, which
/// is also the shape `cargo tile` reads a captured run's progress out of.
impl Display for UnitCounter {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let filled = PROGRESS_BAR_WIDTH
            .saturating_mul(self.done)
            .checked_div(self.total)
            .unwrap_or(0)
            .min(PROGRESS_BAR_WIDTH);
        let head = match filled {
            0 => "",
            _ if self.done >= self.total => PROGRESS_BAR_FILL,
            _ => PROGRESS_BAR_HEAD,
        };
        write!(
            formatter,
            "[{}{head}{}] {}/{}",
            PROGRESS_BAR_FILL.repeat(filled.saturating_sub(1)),
            " ".repeat(PROGRESS_BAR_WIDTH - filled),
            self.done,
            self.total
        )
    }
}

const fn progress_message_for(output_mode: BuildOutputMode) -> Option<&'static str> {
    match output_mode {
        BuildOutputMode::SuppressUnusedImportWarnings => Some("checking for fix candidates"),
        BuildOutputMode::Quiet => Some("validating applied fixes"),
        // The main pass forwards cargo's own status lines, so the spinner is
        // there for what cargo cannot report: a package's status line is printed
        // once however many targets it holds, and mend analyzes each of those
        // targets separately. On a package with a library and thirty examples
        // that is one line followed by minutes of silence.
        BuildOutputMode::Full => Some("analyzing"),
        BuildOutputMode::Json => None,
    }
}

/// The status text for one frame: cargo's unit counter once it has drawn one,
/// then the targets under analysis right now, or `fallback` when the run is
/// between analyses.
fn analyzing_status(
    targets: &[String],
    fallback: &str,
    unit_counter: Option<UnitCounter>,
) -> String {
    let activity = if targets.is_empty() {
        fallback.to_string()
    } else {
        format!("analyzing {}", targets.join(", "))
    };
    match unit_counter {
        Some(unit_counter) => format!("{unit_counter}: {activity}"),
        None => activity,
    }
}

fn progress_frame(message: &str, frame_index: usize) -> String {
    let frame = PROGRESS_FRAMES[frame_index % PROGRESS_FRAMES.len()];
    format!("\rmend: {frame} {message}")
}

fn progress_line_width(message: &str) -> usize { progress_frame(message, 0).chars().count() - 1 }

fn clear_progress_line(width: usize) -> String { format!("\r{}\r", " ".repeat(width)) }

#[cfg(test)]
mod tests {
    use super::UnitCounter;
    use super::analyzing_status;
    use super::clear_progress_line;
    use super::progress_frame;
    use super::progress_line_width;
    use super::progress_message_for;
    use crate::compiler::build::BuildOutputMode;
    use crate::compiler::constants::PROGRESS_BAR_FILL;
    use crate::compiler::constants::PROGRESS_BAR_HEAD;
    use crate::compiler::constants::PROGRESS_BAR_WIDTH;

    #[test]
    fn quiet_mode_uses_validation_status_message() {
        assert_eq!(
            progress_message_for(BuildOutputMode::Quiet),
            Some("validating applied fixes")
        );
    }

    #[test]
    fn json_mode_has_no_progress_status() {
        assert_eq!(progress_message_for(BuildOutputMode::Json), None);
    }

    #[test]
    fn progress_frame_and_clear_line_use_carriage_return() {
        let frame = progress_frame("validating applied fixes", 1);
        let width = progress_line_width("validating applied fixes");

        assert_eq!(frame, "\rmend: / validating applied fixes");
        assert_eq!(
            clear_progress_line(width),
            format!("\r{}\r", " ".repeat(width))
        );
    }

    #[test]
    fn unit_counter_draws_cargos_bar_at_a_fixed_width() {
        let counter = |done, total| UnitCounter { done, total }.to_string();
        let half = PROGRESS_BAR_WIDTH / 2;

        assert_eq!(
            counter(0, 4),
            format!("[{}] 0/4", " ".repeat(PROGRESS_BAR_WIDTH))
        );
        assert_eq!(
            counter(2, 4),
            format!(
                "[{}{PROGRESS_BAR_HEAD}{}] 2/4",
                PROGRESS_BAR_FILL.repeat(half - 1),
                " ".repeat(PROGRESS_BAR_WIDTH - half)
            )
        );
        assert_eq!(
            counter(4, 4),
            format!("[{}] 4/4", PROGRESS_BAR_FILL.repeat(PROGRESS_BAR_WIDTH))
        );
    }

    /// `cargo tile` finds a run's counter as `] done/total:` in its captured
    /// log, the way cargo's own bar leaves it.
    #[test]
    fn status_with_a_counter_leaves_it_where_cargo_leaves_its_own() {
        let status = analyzing_status(
            &["cargo_mend".to_string()],
            "analyzing",
            Some(UnitCounter {
                done:  149,
                total: 403,
            }),
        );

        assert!(status.starts_with('['));
        assert!(status.ends_with("] 149/403: analyzing cargo_mend"));
    }

    #[test]
    fn status_before_cargo_draws_a_bar_names_the_activity_alone() {
        assert_eq!(
            analyzing_status(&[], "checking for fix candidates", None),
            "checking for fix candidates"
        );
    }
}
