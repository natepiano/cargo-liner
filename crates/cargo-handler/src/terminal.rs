//! Starting the grid: load its configuration, build the app, and run it
//! under [`tui_pane::run_terminal`] with the grid's motion, the attract
//! screen's frames and the favorites overlay's fades as the work the
//! loop polls.

use std::process::ExitCode;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use chrono::Local;
use chrono::TimeZone;
use tui_pane::Appearance;
use tui_pane::PollWork;
use tui_pane::Repaint;
use tui_pane::run_terminal;
use tui_pane::spawn_appearance_watcher;

use crate::app::App;
use crate::census;
use crate::census::AgentPlacement;
#[cfg(test)]
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::ChildKind;
use crate::census::ShellTimerObservation;
use crate::census::account::AccountLabels;
use crate::census::accounts::AccountsTable;
use crate::census::accounts::Reading;
use crate::census::scan::ScanStage;
use crate::config::CargoHandler;
use crate::config::LoadedConfig;
use crate::constants::BINARY_NAME;
use crate::startup_log;
use crate::theme;
use crate::theme::AppearanceDiagnostic;
use crate::theme::StartupThemeNotes;
use crate::theme::ThemeInstallation;

#[cfg(test)]
#[path = "terminal/startup_tests.rs"]
mod startup_tests;

#[cfg(test)]
#[path = "terminal/theme_note_tests.rs"]
mod theme_note_tests;

/// Load configuration, install the theme, build the keymap, and run the
/// event loop with the terminal in the alternate screen.
pub(crate) fn run() -> ExitCode {
    startup_log::init();
    let loaded_config = LoadedConfig::load::<CargoHandler>();
    startup_log::startup_event("config_loaded");
    let installation = theme::install_theme(&loaded_config.config.appearance, theme::builtins());
    let (appearance_sender, appearance_updates) = mpsc::channel();
    let startup_appearance = spawn_appearance_watcher(move |appearance| {
        let _ = appearance_sender.send(appearance);
    });
    startup_log::startup_event("appearance_read_returned");
    let startup_notes = startup_theme_notes(&loaded_config, installation, startup_appearance);
    // Read iterm2_profile before App::new consumes loaded_config.
    let iterm2_profile = loaded_config.config.appearance.iterm2_profile.clone();
    let mut app = match App::new(loaded_config, startup_notes) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("{BINARY_NAME}: keymap: {error}");
            return ExitCode::FAILURE;
        },
    };
    run_terminal(&mut app, &iterm2_profile, |app| {
        Ticker::new(
            census::schedule::spawn(
                app.remote_machines.clone(),
                app.hierarchy_command.clone(),
                AccountLabels::new(
                    &app.loaded_config.config.accounts,
                    dirs::home_dir().as_deref(),
                ),
            ),
            appearance_updates,
        )
    })
}

/// Preserve installation failures while applying the watcher's first answer.
fn startup_theme_notes(
    loaded_config: &LoadedConfig,
    installation: ThemeInstallation,
    appearance: Option<Appearance>,
) -> StartupThemeNotes {
    let diagnostic = appearance.map_or(AppearanceDiagnostic::NotObserved, |appearance| {
        tui_pane::apply_system_appearance(appearance, &loaded_config.config.appearance).into()
    });
    StartupThemeNotes::new(installation, diagnostic)
}

/// The next visible text change, independent of the scheduler polling rate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TextChange {
    Static,
    At(u64),
}

impl TextChange {
    fn earlier(self, other: Self) -> Self {
        match (self, other) {
            (Self::Static, answer) | (answer, Self::Static) => answer,
            (Self::At(left), Self::At(right)) => Self::At(left.min(right)),
        }
    }
}

/// Each time-dependent text supplies its own next visible change.
enum DisplayText {
    AgentAge { started: u64 },
    AccountReading { read_at: u64 },
    AccountResetDate,
    ShellCountdown { deadline: u64 },
    Notice,
}

