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
//! `App` (see `app/mod.rs`) — those methods touch `Startup`,
//! framework toasts, and tracing, and have no single subsystem they
//! belong to.

use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Instant;
use std::time::SystemTime;

use tui_pane::ColoredToastId;
use tui_pane::PERF_LOG_TARGET;
use tui_pane::ProgressDetail;

use crate::ci::OwnerRepo;
use crate::project;
use crate::project::AbsolutePath;
use crate::tui::app::CountedPhase;
use crate::tui::app::KeyedPhase;
use crate::tui::app::LanguagePhase;
use crate::tui::app::phase_state::PhaseCompletion;
use crate::tui::app::phase_state::PhaseLifecycle;
use crate::tui::app::phase_state::ProgressRow;
use crate::tui::app::phase_state::ProgressState;
use crate::tui::constants::STARTUP_PHASE_CRATES_IO;
use crate::tui::constants::STARTUP_PHASE_DISK;
use crate::tui::constants::STARTUP_PHASE_GIT;
use crate::tui::constants::STARTUP_PHASE_GITHUB;
use crate::tui::constants::STARTUP_PHASE_LANGUAGES;
use crate::tui::constants::STARTUP_PHASE_LINT;
use crate::tui::constants::STARTUP_PHASE_METADATA;
use crate::tui::constants::STARTUP_PHASE_TESTS;
use crate::tui::constants::STARTUP_ROW_DETAIL_DELAY;
use crate::tui::constants::STARTUP_ROW_MIN_VISIBLE;
use crate::tui::state::StartupNetworkPending;
use crate::tui::state::StartupNetworkReadiness;
use crate::tui::state::StartupNetworkReady;

#[derive(Debug, Default)]
struct PhaseToken;

#[derive(Debug, Default)]
pub(in crate::tui::app) struct StartupPlanning {
    phase_token: PhaseToken,
}

#[derive(Debug)]
pub(in crate::tui::app) struct StartupCollecting {
    phase_token: PhaseToken,
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

/// Keep the last drawn width and time for a delayed detail refresh.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::tui::app) enum StartupToastLayout {
    NotDrawn,
    Drawn { width: usize, at: Instant },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::tui::app) struct StartupToast {
    id:                             ColoredToastId,
    pub(in crate::tui::app) layout: StartupToastLayout,
}

impl StartupToast {
    pub(in crate::tui::app) const fn new(id: ColoredToastId) -> Self {
        Self {
            id,
            layout: StartupToastLayout::NotDrawn,
        }
    }

    pub(in crate::tui::app) const fn id(self) -> ColoredToastId { self.id }
}

/// The live network fetch a startup row reports: the label of the fetch
/// in flight longest, or idle when none is running.
pub(in crate::tui::app) enum StartupFetchDetail {
    Idle,
    InFlight(String),
}

#[derive(Debug, Default)]
pub struct Startup {
    pub scan_complete_at:          Option<Instant>,
    pub(in crate::tui::app) toast: Option<StartupToast>,
    phase:                         StartupPhase,

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
    /// counted work tokens provide startup progress inside large roots without
    /// carrying every path through the UI queue.
    pub languages: LanguagePhase,
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

impl Startup {
    pub fn new() -> Self { Self::default() }

    /// Reset every phase-tracking field to its `Default` state. Called
    /// from `App::rescan` so a fresh scan starts the startup-phase
    /// state machine over.
    pub fn reset(&mut self) { *self = Self::default(); }

    pub const fn is_collecting(&self) -> bool { matches!(self.phase, StartupPhase::Collecting(_)) }

    pub(in crate::tui::app) fn take_planning(&mut self) -> Option<StartupPlanning> {
        match std::mem::take(&mut self.phase) {
            StartupPhase::Planning(planning) => Some(planning),
            phase => {
                self.phase = phase;
                None
            },
        }
    }

    pub(in crate::tui::app) fn take_collecting(&mut self) -> Option<StartupCollecting> {
        match std::mem::take(&mut self.phase) {
            StartupPhase::Collecting(collecting) => Some(collecting),
            phase => {
                self.phase = phase;
                None
            },
        }
    }

