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

use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Instant;
use std::time::SystemTime;

use tui_pane::ColoredToastId;

use crate::ci::OwnerRepo;
use crate::project::AbsolutePath;
use crate::tui::app::CountedPhase;
use crate::tui::app::KeyedPhase;
use crate::tui::app::phase_state::PhaseCompletion;
use crate::tui::state::StartupNetworkPending;
use crate::tui::state::StartupNetworkReadiness;
use crate::tui::state::StartupNetworkReady;

#[derive(Debug, Default)]
pub struct Startup {
    pub scan_complete_at: Option<Instant>,
    pub(super) toast:     Option<StartupToast>,
    phase:                StartupPhase,

    /// Newest lint-relevant source mtime per project, collected from the disk
    /// walk (`BackgroundMsg::DiskUsageBatch`). Consumed once when the startup
    /// phase closes by `App::kick_off_startup_lints` to decide which projects
    /// changed since their last lint.
    pub source_mtimes: HashMap<AbsolutePath, SystemTime>,

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

    /// Drives the "Lint history" startup row: keyed on each Rust project's
    /// path, with `seen` marked when `BackgroundMsg::LintHistoryLoaded`
    /// applies that project's history. Seeded with the full project set up
    /// front, so the row always completes and never strands the panel on a
    /// live lint run.
    pub lint_phase: KeyedPhase<AbsolutePath>,
    /// Counts the startup cached-lint-status load across the project tree
    /// (internal cardinality, not a panel row). Used by
    /// `App::maybe_complete_startup_lint_cache` to decide when the cached
    /// statuses are all applied.
    pub lint_count: CountedPhase,

    /// Tokei language stats. Project-root tokens gate final stats batches;
    /// file-level tokens provide startup progress inside large roots.
    pub languages: KeyedPhase<AbsolutePath>,
    /// Per-project test counts, keyed on project root; `seen` marked as
    /// each `TestCountsBatch` applies. Same denominator as `disk`.
    pub tests:     KeyedPhase<AbsolutePath>,

    /// Internal startup declaration gate. Each startup project-detail worker
    /// marks its path after it has queued any dynamic follow-up work, such as
    /// submodule crates.io fetches. This phase is not rendered, but readiness
    /// requires it so startup cannot close before workers finish declaring
    /// their startup obligations.
    pub details_declared: KeyedPhase<AbsolutePath>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct StartupToast {
    id: ColoredToastId,
}

impl StartupToast {
    pub(super) const fn new(id: ColoredToastId) -> Self { Self { id } }

    pub(super) const fn id(self) -> ColoredToastId { self.id }
}

impl Startup {
    pub fn new() -> Self { Self::default() }

    /// Reset every phase-tracking field to its `Default` state. Called
    /// from `App::rescan` so a fresh scan starts the startup-phase
    /// state machine over.
    pub fn reset(&mut self) { *self = Self::default(); }

    pub const fn is_collecting(&self) -> bool { matches!(self.phase, StartupPhase::Collecting(_)) }

    pub(super) fn take_planning(&mut self) -> Option<StartupPlanning> {
        match std::mem::take(&mut self.phase) {
            StartupPhase::Planning(planning) => Some(planning),
            phase => {
                self.phase = phase;
                None
            },
        }
    }

