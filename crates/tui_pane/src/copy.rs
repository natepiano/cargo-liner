//! Focused-row copy support.
//!
//! Panes decide what their current selection means. The framework
//! owns registration, routing, and the clipboard backend call.

use std::fmt::Display;
use std::fmt::Formatter;
#[cfg(feature = "clipboard")]
use std::io::Write as _;

use crossterm::clipboard::CopyToClipboard;
use crossterm::execute;

use crate::AppContext;
use crate::Pane;

/// Kind of value a pane copied, used for user feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyLabel {
    /// A scalar field value.
    Value,
    /// A filesystem path.
    Path,
    /// A URL.
    Url,
    /// A whole row.
    Row,
    /// A command line.
    Command,
}

impl CopyLabel {
    /// Lowercase noun suitable for short status messages.
    #[must_use]
    pub const fn noun(self) -> &'static str {
        match self {
            Self::Value => "value",
            Self::Path => "path",
            Self::Url => "URL",
            Self::Row => "row",
            Self::Command => "command",
        }
    }
}

/// Text plus label returned by a pane copy resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyPayload {
    /// Text to place on the clipboard.
    pub text:  String,
    /// Payload category for feedback.
    pub label: CopyLabel,
}

impl CopyPayload {
    /// Construct a copy payload.
    #[must_use]
    pub fn new(text: impl Into<String>, label: CopyLabel) -> Self {
        Self {
            text: text.into(),
            label,
        }
    }
}

/// Result returned by a pane-specific copy resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopySelectionResult {
    /// The current selection has text to copy.
    Payload(CopyPayload),
    /// The current selection has no useful copy value.
    Nothing,
}

/// Result of a framework copy attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyOutcome {
    /// The backend wrote the copy sequence successfully.
    Copied {
        /// Label for the copied payload.
        label: CopyLabel,
    },
    /// No focused pane copy resolver produced a payload.
    NothingToCopy,
    /// Clipboard support is not available in this build or terminal path.
    Unavailable {
        /// Why clipboard support was unavailable.
        reason: ClipboardError,
    },
    /// Clipboard support was available, but the write failed.
    Failed {
        /// Why the write failed.
        reason: ClipboardError,
    },
}

/// Error from the clipboard backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardError {
    /// Clipboard support is unavailable.
    Unavailable,
    /// Writing to the terminal or backend failed.
    WriteFailed(String),
}

impl ClipboardError {
    /// Whether this error should be reported as unavailable rather than failed.
    #[must_use]
    pub const fn is_unavailable(&self) -> bool { matches!(self, Self::Unavailable) }
}

impl Display for ClipboardError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str("clipboard unavailable"),
            Self::WriteFailed(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ClipboardError {}

impl From<std::io::Error> for ClipboardError {
    fn from(err: std::io::Error) -> Self { Self::WriteFailed(err.to_string()) }
}

/// Clipboard writer used by the framework copy service.
pub trait ClipboardBackend {
    /// Write `text` to the host clipboard.
    ///
    /// # Errors
    ///
    /// Returns [`ClipboardError`] when clipboard support is unavailable
    /// or the backend write fails.
    fn write_clipboard(&mut self, text: &str) -> Result<(), ClipboardError>;
}

/// Production clipboard backend using Crossterm OSC52.
pub struct SystemClipboard;

impl SystemClipboard {
    /// Construct a system clipboard backend.
    #[must_use]
    pub const fn new() -> Self { Self }
}

impl Default for SystemClipboard {
    fn default() -> Self { Self::new() }
}

impl ClipboardBackend for SystemClipboard {
    fn write_clipboard(&mut self, text: &str) -> Result<(), ClipboardError> {
        write_system_clipboard(text)
    }
}

#[cfg(feature = "clipboard")]
fn write_system_clipboard(text: &str) -> Result<(), ClipboardError> {
    let mut stdout = std::io::stdout();
    execute!(stdout, CopyToClipboard::to_clipboard_from(text))?;
    stdout.flush()?;
    Ok(())
}

#[cfg(not(feature = "clipboard"))]
fn write_system_clipboard(_: &str) -> Result<(), ClipboardError> {
    Err(ClipboardError::Unavailable)
}

/// Optional pane trait for framework-owned copy support.
pub trait CopySelection<Ctx: AppContext>: Pane<Ctx> {
    /// Return the payload for the pane's current selection.
    fn copy_selection(ctx: &Ctx) -> CopySelectionResult;
}
