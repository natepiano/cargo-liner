//! Starting the grid: load its configuration, build the app, and run it
//! under [`tui_pane::run_terminal`] with the grid's motion, the attract
//! screen's frames and the favorites overlay's fades as the work the
//! loop polls.

use std::process::ExitCode;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use tui_pane::Appearance;
use tui_pane::PollWork;
use tui_pane::Repaint;
use tui_pane::install_theme;
use tui_pane::run_terminal;
use tui_pane::spawn_appearance_watcher;

use crate::app::App;
use crate::census;
#[cfg(test)]
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::account::AccountLabels;
use crate::census::scan::ScanStage;
use crate::config::CargoHandler;
use crate::config::LoadedConfig;
use crate::constants::BINARY_NAME;
use crate::startup_log;
use crate::theme;

#[cfg(test)]
#[path = "terminal/startup_tests.rs"]
mod startup_tests;

/// Load configuration, install the theme, build the keymap, and run the
/// event loop with the terminal in the alternate screen.
pub(crate) fn run() -> ExitCode {
    startup_log::init();
    let loaded_config = LoadedConfig::load::<CargoHandler>();
    startup_log::startup_event("config_loaded");
    let mut startup_note = install_theme(&loaded_config.config.appearance, theme::builtins());
    let (appearance_sender, appearance_updates) = mpsc::channel();
    let startup_appearance = spawn_appearance_watcher(move |appearance| {
        let _ = appearance_sender.send(appearance);
    });
    startup_log::startup_event("appearance_read_returned");
    if let Some(appearance) = startup_appearance {
        startup_note =
            tui_pane::apply_system_appearance(appearance, &loaded_config.config.appearance);
    }
    // Read before the config is handed to the app, which takes it.
    let iterm2_profile = loaded_config.config.appearance.iterm2_profile.clone();
    let mut app = match App::new(loaded_config, startup_note) {
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

/// What the loop folds in on every pass besides input: the census
/// answers, the ages ticking, the favorites overlay's removal fade, the
/// grid's motion and the attract screen's frames.
struct Ticker {
    /// Answers from the census scheduler.
    updates:      Receiver<CensusUpdate>,
    /// Light or dark appearances observed by the watcher.
    appearances:  Receiver<Appearance>,
    /// The unix second the ages were last drawn at, while any row is
    /// shown.
    shown_second: Option<u64>,
}

impl Ticker {
    /// Poll `updates` for census answers.
    const fn new(updates: Receiver<CensusUpdate>, appearances: Receiver<Appearance>) -> Self {
        Self {
            updates,
            appearances,
            shown_second: None,
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

    /// Whether the ages need drawing again: some row is shown and the
    /// unix second has moved on from `unix_now` since they were drawn.
    fn ages_moved(&mut self, app: &App, unix_now: u64) -> bool {
        let shown = app
            .census
            .lists_an_agent(&app.loaded_config.config.machines.remote);
        let shown = shown
            || app.accounts.table.is_visible()
            || app
                .census
                .has_hierarchy_notice(&app.loaded_config.config.machines.remote);
        if !shown || self.shown_second == Some(unix_now) {
            return false;
        }
        self.shown_second = Some(unix_now);
        true
    }

    /// [`PollWork::poll`] at `unix_now` in unix seconds.
    fn poll_at(&mut self, app: &mut App, now: Instant, unix_now: u64) -> Repaint {
        let accounts_changed = app.accounts.refresh();
        let census_changed = self.drain_census(app);
        let appearance_changed = self.drain_appearances(app);
        let ages_moved = self.ages_moved(app, unix_now);
        // Each of the three is asked every pass: asking is what moves it
        // on, so none may be skipped because another already wants a
        // frame.
        let favorites = tui_pane::poll_favorites(app, now);
        // A grid in motion repaints every poll until it settles.
        let grid_moving = app.tiles.tick();
        // The attract screen asks for frames on its own cadence while
        // it is on the screen, and for one more at the end of the quiet
        // it waits out before coming back.
        let attract = app.attract.frame_due();
        if accounts_changed
            || census_changed
            || appearance_changed
            || ages_moved
            || favorites == Repaint::Needed
            || grid_moving
            || attract == Repaint::Needed
        {
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
    use ratatui::layout::Rect;
    use tui_pane::FrameworkOverlayId;
    use tui_pane::dispatch_key;

    use super::*;
    use crate::agent_cell::TreeColumn;
    use crate::census::Account;
    use crate::census::Agent;
    use crate::census::AgentActivity;
    use crate::census::HierarchyEdges;
    use crate::census::MachineState;
    use crate::census::ServiceTier;
    use crate::census::SessionIdentity;
    use crate::census::accounts::AccountsFailure;
    use crate::census::accounts::AccountsTable;
    use crate::sort::TreeSort;
    use crate::tiles::TileKey;

    #[test]
    fn failed_accounts_without_agents_repaint_each_new_second() {
        let mut app = App::new_for_test().expect("test app");
        app.census.apply(CensusUpdate::Local {
            rows:      Vec::new(),
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Completed,
        });
        app.accounts.table = AccountsTable::Failed(AccountsFailure::InvalidReading);
        let mut ticker = Ticker::new(mpsc::channel().1, mpsc::channel().1);
        assert!(ticker.ages_moved(&app, 100));
        assert!(!ticker.ages_moved(&app, 100));
        assert!(ticker.ages_moved(&app, 101));
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
    fn transport_hosts_do_not_request_age_repaints() {
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
        assert!(ticker.ages_moved(&app, 101));
        assert!(!ticker.ages_moved(&app, 101));
        assert!(ticker.ages_moved(&app, 102));
    }

    /// While a row is shown each new unix second repaints so the ages
    /// tick, and a census answer that changes something repaints at
    /// once.
    #[test]
    fn shown_rows_repaint_each_second_and_on_a_changed_answer() {
        let mut app = App::new_for_test().expect("test app should build");
        let (sender, receiver) = mpsc::channel();
        let (_appearance_sender, appearance_receiver) = mpsc::channel();
        let mut ticker = Ticker::new(receiver, appearance_receiver);
        let now = Instant::now();
        let row = AgentRow {
            agent:           Agent::Claude,
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
        assert_eq!(ticker.poll_at(&mut app, now, 101), Repaint::Needed);

        sender
            .send(CensusUpdate::Local {
                rows:      vec![row.clone()],
                hierarchy: HierarchyEdges::default().into(),
                stage:     ScanStage::Classified,
            })
            .expect("the ticker should hold the receiver");
        assert_eq!(ticker.poll_at(&mut app, now, 101), Repaint::NotNeeded);
        sender
            .send(CensusUpdate::Local {
                rows:      vec![census::AgentRow {
                    activity: crate::census::AgentActivity::Unreported,
                    ..row
                }],
                hierarchy: HierarchyEdges::default().into(),
                stage:     ScanStage::Classified,
            })
            .expect("the ticker should hold the receiver");
        assert_eq!(ticker.poll_at(&mut app, now, 101), Repaint::Needed);
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