    pub(in crate::tui::app) fn log_phase_plan(&self) {
        tracing::trace!(
            target: PERF_LOG_TARGET,
            disk_expected = self.disk.expected_len(),
            git_expected = self.git.expected_len(),
            repo_expected = self.repo.expected_len(),
            crates_io_expected = self.crates_io.expected_len(),
            detail_declaration_expected = self.details_declared.expected_len(),
            lint_expected = self.lint_phase.expected_len(),
            languages_expected = self.languages.expected_len(),
            tests_expected = self.tests.expected_len(),
            metadata_expected = self.metadata.expected_len(),
            "startup_phase_plan"
        );
    }
    /// The panel rows, sorted alphabetically by label (case-insensitive) for
    /// display. The phase array below is in phase order — matching the gate
    /// and timeout arrays — and the result is sorted before returning. Each
    /// phase contributes a row only when it is not omitted (lint and crates.io
    /// are omitted until they have work). The repo row renders `Waiting` until
    /// its denominator stabilizes. The phases are keyed differently, so the
    /// array is over `&dyn PhaseCompletion`.
    pub(in crate::tui::app) fn startup_panel_rows(
        &self,
        now: Instant,
        github_detail: &StartupFetchDetail,
        crates_io_detail: &StartupFetchDetail,
    ) -> Vec<ProgressRow> {
        let mut rows: Vec<ProgressRow> = self
            .panel_phases()
            .into_iter()
            .filter_map(|(label, phase)| {
                let state = phase.progress_state(now, STARTUP_ROW_MIN_VISIBLE)?;
                let detail = row_wants_detail(now, phase.first_seen(), state)
                    .then(|| self.row_detail(label, github_detail, crates_io_detail))
                    .flatten();
                Some(ProgressRow {
                    label,
                    state,
                    detail: detail.map_or(ProgressDetail::Absent, ProgressDetail::Message),
                })
            })
            .collect();
        rows.sort_by(|a, b| {
            a.label
                .bytes()
                .map(|byte| byte.to_ascii_lowercase())
                .cmp(b.label.bytes().map(|byte| byte.to_ascii_lowercase()))
        });
        rows
    }