    pub(super) fn take_collecting(&mut self) -> Option<StartupCollecting> {
        match std::mem::take(&mut self.phase) {
            StartupPhase::Collecting(collecting) => Some(collecting),
            phase => {
                self.phase = phase;
                None
            },
        }
    }
}

#[derive(Debug)]
pub(super) struct StartupPlan {
    pub(super) disk_expected:       HashSet<AbsolutePath>,
    pub(super) git_expected:        HashSet<AbsolutePath>,
    pub(super) git_seen:            HashSet<AbsolutePath>,
    pub(super) metadata_expected:   HashSet<AbsolutePath>,
    pub(super) lint_history:        HashSet<AbsolutePath>,
    pub(super) lint_count_expected: usize,
    pub(super) crates_io_expected:  HashSet<String>,
    pub(super) detail_expected:     HashSet<AbsolutePath>,
    pub(super) github_running:      Vec<OwnerRepo>,
    pub(super) crates_io_running:   Vec<String>,
}

#[derive(Debug, Default)]
pub(super) struct StartupPlanning {
    phase_token: PhaseToken,
}

impl StartupPlanning {
    pub(super) fn install(self, startup: &mut Startup, plan: &StartupPlan) {
        startup
            .languages
            .reset_with_expected(plan.disk_expected.clone());
        startup
            .tests
            .reset_with_expected(plan.disk_expected.clone());
        startup.disk.reset_with_expected(plan.disk_expected.clone());
        startup.git.reset_with_expected(plan.git_expected.clone());
        startup.git.seen.clone_from(&plan.git_seen);
        startup.repo.reset_growing();
        for repo in &plan.github_running {
            startup.repo.expected.insert(repo.clone());
        }
        if plan.crates_io_expected.is_empty() {
            startup.crates_io.reset_unknown();
        } else {
            startup
                .crates_io
                .reset_with_expected(plan.crates_io_expected.clone());
        }
        for name in &plan.crates_io_running {
            startup.crates_io.expected.insert(name.clone());
        }
        if plan.lint_history.is_empty() {
            startup.lint_phase.reset_unknown();
        } else {
            startup
                .lint_phase
                .reset_with_expected(plan.lint_history.clone());
        }
        startup
            .metadata
            .reset_with_expected(plan.metadata_expected.clone());
        startup
            .details_declared
            .reset_with_expected(plan.detail_expected.clone());
        startup.lint_count.expected = Some(plan.lint_count_expected);
        startup.lint_count.seen = 0;
        startup.lint_count.complete_at = None;
        startup.phase = StartupPhase::Collecting(self.into_collecting());
    }

    const fn into_collecting(self) -> StartupCollecting {
        let Self { phase_token } = self;
        StartupCollecting { phase_token }
    }
}

#[derive(Debug)]
pub(super) struct StartupCollecting {
    phase_token: PhaseToken,
}

impl StartupCollecting {
    pub(super) fn try_ready(
        self,
        startup: &Startup,
        now: Instant,
        scan_complete_at: Instant,
        network: StartupNetworkReadiness,
    ) -> StartupReadiness {
        if !startup.details_declared.is_terminal() {
            return StartupReadiness::DeclarationsPending(self);
        }
        if !startup.all_rows_gate_satisfied(now) {
            return StartupReadiness::RowsPending(self);
        }
        match network {
            StartupNetworkReadiness::Ready(network) => StartupReadiness::Ready(StartupReady {
                collecting: self,
                completed_at: now,
                scan_complete_at,
                network,
            }),
            StartupNetworkReadiness::Pending(pending) => StartupReadiness::NetworkPending {
                pending,
                collecting: self,
            },
        }
    }

    pub(super) const fn restore(self, startup: &mut Startup) {
        startup.phase = StartupPhase::Collecting(self);
    }

    const fn into_closing(self) {
        let Self { phase_token } = self;
        let PhaseToken = phase_token;
    }
}

#[derive(Debug)]
pub(super) struct StartupReady {
    collecting:       StartupCollecting,
    completed_at:     Instant,
    scan_complete_at: Instant,
    network:          StartupNetworkReady,
}

impl StartupReady {
    pub(super) const fn begin_closing(self, startup: &mut Startup) -> StartedStartupClosing {
        let Self {
            collecting,
            completed_at,
            scan_complete_at,
            network,
        } = self;
        collecting.into_closing();
        startup.phase = StartupPhase::Closing;
        StartedStartupClosing {
            toast: startup.toast.take(),
            completed_at,
            scan_complete_at,
            network,
        }
    }
}

#[derive(Debug)]
enum StartupPhase {
    Planning(StartupPlanning),
    Collecting(StartupCollecting),
    Closing,
}

impl Default for StartupPhase {
    fn default() -> Self { Self::Planning(StartupPlanning::default()) }
}

#[derive(Debug, Default)]
struct PhaseToken;

pub(super) enum StartupReadiness {
    Ready(StartupReady),
    RowsPending(StartupCollecting),
    DeclarationsPending(StartupCollecting),
    NetworkPending {
        pending:    StartupNetworkPending,
        collecting: StartupCollecting,
    },
}

pub(super) struct StartedStartupClosing {
    pub(super) toast:            Option<StartupToast>,
    pub(super) completed_at:     Instant,
    pub(super) scan_complete_at: Instant,
    pub(super) network:          StartupNetworkReady,
}