impl DisplayText {
    fn next_change(&self, now: u64) -> TextChange {
        match *self {
            Self::AgentAge { started } => {
                let elapsed = now.saturating_sub(started);
                // The age column shows minutes, including minutes within hours and days.
                elapsed
                    .checked_div(60)
                    .and_then(|minutes| minutes.checked_add(1))
                    .and_then(|minutes| minutes.checked_mul(60))
                    .and_then(|seconds| started.checked_add(seconds))
                    .map_or(TextChange::Static, TextChange::At)
            },
            Self::AccountReading { read_at } => {
                let elapsed = now.saturating_sub(read_at);
                let unit = [86_400, 3_600, 60, 1]
                    .into_iter()
                    .find(|unit| elapsed >= *unit)
                    .unwrap_or(1);
                let age = read_at
                    .checked_add((elapsed / unit + 1).saturating_mul(unit))
                    .map_or(TextChange::Static, TextChange::At);
                let stale = read_at
                    .checked_add(crate::constants::ACCOUNTS_STALE_SECONDS + 1)
                    .filter(|at| *at > now)
                    .map_or(TextChange::Static, TextChange::At);
                age.earlier(stale)
            },
            Self::AccountResetDate => {
                // A missing local midnight is retried each second across a clock transition.
                i64::try_from(now)
                    .ok()
                    .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
                    .and_then(|date| date.with_timezone(&Local).date_naive().succ_opt())
                    .and_then(|date| date.and_hms_opt(0, 0, 0))
                    .and_then(|date| Local.from_local_datetime(&date).earliest())
                    .and_then(|date| u64::try_from(date.timestamp()).ok())
                    .filter(|at| *at > now)
                    .map_or_else(|| TextChange::At(now.saturating_add(1)), TextChange::At)
            },
            Self::ShellCountdown { deadline } if now < deadline => TextChange::At(now + 1),
            Self::ShellCountdown { .. } | Self::Notice => TextChange::Static,
        }
    }
}

#[derive(Debug, Default)]
enum BodyDisplayTime {
    #[default]
    Unobserved,
    Observed {
        at:   u64,
        next: TextChange,
    },
}

#[derive(Default)]
enum UptimeDisplayTime {
    #[default]
    Unobserved,
    Observed(u64),
}

/// Body text and Uptime have separate deadlines: only the latter permits a status-only frame.
#[derive(Default)]
struct DisplayTime {
    body:   BodyDisplayTime,
    uptime: UptimeDisplayTime,
}

fn next_body_change(app: &App, now: u64) -> TextChange {
    let mut next = TextChange::Static;
    let machines = app
        .census
        .machines(&app.loaded_config.config.machines.remote);
    let placement = AgentPlacement::from_machines(&machines);
    for thread in &placement.threads {
        let child = &thread.owner_row.children[thread.child_index];
        next = next.earlier(
            DisplayText::AgentAge {
                started: child.started,
            }
            .next_change(now),
        );
    }
    for machine in machines {
        for row in machine.state.rows().iter().filter(|row| row.is_visible()) {
            next = next.earlier(
                DisplayText::AgentAge {
                    started: row.started,
                }
                .next_change(now),
            );
            for child in &row.children {
                next = next.earlier(
                    DisplayText::AgentAge {
                        started: child.started,
                    }
                    .next_change(now),
                );
                if let ChildKind::Shell(ShellTimerObservation::ObservedCountdown(timer)) =
                    child.kind
                {
                    next = next.earlier(
                        DisplayText::ShellCountdown {
                            deadline: timer.deadline,
                        }
                        .next_change(now),
                    );
                }
            }
        }
        if machine.state.hierarchy_notice().is_visible() {
            next = next.earlier(DisplayText::Notice.next_change(now));
        }
    }
    let reading = match &app.accounts.table {
        AccountsTable::Ready(reading) | AccountsTable::FailedKeepingLast { last: reading, .. } => {
            Some(reading)
        },
        AccountsTable::Failed(_)
        | AccountsTable::NotConfigured
        | AccountsTable::AwaitingFirstReading => None,
    };
    if let Some(reading) = reading {
        for row in &reading.accounts {
            if let Reading::Known(at) = row.read_at {
                next = next.earlier(
                    DisplayText::AccountReading {
                        read_at: u64::try_from(at.timestamp()).unwrap_or_default(),
                    }
                    .next_change(now),
                );
            }
            if matches!(row.resets_at, Reading::Known(_)) {
                next = next.earlier(DisplayText::AccountResetDate.next_change(now));
            }
        }
    }
    next
}

/// What the loop folds in on every pass besides input: the census
/// answers, the ages ticking, the favorites overlay's removal fade, the
/// grid's motion and the attract screen's frames.
struct Ticker {
    /// Answers from the census scheduler.
    updates:      Receiver<CensusUpdate>,
    /// Light or dark appearances observed by the watcher.
    appearances:  Receiver<Appearance>,
    /// Deadlines for visible body text and the independent Uptime clock.
    display_time: DisplayTime,
}

