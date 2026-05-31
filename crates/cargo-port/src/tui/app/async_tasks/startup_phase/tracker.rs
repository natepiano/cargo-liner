use std::collections::HashSet;
use std::time::Instant;

use ratatui::style::Color;

use super::toast_bodies;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::LanguageStats;
use crate::project::TestCounts;
use crate::tui::app::App;
use crate::tui::app::Startup;
use crate::tui::app::phase_state::FailureReason;
use crate::tui::app::phase_state::PhaseCompletion;
use crate::tui::app::phase_state::ProgressRow;
use crate::tui::app::phase_state::ProgressState;
use crate::tui::app::startup;
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
use crate::tui::constants::STARTUP_ROW_TIMEOUT;
impl Startup {
    pub(super) fn log_phase_plan(&self) {
        tracing::info!(
            disk_expected = self.disk.expected_len(),
            git_expected = self.git.expected_len(),
            repo_expected = self.repo.expected_len(),
            lint_expected = self.lint_phase.expected_len(),
            metadata_expected = self.metadata.expected_len(),
            "startup_phase_plan"
        );
    }
    pub(super) fn maybe_complete_lints(&mut self, now: Instant, scan_complete_at: Instant) {
        // Lint is only "complete" once real lint work has been registered —
        // an initialized-empty expected set stays open. This diverges from
        // the generic `PhaseCompletion::is_complete` semantics on purpose,
        // so the check stays inline rather than going through the trait.
        let lint = &self.lint_phase;
        let should_complete = lint.complete_at.is_none()
            && lint
                .expected
                .keys()
                .is_some_and(|expected| !expected.is_empty() && lint.seen.len() >= expected.len());
        if !should_complete {
            return;
        }
        self.lint_phase.complete_at = Some(now);
        tracing::info!(
            phase = "lint_terminal_applied",
            since_scan_complete_ms =
                tui_pane::perf_log_ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.lint_phase.seen.len(),
            expected = self.lint_phase.expected_len(),
            "startup_phase_complete"
        );
    }
    /// The panel rows, in display order: disk, git, GitHub (repo), crates.io,
    /// metadata, lint, languages, tests. Each phase contributes a row only
    /// when it is not omitted (lint and crates.io are omitted until they have
    /// work). The repo row renders `Waiting` until its denominator
    /// stabilizes. The phases are keyed differently, so the array is over
    /// `&dyn PhaseCompletion`.
    pub(super) fn startup_panel_rows(
        &self,
        now: Instant,
        github_detail: Option<&str>,
        crates_io_detail: Option<&str>,
    ) -> Vec<ProgressRow> {
        let phases: [(&'static str, &dyn PhaseCompletion); 8] = [
            (STARTUP_PHASE_DISK, &self.disk),
            (STARTUP_PHASE_GIT, &self.git),
            (STARTUP_PHASE_GITHUB, &self.repo),
            (STARTUP_PHASE_CRATES_IO, &self.crates_io),
            (STARTUP_PHASE_METADATA, &self.metadata),
            (STARTUP_PHASE_LINT, &self.lint_phase),
            (STARTUP_PHASE_LANGUAGES, &self.languages),
            (STARTUP_PHASE_TESTS, &self.tests),
        ];
        phases
            .into_iter()
            .filter_map(|(label, phase)| {
                let state = phase.progress_state(now, STARTUP_ROW_MIN_VISIBLE)?;
                let detail = row_wants_detail(now, phase.first_seen(), state)
                    .then(|| self.row_detail(label, github_detail, crates_io_detail))
                    .flatten();
                Some(ProgressRow {
                    label,
                    state,
                    detail,
                })
            })
            .collect()
    }
    /// The item a slow row is currently working on (or about to): the live
    /// in-flight fetch for the network rows, the lexically-first pending key
    /// for the keyed batch rows.
    fn row_detail(
        &self,
        label: &str,
        github_detail: Option<&str>,
        crates_io_detail: Option<&str>,
    ) -> Option<String> {
        let home = |path: &AbsolutePath| project::home_relative_path(path.as_path());
        match label {
            STARTUP_PHASE_DISK => self.disk.pending_sample(home),
            STARTUP_PHASE_GIT => self.git.pending_sample(home),
            STARTUP_PHASE_GITHUB => github_detail
                .map(ToString::to_string)
                .or_else(|| self.repo.pending_sample(ToString::to_string)),
            STARTUP_PHASE_CRATES_IO => crates_io_detail
                .map(ToString::to_string)
                .or_else(|| self.crates_io.pending_sample(Clone::clone)),
            STARTUP_PHASE_METADATA => self.metadata.pending_sample(home),
            STARTUP_PHASE_LANGUAGES => self.languages.pending_sample(home),
            STARTUP_PHASE_TESTS => self.tests.pending_sample(home),
            _ => None,
        }
    }
    /// `true` once every tracked phase no longer holds the panel open —
    /// omitted, or complete past its minimum-visible floor. Iterating the
    /// phases (rather than naming each) means a row added in a later phase
    /// cannot silently miss the gate. `repo` is keyed differently, so the
    /// array is over `&dyn PhaseCompletion`.
    pub(super) fn all_rows_gate_satisfied(&self, now: Instant) -> bool {
        let phases: [&dyn PhaseCompletion; 8] = [
            &self.disk,
            &self.git,
            &self.repo,
            &self.crates_io,
            &self.metadata,
            &self.lint_phase,
            &self.languages,
            &self.tests,
        ];
        phases
            .iter()
            .all(|phase| phase.gate_satisfied(now, STARTUP_ROW_MIN_VISIBLE))
    }
}
impl App {
    pub fn initialize_startup_phase_tracker(&mut self) {
        self.reset_startup_phase_state();
        self.start_startup_toast();
        self.startup.log_phase_plan();
        self.maybe_log_startup_phase_completions();
    }
    pub(super) fn reset_startup_phase_state(&mut self) {
        let disk_expected = startup::initial_disk_roots(&self.project_list);
        let git_expected = self
            .project_list
            .git_directories()
            .into_iter()
            .collect::<HashSet<_>>();
        let git_seen = self
            .project_list
            .iter()
            .filter(|entry| entry.item.git_info().is_some())
            .filter_map(|entry| entry.item.git_directory())
            .collect::<HashSet<_>>();
        let metadata_expected = startup::initial_metadata_roots(&self.project_list);
        self.startup.scan_complete_at = Some(Instant::now());
        self.startup.toast = None;
        self.startup.complete_at = None;
        // Languages (tokei) and test counts scan the same project roots as
        // disk usage and emit one batch entry per root, so they share disk's
        // denominator. Seed them before any batch can arrive.
        self.startup
            .languages
            .reset_with_expected(disk_expected.clone());
        self.startup
            .tests
            .reset_with_expected(disk_expected.clone());
        self.startup.disk.reset_with_expected(disk_expected);
        self.startup.git.reset_with_expected(git_expected);
        self.startup.git.seen = git_seen;
        // Repo's GitHub set accrues as git remotes resolve; it renders
        // `Waiting` until git completes and the denominator stabilizes.
        self.startup.repo.reset_growing();
        // crates.io fetches are dispatched for every publishable crate the
        // moment background services register, so the target set is known
        // upfront — seed a stable denominator (empty target list omits the
        // row). `seen` is marked as each `CratesIoFetchComplete` arrives.
        let crates_io_expected: HashSet<String> = self
            .collect_publishable_crates_io_targets()
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        if crates_io_expected.is_empty() {
            self.startup.crates_io.reset_unknown();
        } else {
            self.startup
                .crates_io
                .reset_with_expected(crates_io_expected);
        }
        // Lint stays omitted (Unknown) until a real lint run is queued, so
        // an empty lint set never renders a premature 100% row.
        self.startup.lint_phase.reset_unknown();
        self.startup.metadata.reset_with_expected(metadata_expected);
    }
    pub(super) fn start_startup_toast(&mut self) {
        let now = Instant::now();
        // These rows are visible from panel creation; stamp their
        // minimum-visible floor now. Repo renders `Waiting` from the start.
        // Lint stamps later, when its first run is queued.
        self.startup.disk.stamp_first_seen(now);
        self.startup.git.stamp_first_seen(now);
        self.startup.repo.stamp_first_seen(now);
        self.startup.crates_io.stamp_first_seen(now);
        self.startup.metadata.stamp_first_seen(now);
        self.startup.languages.stamp_first_seen(now);
        self.startup.tests.stamp_first_seen(now);
        let (lines, colors) = self.startup_panel_lines(now);
        let task_id = self
            .framework
            .toasts
            .start_colored_task("Startup", lines, colors);
        self.startup.toast = Some(task_id);
    }
    /// Build the panel's per-line text and matching per-line colors from the
    /// current phase states and the live in-flight network fetches.
    fn startup_panel_lines(&self, now: Instant) -> (Vec<String>, Vec<Color>) {
        let github_detail = self.in_flight_github_label();
        let crates_io_detail = self.in_flight_crates_io_label();
        let width = tui_pane::toast_body_width(self.framework.toast_settings());
        let rows = self.startup.startup_panel_rows(
            now,
            github_detail.as_deref(),
            crates_io_detail.as_deref(),
        );
        toast_bodies::startup_panel_body(&rows, width)
    }
    /// The GitHub repo fetch that has been in flight longest — the row's
    /// "currently working on" detail.
    fn in_flight_github_label(&self) -> Option<String> {
        self.net
            .github
            .running()
            .running
            .iter()
            .min_by_key(|(_, started)| **started)
            .map(|(repo, _)| repo.to_string())
    }
    /// The crates.io fetch in flight (one at a time) — the row's detail.
    fn in_flight_crates_io_label(&self) -> Option<String> {
        self.net
            .crates_io
            .running()
            .running
            .iter()
            .min_by_key(|(_, started)| **started)
            .map(|(name, _)| name.clone())
    }
    pub fn maybe_log_startup_phase_completions(&mut self) {
        let Some(scan_complete_at) = self.startup.scan_complete_at else {
            return;
        };
        // Once the panel has closed, a late phase result must not re-run
        // the gate or touch the (taken) panel toast. The per-phase `seen`
        // bookkeeping in the handlers is idempotent and harmless.
        if self.startup.complete_at.is_some() {
            return;
        }
        let now = Instant::now();
        self.maybe_complete_startup_disk(now, scan_complete_at);
        self.maybe_complete_startup_git(now, scan_complete_at);
        self.maybe_complete_startup_repo(now, scan_complete_at);
        self.maybe_complete_startup_metadata(now, scan_complete_at);
        self.startup.maybe_complete_lints(now, scan_complete_at);
        // crates.io, languages, and test counts have no special logging;
        // just record their completion timestamp (for the min-visible floor)
        // once every expected entry has been seen.
        self.startup.crates_io.complete_once(now);
        self.startup.languages.complete_once(now);
        self.startup.tests.complete_once(now);
        self.refresh_startup_panel(now);
        self.maybe_complete_startup_ready(now, scan_complete_at);
    }
    /// Repaint the startup panel body from the current phase states. A
    /// no-op once the panel has been closed.
    pub(super) fn refresh_startup_panel(&mut self, now: Instant) {
        let Some(toast) = self.startup.toast else {
            return;
        };
        let (lines, colors) = self.startup_panel_lines(now);
        self.framework
            .toasts
            .update_task_colored(toast, lines, colors);
    }
    /// Re-evaluate the panel each frame so the minimum-visible floor and
    /// the per-row timeout can close it even when no new `BackgroundMsg`
    /// arrives.
    pub fn tick_startup_panel(&mut self) {
        if self.startup.complete_at.is_some() {
            return;
        }
        let Some(scan_complete_at) = self.startup.scan_complete_at else {
            return;
        };
        let now = Instant::now();
        self.sweep_startup_timeouts(now);
        self.refresh_startup_panel(now);
        self.maybe_complete_startup_ready(now, scan_complete_at);
    }
    /// Fail any phase that has been visible past `STARTUP_ROW_TIMEOUT`
    /// without completing — the backstop that guarantees startup always
    /// finishes. A newly-timed-out phase pops one warning toast.
    pub(super) fn sweep_startup_timeouts(&mut self, now: Instant) {
        let timeout = STARTUP_ROW_TIMEOUT;
        let timed_out: [(bool, &'static str); 8] = [
            (
                self.startup.disk.time_out(now, timeout).is_some(),
                STARTUP_PHASE_DISK,
            ),
            (
                self.startup.git.time_out(now, timeout).is_some(),
                STARTUP_PHASE_GIT,
            ),
            (
                self.startup.repo.time_out(now, timeout).is_some(),
                STARTUP_PHASE_GITHUB,
            ),
            (
                self.startup.crates_io.time_out(now, timeout).is_some(),
                STARTUP_PHASE_CRATES_IO,
            ),
            (
                self.startup.metadata.time_out(now, timeout).is_some(),
                STARTUP_PHASE_METADATA,
            ),
            (
                self.startup.lint_phase.time_out(now, timeout).is_some(),
                STARTUP_PHASE_LINT,
            ),
            (
                self.startup.languages.time_out(now, timeout).is_some(),
                STARTUP_PHASE_LANGUAGES,
            ),
            (
                self.startup.tests.time_out(now, timeout).is_some(),
                STARTUP_PHASE_TESTS,
            ),
        ];
        for (newly_failed, label) in timed_out {
            if newly_failed {
                self.show_timed_warning_toast(
                    "Startup timed out",
                    format!("{label} did not finish in time"),
                );
            }
        }
    }
    /// Mark the repo row failed (rate-limited or unreachable GitHub) so the
    /// panel finishes without waiting out the timeout. No-op once startup
    /// has completed or the repo row is already terminal; the accompanying
    /// service-unavailable toast already names the reason.
    pub fn fail_startup_repo_phase(&mut self, reason: FailureReason) {
        if self.startup.complete_at.is_some() {
            return;
        }
        let repo = &mut self.startup.repo;
        if repo.failure.is_some() || repo.complete_at.is_some() || repo.expected.is_unknown() {
            return;
        }
        repo.failure = Some(reason);
        self.maybe_log_startup_phase_completions();
    }
    /// Mark the languages row's `seen` from a `LanguageStatsBatch`. Runs
    /// alongside the `ProjectList` handler, which owns the actual stats.
    pub fn mark_startup_languages_seen(&mut self, entries: &[(AbsolutePath, LanguageStats)]) {
        for (path, _) in entries {
            self.startup.languages.seen.insert(path.clone());
        }
        self.maybe_log_startup_phase_completions();
    }
    /// Mark the tests row's `seen` from a `TestCountsBatch`. Runs alongside
    /// the `ProjectList` handler, which owns the actual counts.
    pub fn mark_startup_tests_seen(&mut self, entries: &[(AbsolutePath, TestCounts)]) {
        for (path, _) in entries {
            self.startup.tests.seen.insert(path.clone());
        }
        self.maybe_log_startup_phase_completions();
    }
    pub fn maybe_complete_startup_disk(&mut self, now: Instant, scan_complete_at: Instant) {
        if !self.startup.disk.complete_once(now) {
            return;
        }
        tracing::info!(
            phase = "disk_applied",
            since_scan_complete_ms =
                tui_pane::perf_log_ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.startup.disk.seen.len(),
            expected = self.startup.disk.expected_len(),
            "startup_phase_complete"
        );
    }
    pub fn maybe_complete_startup_git(&mut self, now: Instant, scan_complete_at: Instant) {
        if !self.startup.git.complete_once(now) {
            return;
        }
        tracing::info!(
            phase = "git_local_applied",
            since_scan_complete_ms =
                tui_pane::perf_log_ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.startup.git.seen.len(),
            expected = self.startup.git.expected_len(),
            "startup_phase_complete"
        );
    }
    pub fn maybe_complete_startup_repo(&mut self, now: Instant, scan_complete_at: Instant) {
        // Gate repo-phase completion on git being terminal (complete or
        // failed). Without this, a scan that completes before any
        // `RepoFetchQueued` arrives would see `repo.seen (0) >=
        // repo.expected (0)` and mark the phase done prematurely;
        // subsequent staggered git arrivals would then strand their repo
        // fetches outside the startup panel. Treating a timed-out git as
        // terminal releases repo so the panel still finishes.
        if !self.startup.git.is_terminal() {
            return;
        }
        // Git is terminal, so every GitHub remote that will be queued has
        // resolved (or git gave up): freeze the denominator so the row
        // switches from `Waiting` to a determinate bar. Idempotent; a late
        // `RepoFetchQueued` is dropped in `handle_repo_fetch_queued`.
        self.startup.repo.expected.stabilize();
        if !self.startup.repo.complete_once(now) {
            return;
        }
        tracing::info!(
            phase = "repo_fetch_applied",
            since_scan_complete_ms =
                tui_pane::perf_log_ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.startup.repo.seen.len(),
            expected = self.startup.repo.expected_len(),
            "startup_phase_complete"
        );
    }
    pub(super) fn maybe_complete_startup_metadata(
        &mut self,
        now: Instant,
        scan_complete_at: Instant,
    ) {
        if !self.startup.metadata.complete_once(now) {
            return;
        }
        tracing::info!(
            phase = "metadata_applied",
            since_scan_complete_ms =
                tui_pane::perf_log_ms(now.duration_since(scan_complete_at).as_millis()),
            seen = self.startup.metadata.seen.len(),
            expected = self.startup.metadata.expected_len(),
            "startup_phase_complete"
        );
    }
    pub fn maybe_complete_startup_ready(&mut self, now: Instant, scan_complete_at: Instant) {
        let lint_seen = self.startup.lint_phase.seen.len();
        let lint_expected = self.startup.lint_phase.expected_len();
        if self.startup.complete_at.is_some() {
            return;
        }
        if !self.startup.all_rows_gate_satisfied(now) {
            return;
        }
        self.startup.complete_at = Some(now);
        // Paint the final all-complete panel, then close it explicitly —
        // the body-string panel has no tracked-items auto-finish.
        self.refresh_startup_panel(now);
        if let Some(toast) = self.startup.toast.take() {
            self.finish_task_toast(toast);
        }
        let since_scan_ms = tui_pane::perf_log_ms(now.duration_since(scan_complete_at).as_millis());
        tracing::info!(
            since_scan_complete_ms = since_scan_ms,
            disk_seen = self.startup.disk.seen.len(),
            disk_expected = self.startup.disk.expected_len(),
            git_seen = self.startup.git.seen.len(),
            git_expected = self.startup.git.expected_len(),
            repo_seen = self.startup.repo.seen.len(),
            repo_expected = self.startup.repo.expected_len(),
            lint_seen = lint_seen,
            lint_expected = lint_expected,
            metadata_seen = self.startup.metadata.seen.len(),
            metadata_expected = self.startup.metadata.expected_len(),
            "startup_complete"
        );
        tracing::info!(since_scan_complete_ms = since_scan_ms, "steady_state_begin");
    }
}

/// A still-in-progress determinate row that has been visible past
/// `STARTUP_ROW_DETAIL_DELAY` warrants showing the item it is working on; a
/// fast row reaches 100% before the delay elapses and so never shows it.
fn row_wants_detail(now: Instant, first_seen: Option<Instant>, state: ProgressState) -> bool {
    let in_progress =
        matches!(state, ProgressState::Progress(percentage) if percentage.get() < 100);
    in_progress
        && first_seen.is_some_and(|first| now.duration_since(first) >= STARTUP_ROW_DETAIL_DELAY)
}
