//! Terminal lifecycle and the input dispatch ladder: what runs an app
//! built on the framework from the moment it takes the terminal to the
//! moment it hands it back.
//!
//! [`run_terminal`] takes the terminal, runs the loop, restores the
//! terminal, and relaunches the binary when a restart was asked for.
//! The app is a [`TerminalApp`]; the work it folds in on every pass is a
//! [`PollWork`], built after the terminal is set up; how the loop is
//! timed is its [`FrameProbe`], which also times the other
//! [`FramePhase`]s of a frame wherever the app calls it.
//!
//! A frame is drawn only when an event arrived or the app's work asked
//! for one, paced against a fixed deadline. A burst of resize events is
//! drained before the next draw, and every couple of seconds the whole
//! screen is written over.
//!
//! [`dispatch_key`] is the key ladder: the app's modal, then an open
//! framework overlay, then [`TerminalApp::attract_key`], then the
//! framework globals, then the app's own globals.
//!
//! In iTerm2 the session is moved onto the app's profile for as long as
//! it runs, and back on the way out, a panic included.

mod app;
mod constants;
mod deadline;
mod event_loop;
mod iterm2;
mod keys;
mod lifecycle;

pub use app::FramePhase;
pub use app::FrameProbe;
pub use app::NoProbe;
pub use app::PollWork;
pub use app::Repaint;
pub use app::TerminalApp;
pub use deadline::VisualDeadline;
pub use keys::dispatch_key;
pub use lifecycle::run_terminal;