impl Ticker {
    /// Poll `updates` for census answers.
    const fn new(updates: Receiver<CensusUpdate>, appearances: Receiver<Appearance>) -> Self {
        Self {
            updates,
            appearances,
            display_time: DisplayTime {
                body:   BodyDisplayTime::Unobserved,
                uptime: UptimeDisplayTime::Unobserved,
            },
        }
    }

    /// Apply every system appearance that arrived since the last pass.
    fn drain_appearances(&self, app: &mut App) -> bool {
        let mut changed = false;
        while let Ok(appearance) = self.appearances.try_recv() {
            app.apply_system_appearance(appearance);
            changed = true;
        }
        changed
    }

    /// Fold in the census answers that have arrived, answering whether
    /// any changed what the summary shows. An answer from a host taken
    /// out of the list since its probe went out is dropped.
    fn drain_census(&self, app: &mut App) -> bool {
        let mut changed = false;
        while let Ok(update) = self.updates.try_recv() {
            match &update {
                CensusUpdate::Local { rows, stage, .. } => {
                    if matches!(stage, ScanStage::Completed) {
                        changed |= app.footer_colors.refresh();
                    }
                    startup_log::startup_event(&format!("local_applied rows={}", rows.len()));
                },
                CensusUpdate::Remote { host, .. } => {
                    startup_log::startup_event(&format!("remote_applied {host}"));
                },
            }
            if let CensusUpdate::Remote { host, .. } = &update
                && !app.loaded_config.config.machines.remote.contains(host)
            {
                continue;
            }
            match update {
                CensusUpdate::Local {
                    stage: ScanStage::Classified,
                    ..
                } => {
                    changed |= app.census.apply(update);
                },
                CensusUpdate::Local {
                    stage: ScanStage::Completed,
                    ..
                }
                | CensusUpdate::Remote { .. } => {
                    changed |= app.census.apply(update);
                    app.prune_tree_sorts();
                },
            }
        }
        changed
    }

    /// Check body text only at its next visible change; a backwards clock rechecks it.
    fn ages_moved(&mut self, app: &App, unix_now: u64) -> bool {
        let was_observed = !matches!(self.display_time.body, BodyDisplayTime::Unobserved);
        let changed = match self.display_time.body {
            BodyDisplayTime::Unobserved => true,
            BodyDisplayTime::Observed { at, .. } if at == unix_now => return false,
            BodyDisplayTime::Observed {
                at,
                next: TextChange::At(next),
            } => {
                if unix_now >= at && unix_now < next {
                    return false;
                }
                true
            },
            BodyDisplayTime::Observed {
                next: TextChange::Static,
                ..
            } => false,
        };
        let next = next_body_change(app, unix_now);
        self.display_time.body = BodyDisplayTime::Observed { at: unix_now, next };
        if changed {
            was_observed || next != TextChange::Static
        } else {
            next != TextChange::Static
        }
    }

    fn uptime_moved(&mut self, app: &App, now: Instant) -> Repaint {
        let seconds = now.saturating_duration_since(app.started).as_secs();
        let changed = matches!(self.display_time.uptime,
            UptimeDisplayTime::Observed(previous) if previous != seconds);
        self.display_time.uptime = UptimeDisplayTime::Observed(seconds);
        if changed {
            Repaint::Needed
        } else {
            Repaint::NotNeeded
        }
    }

    /// [`PollWork::poll`] at `unix_now` in unix seconds.
    fn poll_at(&mut self, app: &mut App, now: Instant, unix_now: u64) -> Repaint {
        app.prepare_full_frame();
        let accounts_changed = app.accounts.refresh();
        let census_changed = self.drain_census(app);
        let appearance_changed = self.drain_appearances(app);
        if accounts_changed || census_changed || appearance_changed {
            self.display_time.body = BodyDisplayTime::Unobserved;
        }
        let ages_moved = self.ages_moved(app, unix_now);
        let uptime_moved = self.uptime_moved(app, now);
        // poll_favorites, TileGrid::tick and Attract::frame_due advance
        // their state every pass, even when another requests a repaint.
        let favorites = tui_pane::poll_favorites(app, now);
        // TileGrid::tick requests repaints until app.tiles settles.
        let grid_moving = app.tiles.tick();
        // Attract::frame_due requests frames while app.attract is shown
        // and once more when its idle delay expires.
        let attract = app.attract.frame_due();
        if accounts_changed
            || census_changed
            || appearance_changed
            || ages_moved
            || favorites == Repaint::Needed
            || grid_moving
            || attract == Repaint::Needed
        {
            app.invalidate_body();
            Repaint::Needed
        } else if uptime_moved == Repaint::Needed {
            app.request_clock_frame();
            Repaint::Needed
        } else {
            Repaint::NotNeeded
        }
    }
}

