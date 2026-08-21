use std::io;
use std::io::IsTerminal;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::thread;
use std::thread::JoinHandle;

use super::BuildOutputMode;
use crate::compiler::analyzing;
use crate::compiler::constants::PROGRESS_FRAMES;
use crate::compiler::constants::PROGRESS_INTERVAL;

pub(super) trait ProgressDisplay {
    fn is_active(&self) -> bool;

    fn write_status_notice(&mut self, notice: &str);

    fn stop_for_forwarded_output(&mut self);
}

pub(super) struct CargoProgress {
    state: Option<CargoProgressState>,
}

struct CargoProgressState {
    active:      Arc<AtomicBool>,
    output_lock: Arc<Mutex<()>>,
    handle:      Option<JoinHandle<()>>,
    /// Width of the status text the spinner thread printed last. The text names
    /// the targets currently under analysis, so it changes length between
    /// frames and cannot be fixed at startup the way a constant message could.
    line_width:  Arc<AtomicUsize>,
}

impl CargoProgress {
    pub(super) fn start(output_mode: BuildOutputMode, findings_dir: &Path) -> Self {
        let Some(message) = progress_message_for(output_mode) else {
            return Self { state: None };
        };
        if !io::stderr().is_terminal() {
            return Self { state: None };
        }

        let active = Arc::new(AtomicBool::new(true));
        let output_lock = Arc::new(Mutex::new(()));
        let line_width = Arc::new(AtomicUsize::new(progress_line_width(message)));
        let thread_active = Arc::clone(&active);
        let thread_lock = Arc::clone(&output_lock);
        let thread_width = Arc::clone(&line_width);
        let thread_findings_dir = findings_dir.to_path_buf();
        let handle = thread::spawn(move || {
            let mut frame_index = 0;
            while thread_active.load(Ordering::Relaxed) {
                let status = analyzing_status(&thread_findings_dir, message);
                let width = progress_line_width(&status);
                if let Ok(_guard) = thread_lock.lock() {
                    let previous = thread_width.swap(width, Ordering::Relaxed);
                    // Pad out to the previous width so a shorter status does not
                    // leave the tail of the longer one on screen.
                    eprint!(
                        "{}{}",
                        progress_frame(&status, frame_index),
                        " ".repeat(previous.saturating_sub(width))
                    );
                    let _ = io::stderr().flush();
                }
                frame_index = (frame_index + 1) % PROGRESS_FRAMES.len();
                thread::sleep(PROGRESS_INTERVAL);
            }
        });

        Self {
            state: Some(CargoProgressState {
                active,
                output_lock,
                handle: Some(handle),
                line_width,
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
        if let Ok(_guard) = self.output_lock.lock() {
            eprint!(
                "{}",
                clear_progress_line(self.line_width.load(Ordering::Relaxed))
            );
            let _ = io::stderr().flush();
        }
    }

    fn write_status_notice(&self, notice: &str) {
        if let Ok(_guard) = self.output_lock.lock() {
            eprint!(
                "{}",
                clear_progress_line(self.line_width.load(Ordering::Relaxed))
            );
            eprintln!("{notice}");
            let _ = io::stderr().flush();
        }
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

/// The status text for one frame: the targets under analysis right now, or
/// `fallback` when the run is between analyses.
fn analyzing_status(findings_dir: &Path, fallback: &str) -> String {
    let targets = analyzing::targets_in_flight(findings_dir);
    if targets.is_empty() {
        return fallback.to_string();
    }
    format!("analyzing {}", targets.join(", "))
}

fn progress_frame(message: &str, frame_index: usize) -> String {
    let frame = PROGRESS_FRAMES[frame_index % PROGRESS_FRAMES.len()];
    format!("\rmend: {frame} {message}")
}

fn progress_line_width(message: &str) -> usize { progress_frame(message, 0).chars().count() - 1 }

fn clear_progress_line(width: usize) -> String { format!("\r{}\r", " ".repeat(width)) }

#[cfg(test)]
mod tests {
    use super::clear_progress_line;
    use super::progress_frame;
    use super::progress_line_width;
    use super::progress_message_for;
    use crate::compiler::build::BuildOutputMode;

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
}
