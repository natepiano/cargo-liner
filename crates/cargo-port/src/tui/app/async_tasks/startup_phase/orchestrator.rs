//! The `Startup` subsystem.
//!
//! Owns the startup-phase trackers — the per-phase `KeyedPhase` /
//! `CountedPhase` counters that drive the consolidated "Startup" toast,
//! whose body is the multi-row progress-bar panel (disk, git, GitHub,
//! metadata, lint, languages, tests). Phase-tracking data isn't scan
//! data and isn't lint data; it coordinates startup, so it lives on its
//! own subsystem.
//!
//! Cross-subsystem `maybe_complete_startup_*` orchestration stays on
//! `App` (see `tracker.rs`) — those methods touch `Startup`,
//! framework toasts, and tracing, and have no single subsystem they
//! belong to.

use std::time::Instant;

use tui_pane::ToastTaskId;

use crate::ci::OwnerRepo;
use crate::project::AbsolutePath;
use crate::tui::app::CountedPhase;
use crate::tui::app::KeyedPhase;

#[derive(Debug, Default)]
pub struct Startup {
    pub scan_complete_at: Option<Instant>,
    pub toast:            Option<ToastTaskId>,
    pub complete_at:      Option<Instant>,

    pub disk:      KeyedPhase<AbsolutePath>,
    pub git:       KeyedPhase<AbsolutePath>,
    pub repo:      KeyedPhase<OwnerRepo>,
    /// Keyed on crates.io crate name; denominator seeded upfront from the
    /// publishable-crate target list, `seen` marked as each
    /// `BackgroundMsg::CratesIoFetchComplete` arrives (which fires even on
    /// fetch failure, so the row cannot hang).
    pub crates_io: KeyedPhase<String>,
    /// Keyed on workspace root; seen when a `BackgroundMsg::CargoMetadata`
    /// arrival is either merged into the store or converted into an
    /// error toast.
    pub metadata:  KeyedPhase<AbsolutePath>,

    /// Tracks terminal lint events (`Passed` / `Failed`) keyed
    /// on project path; `seen` counts only terminal arrivals.
    pub lint_phase: KeyedPhase<AbsolutePath>,
    /// Counts startup-time lint completions across the project tree.
    /// Used by `Startup::maybe_complete_lints` to decide when the
    /// startup-lint pass is done.
    pub lint_count: CountedPhase,

    /// Tokei language stats, keyed on project root; `seen` marked as each
    /// `LanguageStatsBatch` applies. Same denominator as `disk`.
    pub languages: KeyedPhase<AbsolutePath>,
    /// Per-project test counts, keyed on project root; `seen` marked as
    /// each `TestCountsBatch` applies. Same denominator as `disk`.
    pub tests:     KeyedPhase<AbsolutePath>,
}

impl Startup {
    pub fn new() -> Self { Self::default() }

    /// Reset every phase-tracking field to its `Default` state. Called
    /// from `App::rescan` so a fresh scan starts the startup-phase
    /// state machine over.
    pub fn reset(&mut self) { *self = Self::default(); }
}