impl PollWork<App> for Ticker {
    fn poll(&mut self, app: &mut App, now: Instant) -> Repaint {
        self.poll_at(app, now, census::unix_now())
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::sync::mpsc;

    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use tui_pane::FrameworkOverlayId;
    use tui_pane::TerminalApp;
    use tui_pane::dispatch_key;

    use super::*;
    use crate::agent_cell::TreeColumn;
    use crate::census::Account;
    use crate::census::Agent;
    use crate::census::AgentActivity;
    use crate::census::HierarchyEdges;
    use crate::census::MachineState;
    use crate::census::RoleFallback;
    use crate::census::ServiceTier;
    use crate::census::SessionIdentity;
    use crate::census::Timer;
    use crate::census::accounts::AccountsFailure;
    use crate::census::accounts::AccountsReading;
    use crate::census::accounts::AccountsTable;
    use crate::sort::TreeSort;
    use crate::tiles::TileKey;

    #[test]
    fn unchanged_account_failure_needs_no_clock_frame() {
        let mut app = App::new_for_test().expect("test app");
        app.census.apply(CensusUpdate::Local {
            rows:      Vec::new(),
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Completed,
        });
        app.accounts.table = AccountsTable::Failed(AccountsFailure::InvalidReading);
        let mut ticker = Ticker::new(mpsc::channel().1, mpsc::channel().1);
        assert!(!ticker.ages_moved(&app, 100));
        assert!(!ticker.ages_moved(&app, 100));
        assert!(!ticker.ages_moved(&app, 101));
    }

    #[test]
    fn idle_display_time_skips_unchanged_old_ages() {
        let mut app = App::new_for_test().expect("test app");
        let row: AgentRow = serde_json::from_value(serde_json::json!({
            "agent":"claude", "name":"root", "pid":10,
            "started":100, "process_started":100, "directory":"/fixture",
            "desktop":null, "branch":null, "status":"idle", "launched_by":null, "children":[]
        }))
        .expect("fixture agent");
        app.census.apply(CensusUpdate::Local {
            rows:      vec![row],
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Completed,
        });
        let mut ticker = Ticker::new(mpsc::channel().1, mpsc::channel().1);
        assert_eq!(next_body_change(&app, 1000), TextChange::At(1060));
        assert!(ticker.ages_moved(&app, 1000));
        let mut draws = 0;
        for now in 1001..1060 {
            draws += usize::from(ticker.ages_moved(&app, now));
        }
        assert_eq!(draws, 0, "the displayed 15 minute age is unchanged");
        let moved = ticker.ages_moved(&app, 1060);
        assert!(moved, "{:?}", ticker.display_time.body);
    }

    #[test]
    fn idle_display_time_repaints_agent_ages_only_at_their_minute_boundary() {
        let mut app = App::new_for_test().expect("test app");
        let row: AgentRow = serde_json::from_value(serde_json::json!({
            "agent":"claude", "name":"root", "pid":10,
            "started":90, "process_started":90, "directory":"/fixture",
            "desktop":null, "branch":null, "status":"idle", "launched_by":null, "children":[]
        }))
        .expect("fixture agent");
        app.census.apply(CensusUpdate::Local {
            rows:      vec![row],
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Completed,
        });
        let mut ticker = Ticker::new(mpsc::channel().1, mpsc::channel().1);
        assert_eq!(next_body_change(&app, 100), TextChange::At(150));
        let frames: Vec<_> = [100, 101, 149, 150, 151, 209, 210]
            .into_iter()
            .map(|now| ticker.ages_moved(&app, now))
            .collect();
        assert_eq!(
            frames,
            [true, false, false, true, false, false, true],
            "the first observation and each displayed minute request a body frame"
        );
    }

    /// Freeze Uptime and settle motion, so only a body deadline can draw another body.
    fn settle_body(
        app: &mut App,
        ticker: &mut Ticker,
        at: u64,
    ) -> (Instant, Terminal<TestBackend>) {
        let now = app.started;
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24))
            .expect("test terminal");
        let keymap = std::rc::Rc::clone(&app.keymap);
        for _ in 0..2 {
            ticker.poll_at(app, now, at);
            terminal
                .draw(|frame| app.draw(frame, &keymap))
                .expect("settle body");
            app.tiles.settle_for_test();
        }
        (now, terminal)
    }

    fn assert_body_boundary(
        app: &mut App,
        ticker: &mut Ticker,
        terminal: &mut Terminal<TestBackend>,
        now: Instant,
        at: u64,
        expected: Repaint,
    ) {
        let before = app.full_frame_count();
        let actual = ticker.poll_at(app, now, at);
        assert_eq!(actual, expected, "body repaint at {at}");
        if actual == Repaint::Needed {
            let keymap = std::rc::Rc::clone(&app.keymap);
            terminal
                .draw(|frame| app.draw(frame, &keymap))
                .expect("body frame");
        }
        assert_eq!(
            app.full_frame_count() - before,
            usize::from(expected == Repaint::Needed),
            "body layout at {at}"
        );
    }

    #[test]
    fn managed_worker_age_repaints_at_its_own_minute_boundary() {
        let mut app = App::new_for_test().expect("test app");
        let (manager, worker) = crate::census::mesh_worker_fixture();
        app.loaded_config.config.machines.remote = vec!["mac".into()];
        app.census.apply(CensusUpdate::Local {
            rows:      manager.rows().to_vec(),
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Completed,
        });
        app.census.apply(CensusUpdate::Remote {
            host:  "mac".into(),
            state: worker,
        });
        let mut ticker = Ticker::new(mpsc::channel().1, mpsc::channel().1);
        let (now, mut terminal) = settle_body(&mut app, &mut ticker, 99);
        assert_body_boundary(
            &mut app,
            &mut ticker,
            &mut terminal,
            now,
            99,
            Repaint::NotNeeded,
        );
        assert_body_boundary(
            &mut app,
            &mut ticker,
            &mut terminal,
            now,
            100,
            Repaint::Needed,
        );
        assert_body_boundary(
            &mut app,
            &mut ticker,
            &mut terminal,
            now,
            101,
            Repaint::NotNeeded,
        );
    }

    #[test]
    fn shell_countdown_ticks_and_expiry_request_body_frames() {
        let mut app = App::new_for_test().expect("test app");
        let mut row: AgentRow = serde_json::from_value(serde_json::json!({
            "agent":"claude", "name":"root", "pid":10,
            "started":90, "process_started":90, "directory":"/fixture",
            "desktop":null, "branch":null, "status":"idle", "launched_by":null, "children":[]
        }))
        .expect("fixture agent");
        let mut child = crate::census::mesh_worker_fixture().1.rows()[0].children[0].clone();
        child.kind = ChildKind::Shell(ShellTimerObservation::ObservedCountdown(Timer {
            started:  100,
            deadline: 103,
        }));
        child.started = 90;
        row.children = vec![child];
        app.census.apply(CensusUpdate::Local {
            rows:      vec![row],
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Completed,
        });
        let mut ticker = Ticker::new(mpsc::channel().1, mpsc::channel().1);
        let (now, mut terminal) = settle_body(&mut app, &mut ticker, 100);
        for (at, expected) in [
            (100, Repaint::NotNeeded),
            (101, Repaint::Needed),
            (102, Repaint::Needed),
            (103, Repaint::Needed),
            (104, Repaint::NotNeeded),
        ] {
            assert_body_boundary(&mut app, &mut ticker, &mut terminal, now, at, expected);
        }
    }

    #[test]
    fn account_reading_age_and_staleness_request_body_frames() {
        let mut app = App::new_for_test().expect("test app");
        app.accounts.table = AccountsTable::Ready(AccountsReading::parse(br#"{"accounts":[{"label":"fixture","roles":[],"weekly_left_percent":50,"resets_at":null,"credits":null,"use_credits":false,"read_ok":true,"read_at":"1970-01-01T00:01:40Z"}],"problems":[]}"#).expect("account reading"));
        let mut ticker = Ticker::new(mpsc::channel().1, mpsc::channel().1);
        let (now, mut terminal) = settle_body(&mut app, &mut ticker, 100);
        for (at, expected) in [
            (100, Repaint::NotNeeded),
            (101, Repaint::Needed),
            (159, Repaint::Needed),
            (160, Repaint::Needed),
            (161, Repaint::NotNeeded),
            (220, Repaint::Needed),
            (3699, Repaint::Needed),
            (3700, Repaint::Needed),
            (3701, Repaint::Needed),
            (3702, Repaint::NotNeeded),
        ] {
            assert_body_boundary(&mut app, &mut ticker, &mut terminal, now, at, expected);
        }
    }

    #[test]
    fn idle_display_time_tracks_each_text_boundary() {
        for elapsed in [0, 59, 60, 3_599, 3_600, 86_399, 86_400] {
            assert_eq!(
                DisplayText::AgentAge { started: 100 }.next_change(100 + elapsed),
                TextChange::At(100 + (elapsed / 60 + 1) * 60)
            );
        }
        for (elapsed, next) in [
            (0, 1),
            (59, 60),
            (60, 120),
            (3_599, 3_600),
            (3_600, 3_601),
            (3_601, 7_200),
            (86_400, 172_800),
        ] {
            assert_eq!(
                DisplayText::AccountReading { read_at: 100 }.next_change(100 + elapsed),
                TextChange::At(100 + next),
                "reading age {elapsed}"
            );
        }
        assert_eq!(
            DisplayText::ShellCountdown { deadline: 120 }.next_change(119),
            TextChange::At(120)
        );
        assert_eq!(
            DisplayText::ShellCountdown { deadline: 120 }.next_change(120),
            TextChange::Static
        );
        assert_eq!(DisplayText::Notice.next_change(100), TextChange::Static);
        assert!(
            matches!(DisplayText::AccountResetDate.next_change(1_700_000_000),
            TextChange::At(at) if at > 1_700_000_000 && at <= 1_700_086_400)
        );
    }

    fn key(code: KeyCode) -> KeyEvent { KeyEvent::new(code, KeyModifiers::NONE) }

    fn ctrl(character: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL)
    }

    /// Classification and cached links do not each trigger a footer file check.
    #[test]
    fn a_completed_local_scan_checks_the_footer_once() {
        let directory = tempfile::tempdir().expect("a fixture footer directory");
        let path = directory.path().join("statusline.conf");
        std::fs::write(&path, "account.codex1=teal").expect("write a fixture footer");
        let mut app = App::new_for_test().expect("test app should build");
        app.footer_colors = crate::footer_colors::FooterColorFile::fixture(path);
        let (sender, updates) = mpsc::channel();
        let (_appearance_sender, appearances) = mpsc::channel();
        let ticker = Ticker::new(updates, appearances);

        for interim_answers in [2, 1] {
            let before = app.footer_colors.revision_checks();
            for _ in 0..interim_answers {
                sender
                    .send(CensusUpdate::Local {
                        rows:      Vec::new(),
                        hierarchy: HierarchyEdges::default().into(),
                        stage:     ScanStage::Classified,
                    })
                    .expect("publish an interim local answer");
            }
            ticker.drain_census(&mut app);
            assert_eq!(app.footer_colors.revision_checks(), before);

            sender
                .send(CensusUpdate::Local {
                    rows:      Vec::new(),
                    hierarchy: HierarchyEdges::default().into(),
                    stage:     ScanStage::Completed,
                })
                .expect("publish the completed scan");
            let changed = ticker.drain_census(&mut app);
            assert_eq!(app.footer_colors.revision_checks(), before + 1);
            assert_eq!(
                app.footer_colors.colors().account("codex1"),
                crate::footer_colors::FooterTint::Footer(crate::footer_colors::FooterColor::Teal)
            );
            assert_eq!(changed, before == 0, "only the first palette changes");
            assert!(!ticker.drain_census(&mut app));
            assert_eq!(app.footer_colors.revision_checks(), before + 1);
        }
    }

    /// Interim answers preserve preferences; completed scans prune departed roots.
    #[test]
    fn the_ticker_prunes_tree_sorts_after_completed_census_answers() {
        let mut app = App::new_for_test().expect("a test app");
        let row: AgentRow = serde_json::from_value(serde_json::json!({
            "agent":"claude", "name":"root", "pid":10,
            "started":100, "process_started":100, "directory":"/fixture",
            "desktop":null, "branch":null, "status":"idle", "launched_by":null, "children":[]
        }))
        .expect("a fixture agent");
        let id = TileKey::Unassigned {
            machine: "natedev".to_string(),
        };
        let (sender, updates) = mpsc::channel();
        let (_appearance_sender, appearances) = mpsc::channel();
        let ticker = Ticker::new(updates, appearances);
        sender
            .send(CensusUpdate::Local {
                rows:      vec![row.clone()],
                hierarchy: HierarchyEdges::default().into(),
                stage:     ScanStage::Completed,
            })
            .expect("publish a completed scan");
        ticker.drain_census(&mut app);
        let mut sort = TreeSort::default();
        sort.select(TreeColumn::Role);
        app.tree_sorts.insert(id.clone(), sort);

        sender
            .send(CensusUpdate::Local {
                rows:      Vec::new(),
                hierarchy: HierarchyEdges::default().into(),
                stage:     ScanStage::Classified,
            })
            .expect("publish an interim answer");
        ticker.drain_census(&mut app);
        assert_eq!(app.tree_sorts.get(&id), Some(&sort));

        sender
            .send(CensusUpdate::Local {
                rows:      Vec::new(),
                hierarchy: HierarchyEdges::default().into(),
                stage:     ScanStage::Completed,
            })
            .expect("publish a completed scan");
        ticker.drain_census(&mut app);
        assert!(app.tree_sorts.is_empty());

        app.loaded_config.config.machines.remote = vec!["mac".to_string()];
        sender
            .send(CensusUpdate::Remote {
                host:  "mac".to_string(),
                state: MachineState::Answered(vec![row], HierarchyEdges::default().into()),
            })
            .expect("publish a remote answer");
        ticker.drain_census(&mut app);
        let remote_id = TileKey::Unassigned {
            machine: "mac".to_string(),
        };
        app.tree_sorts.insert(remote_id.clone(), sort);
        sender
            .send(CensusUpdate::Remote {
                host:  "unconfigured".to_string(),
                state: MachineState::Failed("fixture failure".to_string()),
            })
            .expect("publish an unconfigured answer");
        ticker.drain_census(&mut app);
        assert_eq!(app.tree_sorts.get(&remote_id), Some(&sort));
        sender
            .send(CensusUpdate::Remote {
                host:  "mac".to_string(),
                state: MachineState::Answered(Vec::new(), HierarchyEdges::default().into()),
            })
            .expect("publish a completed remote answer");
        ticker.drain_census(&mut app);
        assert!(app.tree_sorts.is_empty());
    }

    /// An idle app with a settled grid costs the loop nothing, and `+`
    /// sets the grid moving, which does. The grid is laid out by hand,
    /// as a drawn frame would leave it, so there is room for the cell.
    #[test]
    fn the_ticker_repaints_only_while_something_moves() {
        let mut app = App::new_for_test().expect("test app should build");
        let growth = app.loaded_config.config.tiles.growth();
        app.tiles.set_layout(Rect::new(0, 0, 80, 23), growth);
        let (_sender, receiver) = mpsc::channel();
        let (_appearance_sender, appearance_receiver) = mpsc::channel();
        let mut ticker = Ticker::new(receiver, appearance_receiver);
        assert_eq!(ticker.poll(&mut app, Instant::now()), Repaint::NotNeeded);

        dispatch_key(&mut app, key(KeyCode::Char('+')));

        assert_eq!(ticker.poll(&mut app, Instant::now()), Repaint::Needed);
    }

    /// Carriers alone do not animate ages locally or on a configured remote.
    #[test]
    fn transport_hosts_do_not_request_age_repaints() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = App::new_for_test().expect("test app should build");
        let (_sender, receiver) = mpsc::channel();
        let (_appearance_sender, appearance_receiver) = mpsc::channel();
        let mut ticker = Ticker::new(receiver, appearance_receiver);
        let (manager, worker) = census::mesh_worker_fixture();
        app.census.apply(CensusUpdate::Local {
            rows:      worker.rows().to_vec(),
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Classified,
        });
        app.loaded_config.config.machines.remote = vec!["mac".to_string()];
        app.census.apply(CensusUpdate::Remote {
            host:  "mac".to_string(),
            state: worker,
        });
        assert!(!ticker.ages_moved(&app, 100));
        assert!(!ticker.ages_moved(&app, 101));
        app.census.apply(CensusUpdate::Remote {
            host:  "mac".to_string(),
            state: manager,
        });
        // A census update invalidates the body deadlines before the same-second check.
        ticker.display_time.body = BodyDisplayTime::Unobserved;
        assert!(ticker.ages_moved(&app, 101));
        assert!(!ticker.ages_moved(&app, 101));
        assert!(!ticker.ages_moved(&app, 102));
        let TextChange::At(next) = next_body_change(&app, 102) else {
            return Err("the visible agent has a minute deadline".into());
        };
        assert!(ticker.ages_moved(&app, next));
        Ok(())
    }

    /// A visible age boundary or changed census answer requests a body frame.
    #[test]
    fn shown_rows_repaint_at_age_boundaries_and_on_a_changed_answer() {
        let mut app = App::new_for_test().expect("test app should build");
        let (sender, receiver) = mpsc::channel();
        let (_appearance_sender, appearance_receiver) = mpsc::channel();
        let mut ticker = Ticker::new(receiver, appearance_receiver);
        let now = Instant::now();
        let row = AgentRow {
            agent:           Agent::Claude,
            role_fallback:   RoleFallback::None,
            session_id:      SessionIdentity::Unrecorded,
            service_tier:    ServiceTier::Unrecorded,
            account:         Account::Unrecorded,
            name:            "enh/handler".to_string(),
            activity:        AgentActivity::from(Some("busy")),
            started:         90,
            process_started: 90,
            pid:             428_044,
            desktop:         None,
            directory:       "~/rust/handler".to_string(),
            branch:          None,
            launched_by:     None,
            children:        Vec::new(),
        };
        sender
            .send(CensusUpdate::Local {
                rows:      vec![row.clone()],
                hierarchy: HierarchyEdges::default().into(),
                stage:     ScanStage::Classified,
            })
            .expect("the ticker should hold the receiver");

        assert_eq!(ticker.poll_at(&mut app, now, 100), Repaint::Needed);
        assert_eq!(ticker.poll_at(&mut app, now, 100), Repaint::NotNeeded);
        assert_eq!(ticker.poll_at(&mut app, now, 101), Repaint::NotNeeded);
        assert_eq!(ticker.poll_at(&mut app, now, 150), Repaint::Needed);

        sender
            .send(CensusUpdate::Local {
                rows:      vec![row.clone()],
                hierarchy: HierarchyEdges::default().into(),
                stage:     ScanStage::Classified,
            })
            .expect("the ticker should hold the receiver");
        assert_eq!(ticker.poll_at(&mut app, now, 150), Repaint::NotNeeded);
        sender
            .send(CensusUpdate::Local {
                rows:      vec![census::AgentRow {
                    role_fallback: crate::census::RoleFallback::None,
                    activity: crate::census::AgentActivity::Unreported,
                    ..row
                }],
                hierarchy: HierarchyEdges::default().into(),
                stage:     ScanStage::Classified,
            })
            .expect("the ticker should hold the receiver");
        assert_eq!(ticker.poll_at(&mut app, now, 150), Repaint::Needed);
    }

    /// The favorites modal owns the keyboard while it is open: `?`
    /// opens no framework overlay behind it, and `Esc` closes it.
    #[test]
    fn the_favorites_overlay_swallows_keys_until_escape() {
        let mut app = App::new_for_test().expect("test app should build");
        dispatch_key(&mut app, ctrl('o'));
        assert!(app.favorites_overlay.is_open());

        dispatch_key(&mut app, key(KeyCode::Char('?')));
        assert_eq!(app.framework.overlay(), None);
        assert!(app.favorites_overlay.is_open());

        dispatch_key(&mut app, key(KeyCode::Esc));
        assert!(!app.favorites_overlay.is_open());
    }

    /// `s`, ctrl-k and `?` each open their framework overlay, and `Esc`
    /// closes it.
    #[test]
    fn each_framework_overlay_opens_and_escape_closes_it() {
        let mut app = App::new_for_test().expect("test app should build");
        for (overlay, opener) in [
            (FrameworkOverlayId::Settings, key(KeyCode::Char('s'))),
            (FrameworkOverlayId::Keymap, ctrl('k')),
            (FrameworkOverlayId::GlobalShortcuts, key(KeyCode::Char('?'))),
        ] {
            dispatch_key(&mut app, opener);
            assert_eq!(app.framework.overlay(), Some(overlay));
            dispatch_key(&mut app, key(KeyCode::Esc));
            assert_eq!(app.framework.overlay(), None);
        }
    }
}