    fn panel_phases(&self) -> [(&'static str, &dyn PhaseCompletion); 8] {
        [
            (STARTUP_PHASE_DISK, &self.disk),
            (STARTUP_PHASE_GIT, &self.git),
            (STARTUP_PHASE_GITHUB, &self.repo),
            (STARTUP_PHASE_CRATES_IO, &self.crates_io),
            (STARTUP_PHASE_METADATA, &self.metadata),
            (STARTUP_PHASE_LINT, &self.lint_phase),
            (STARTUP_PHASE_LANGUAGES, &self.languages),
            (STARTUP_PHASE_TESTS, &self.tests),
        ]
    }

    pub(in crate::tui::app) fn detail_visibility_changed(
        &self,
        drawn_at: Instant,
        now: Instant,
    ) -> bool {
        self.panel_phases().into_iter().any(|(_, phase)| {
            phase
                .progress_state(now, STARTUP_ROW_MIN_VISIBLE)
                .is_some_and(|state| {
                    row_wants_detail(now, phase.first_seen(), state)
                        != row_wants_detail(drawn_at, phase.first_seen(), state)
                })
        })
    }
    /// The item a slow row is currently working on (or about to): the live
    /// in-flight fetch for the network rows, the lexically-first pending key
    /// for the keyed batch rows.
    fn row_detail(
        &self,
        label: &str,
        github_detail: &StartupFetchDetail,
        crates_io_detail: &StartupFetchDetail,
    ) -> Option<String> {
        let home = |path: &AbsolutePath| project::home_relative_path(path.as_path());
        match label {
            STARTUP_PHASE_DISK => self.disk.pending_sample(home),
            STARTUP_PHASE_GIT => self.git.pending_sample(home),
            STARTUP_PHASE_GITHUB => match github_detail {
                StartupFetchDetail::InFlight(label) => Some(label.clone()),
                StartupFetchDetail::Idle => self.repo.pending_sample(ToString::to_string),
            },
            STARTUP_PHASE_CRATES_IO => match crates_io_detail {
                StartupFetchDetail::InFlight(label) => Some(label.clone()),
                StartupFetchDetail::Idle => self.crates_io.pending_sample(Clone::clone),
            },
            STARTUP_PHASE_METADATA => self.metadata.pending_sample(home),
            STARTUP_PHASE_TESTS => self.tests.pending_sample(home),
            _ => None,
        }
    }
    /// `true` once every tracked phase no longer holds the panel open —
    /// omitted, or complete past its minimum-visible floor. Iterating the
    /// phases (rather than naming each) means a row added in a later phase
    /// cannot silently miss the gate. `repo` is keyed differently, so the
    /// array is over `&dyn PhaseCompletion`.
    fn all_rows_gate_satisfied(&self, now: Instant) -> bool {
        self.panel_phases()
            .iter()
            .all(|(_, phase)| phase.gate_satisfied(now, STARTUP_ROW_MIN_VISIBLE))
    }
}

/// A still-in-progress determinate row that has been visible past
/// `STARTUP_ROW_DETAIL_DELAY` warrants showing the item it is working on; a
/// fast row reaches 100% before the delay elapses and so never shows it.
fn row_wants_detail(now: Instant, first_seen: Option<Instant>, state: ProgressState) -> bool {
    let in_progress = matches!(state, ProgressState::Active(percentage) if percentage.get() < 100);
    in_progress
        && first_seen.is_some_and(|first| now.duration_since(first) >= STARTUP_ROW_DETAIL_DELAY)
}

#[derive(Debug)]
pub(in crate::tui::app) struct StartupPlan {
    pub(in crate::tui::app) disk_expected:       HashSet<AbsolutePath>,
    pub(in crate::tui::app) git_expected:        HashSet<AbsolutePath>,
    pub(in crate::tui::app) git_seen:            HashSet<AbsolutePath>,
    pub(in crate::tui::app) metadata_expected:   HashSet<AbsolutePath>,
    pub(in crate::tui::app) lint_history:        HashSet<AbsolutePath>,
    pub(in crate::tui::app) lint_count_expected: usize,
    pub(in crate::tui::app) crates_io_expected:  HashSet<String>,
    pub(in crate::tui::app) detail_expected:     HashSet<AbsolutePath>,
    pub(in crate::tui::app) github_running:      Vec<OwnerRepo>,
    pub(in crate::tui::app) crates_io_running:   Vec<String>,
}

impl StartupPlanning {
    pub(in crate::tui::app) fn install(self, startup: &mut Startup, plan: &StartupPlan) {
        startup
            .languages
            .reset_with_expected_roots(plan.disk_expected.clone());
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
        startup.lint_count.lifecycle = PhaseLifecycle::Running;
        startup.phase = StartupPhase::Collecting(self.into_collecting());
    }

    const fn into_collecting(self) -> StartupCollecting {
        let Self { phase_token } = self;
        StartupCollecting { phase_token }
    }
}

impl StartupCollecting {
    pub(in crate::tui::app) fn try_ready(
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

    pub(in crate::tui::app) const fn restore(self, startup: &mut Startup) {
        startup.phase = StartupPhase::Collecting(self);
    }

    const fn into_closing(self) {
        let Self { phase_token } = self;
        let PhaseToken = phase_token;
    }
}

#[derive(Debug)]
pub(in crate::tui::app) struct StartupReady {
    collecting:       StartupCollecting,
    completed_at:     Instant,
    scan_complete_at: Instant,
    network:          StartupNetworkReady,
}

impl StartupReady {
    pub(in crate::tui::app) const fn begin_closing(
        self,
        startup: &mut Startup,
    ) -> StartedStartupClosing {
        let Self {
            collecting,
            completed_at,
            scan_complete_at,
            network,
        } = self;
        collecting.into_closing();
        startup.phase = StartupPhase::Closing;
        StartedStartupClosing {
            completed_at,
            scan_complete_at,
            network,
        }
    }
}

pub(in crate::tui::app) enum StartupReadiness {
    Ready(StartupReady),
    RowsPending(StartupCollecting),
    DeclarationsPending(StartupCollecting),
    NetworkPending {
        pending:    StartupNetworkPending,
        collecting: StartupCollecting,
    },
}

pub(in crate::tui::app) struct StartedStartupClosing {
    pub(in crate::tui::app) completed_at:     Instant,
    pub(in crate::tui::app) scan_complete_at: Instant,
    pub(in crate::tui::app) network:          StartupNetworkReady,
}
