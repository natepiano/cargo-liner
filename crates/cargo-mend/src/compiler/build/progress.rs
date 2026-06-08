use std::io;
use std::io::IsTerminal;
use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::thread::JoinHandle;

use super::BuildOutputMode;
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
    line_width:  usize,
}

impl CargoProgress {
    pub(super) fn start(output_mode: BuildOutputMode) -> Self {
        let Some(message) = progress_message_for(output_mode) else {
            return Self { state: None };
        };
        if !io::stderr().is_terminal() {
            return Self { state: None };
        }

        let active = Arc::new(AtomicBool::new(true));
        let output_lock = Arc::new(Mutex::new(()));
        let thread_active = Arc::clone(&active);
        let thread_lock = Arc::clone(&output_lock);
        let line_width = progress_line_width(message);
        let handle = thread::spawn(move || {
            let mut frame_index = 0;
            while thread_active.load(Ordering::Relaxed) {
                if let Ok(_guard) = thread_lock.lock() {
                    eprint!("{}", progress_frame(message, frame_index));
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
            eprint!("{}", clear_progress_line(self.line_width));
            let _ = io::stderr().flush();
        }
    }

    fn write_status_notice(&self, notice: &str) {
        if let Ok(_guard) = self.output_lock.lock() {
            eprint!("{}", clear_progress_line(self.line_width));
            eprintln!("{notice}");
            let _ = io::stderr().flush();
        }
    }
}

const fn progress_message_for(output_mode: BuildOutputMode) -> Option<&'static str> {
    match output_mode {
        BuildOutputMode::SuppressUnusedImportWarnings => Some("checking for fix candidates"),
        BuildOutputMode::Quiet => Some("validating applied fixes"),
        BuildOutputMode::Full | BuildOutputMode::Json => None,
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
