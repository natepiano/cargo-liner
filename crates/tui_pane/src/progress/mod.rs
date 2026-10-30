//! Reusable progress tracking and panel rendering.

mod completion;
mod counted_phase;
mod keyed_phase;
mod percentage;
mod render;

pub use completion::FailureReason;
pub use completion::PhaseCompletion;
pub use completion::PhaseLifecycle;
pub use completion::ProgressState;
pub use counted_phase::CountedPhase;
pub use keyed_phase::Denominator;
pub use keyed_phase::KeyedPhase;
pub use percentage::Percentage;
pub use render::ProgressDetail;
pub use render::ProgressPanelStyle;
pub use render::ProgressRow;
pub use render::progress_panel_body;
