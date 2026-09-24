//! Starting the grid: load its configuration, build the app, and run it
//! under [`tui_pane::run_terminal`] with the scans and the sccache reads
//! as the work the loop polls.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::time::Instant;

use tui_pane::PollWork;
use tui_pane::Repaint;
use tui_pane::Updates;
use tui_pane::install_theme;
use tui_pane::run_terminal;

use crate::app::App;
use crate::capture;
use crate::census;
use crate::census::CargoGroup;
use crate::config::CargoTile;
use crate::config::Config;
use crate::config::LoadedConfig;
use crate::constants::ATTRACT_FRAME_INTERVAL;
use crate::constants::CAPTURE_ROOT;
use crate::favorites;
use crate::favorites_overlay::FavoritesOverlayFrameOutcome;
use crate::progress::capture_roots::AccountCaptureDirectory;
use crate::root_scan::SharedCaptureDirectory;
use crate::sccache;
use crate::sccache::SccacheServer;
use crate::sccache::SccacheSummary;
use crate::theme;

/// One scan's account of the machine: the cargo commands running, and
/// whether an sccache server is up behind them.
///
/// The two travel together because they are read together. Phase one
/// already names every process to find the compilers under each cargo,
/// and a running server is one more name in that same pass -- which is
/// what makes the answer free, and what keeps the summary's stats read
/// from having to start a server to discover whether one is running.
pub(crate) struct Scan {
    /// The commands running, newest first.
    pub(crate) groups:           Vec<CargoGroup>,
    /// Whether a process named [`crate::constants::SCCACHE_BINARY`] was among them.
    pub(crate) sccache:          SccacheServer,
    /// Settings reads these observations without reopening any capture path.
    pub(crate) root_status:      Vec<AccountCaptureDirectory>,
    pub(crate) shared_directory: SharedCaptureDirectory,
}

/// Load configuration, install the theme, build the keymap, and run the
/// event loop with the terminal in the alternate screen.
pub(crate) fn run() -> ExitCode { run_with_capture_parent(std::path::PathBuf::from(CAPTURE_ROOT)) }

/// The executable supplies the fixed parent; PTY tests supply an isolated path.
pub(crate) fn run_with_capture_parent(parent: PathBuf) -> ExitCode {
    run_with_scanner(move |config| {
        census::spawn_with_resolver(config, move || {
            crate::progress::capture_roots::CaptureRoots::from_parent(&parent)
        })
        .0
    })
}

fn run_with_scanner(spawn: impl FnOnce(&Config) -> Receiver<Scan>) -> ExitCode {
    let loaded_config = LoadedConfig::load::<CargoTile>();
    let startup_note = install_theme(
        &loaded_config.config.appearance,
        theme::builtins::builtins(),
    );
    // Read before the config is handed to the app, which takes it.
    let iterm2_profile = loaded_config.config.appearance.iterm2_profile.clone();
    let mut app = match App::new(loaded_config, startup_note) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("cargo-tile: keymap: {error}");
            return ExitCode::FAILURE;
        },
    };
    // Before the terminal is taken: the toasts it pushes are drawn by
    // the first frame, and a failure here is a notice, never an exit.
    capture::stand_up(&mut app);

    run_terminal(&mut app, &iterm2_profile, |app| {
        Workers::new(spawn(&app.loaded_config.config))
    })
}

/// What the loop folds in on every pass besides input: the process
/// scans, the sccache reads, the favorites overlay's removal fade, the
/// roster's fades, the grid's motion and the attract screen's frames.
struct Workers {
    /// Process scans from the census worker.
    scans:           Receiver<Scan>,
    /// Each due read runs on a worker of its own and replies here, so a
    /// server that has wedged parks that one thread rather than the loop.
    sccache_reads:   Sender<SccacheSummary>,
    /// Where those reads reply.
    sccache_replies: Receiver<SccacheSummary>,
    /// When the attract screen last asked for a frame.
    attracted:       Instant,
}

impl Workers {
    /// Poll `scans`, with a fresh channel for the sccache reads.
    fn new(scans: Receiver<Scan>) -> Self {
        let (sccache_reads, sccache_replies) = mpsc::channel();
        Self {
            scans,
            sccache_reads,
            sccache_replies,
            attracted: Instant::now(),
        }
    }
}

impl PollWork<App> for Workers {
    fn poll(&mut self, app: &mut App, now: Instant) -> Repaint {
        let mut dirty = false;
        match app.favorites_overlay.advance(now) {
            FavoritesOverlayFrameOutcome::Quiet => {},
            FavoritesOverlayFrameOutcome::Repaint => dirty = true,
            FavoritesOverlayFrameOutcome::CommitRemoval(removal_target) => {
                let result = favorites::remove(removal_target.clone());
                app.favorites_overlay.finish_removal(removal_target, result);
                dirty = true;
            },
        }
        // Frozen, every one of these is skipped: what a scan found,
        // how far a fade has walked and where a travelling cell has
        // reached are the whole of what moves on this screen. The
        // channels are still emptied, so nothing queues up behind the
        // freeze and the first scan after it describes the world as it
        // is then rather than as it was when `f` was pressed.
        if app.updates == Updates::Frozen {
            discard_scans(&self.scans, &self.sccache_replies);
        } else {
            if drain_scans(app, &self.scans) {
                dirty = true;
            }
            // The scan above is what says whether a server is up, so
            // the read is claimed after it rather than before: on the
            // first pass that ordering is the difference between the
            // border filling in straight away and waiting out an
            // interval for the next tick.
            if drain_sccache(app, &self.sccache_replies) {
                dirty = true;
            }
            sccache::refresh_if_due(&mut app.sccache, &self.sccache_reads, Instant::now());
            // A finished row walks its grey toward the ground it is
            // drawn on for the configured spell and then goes, taking
            // its cell with it. Nothing external announces either the
            // steps or the moment, so the poll is what carries them.
            if app
                .roster
                .advance(Instant::now(), app.loaded_config.config.tiles.fade())
            {
                dirty = true;
            }
            // A grid in motion repaints every poll until it settles,
            // which is the one thing here that draws without an event
            // behind it.
            if app.tiles.tick() {
                dirty = true;
            }
            // The attract screen is the other: it runs while nothing is
            // building, which is exactly when this loop would otherwise
            // have nothing to repaint for. It asks for frames only
            // while it is on the screen, so an app with work in front
            // of it goes back to costing nothing.
            //
            // And one more at the end of the quiet it waits out before
            // coming back, which is time nothing else repaints for
            // either: an empty grid standing still. Without that frame
            // the screen would be due and nothing would be drawing to
            // let it back on.
            //
            // Asked for on its own cadence rather than at every poll:
            // a frame of it is every cell of the window, and the
            // terminal parses the whole screen for each one. See
            // [`ATTRACT_FRAME_INTERVAL`].
            if app.attract.showing() && self.attracted.elapsed() >= ATTRACT_FRAME_INTERVAL {
                self.attracted = Instant::now();
                dirty = true;
            }
            if app.attract.due_back(Instant::now()) {
                dirty = true;
            }
        }
        if dirty {
            Repaint::Needed
        } else {
            Repaint::NotNeeded
        }
    }
}

/// Empty both worker channels without reading anything out of them,
/// for a display being held still.
///
/// The workers go on scanning while the screen is frozen -- stopping
/// them would mean unfreezing to a world minutes stale, and restarting
/// them is a cost paid at exactly the moment the reader wants to see
/// something. Left alone the queues would instead grow for as long as
/// the freeze lasts, and unfreezing would walk the display through
/// every scan taken in between.
fn discard_scans(scans: &Receiver<Scan>, sccache_replies: &Receiver<SccacheSummary>) {
    while scans.try_recv().is_ok() {}
    while sccache_replies.try_recv().is_ok() {}
}

/// Take the newest process scan, reporting whether it changed anything.
///
/// Only the newest matters: an older scan queued behind it describes a
/// world that has already moved on.
fn drain_scans(app: &mut App, scans: &Receiver<Scan>) -> bool {
    let mut latest: Option<Scan> = None;
    while let Ok(scan) = scans.try_recv() {
        latest = Some(scan);
    }
    let Some(scan) = latest else {
        return false;
    };
    app.sccache.observe_server(scan.sccache);
    let status_changed =
        app.root_status != scan.root_status || app.shared_directory != scan.shared_directory;
    app.shared_directory = scan.shared_directory;
    app.root_status = scan.root_status;
    app.roster.observe(scan.groups, Instant::now()) || status_changed
}

/// Take whatever the sccache workers have replied, reporting whether the
/// summary's border changed.
///
/// Only the newest matters, for the same reason a scan's does.
fn drain_sccache(app: &mut App, replies: &Receiver<SccacheSummary>) -> bool {
    let mut latest: Option<SccacheSummary> = None;
    while let Ok(summary) = replies.try_recv() {
        latest = Some(summary);
    }
    latest.is_some_and(|summary| sccache::apply(&mut app.sccache, summary))
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::rc::Rc;

    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use tempfile::TempDir;
    use tui_pane::FrameworkOverlayId;
    use tui_pane::TerminalApp;
    use tui_pane::dispatch_key;

    use super::*;
    use crate::attract::SettingsApplicationOutcome;
    use crate::birth_stamp::IdentityEvidence;
    use crate::birth_stamp::KernelObservation;
    use crate::birth_stamp::Observation;
    use crate::birth_stamp::ProcessLifetime;
    use crate::census::CargoGroup;
    use crate::census::CargoProcess;
    use crate::census::CompilerObservation;
    use crate::census::InvocationId;
    use crate::census::Measurement;
    use crate::census::RowProvenance;
    use crate::census::RunStart;
    use crate::census::SelectedProof;
    use crate::census::VisibleParent;
    use crate::census::command_text::CommandText;
    use crate::census::invocation_cpu_accounting::MeasurementAbsence;
    use crate::census::process_identity::CaptureMembership;
    use crate::census::process_identity::ProcessIdentity;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::TEST_INVOCATION_PID;
    use crate::constants::TEST_REPLACEMENT_LIFETIME;
    use crate::favorites::FavoritesFileState;
    use crate::progress::capture::Capture;
    use crate::progress::capture::CaptureRootIndex;
    use crate::progress::capture_read::CaptureLookup;
    use crate::progress::capture_read::CaptureRead;
    use crate::progress::capture_read::RunState;
    use crate::progress::capture_roots::AccountName;
    use crate::progress::capture_roots::CaptureRoots;
    use crate::registration::WorkingDirectoryIdentity;
    use crate::sccache::SccacheServer;
    use crate::settings::AssociationSelection;
    use crate::settings::CaptureAssociation;

    const FAVORITE_ROW: &str = r#"
[[favorite]]
id = "01a03f60-9c14-7b41-8a02-1de4c7c9b332"
saved = "2026-08-26T11:02:44-07:00"
mode = "moving_band"
direction = "left"
width = 10
speed = 32
tail_speed = 72
fraying = "leading"
"#;

    fn key(code: KeyCode) -> KeyEvent { KeyEvent::new(code, KeyModifiers::NONE) }

    fn rendered_buffer_lines(buffer: &Buffer) -> Vec<String> {
        (buffer.area.y..buffer.area.bottom())
            .map(|y| {
                (buffer.area.x..buffer.area.right()).fold(String::new(), |mut line, x| {
                    line.push_str(buffer[(x, y)].symbol());
                    line
                })
            })
            .collect()
    }

    /// Every recovery here leaves the command grid empty while settings changes.
    fn deliver_capture(app: &mut App, capture: Capture) -> bool {
        let (sender, scans) = mpsc::channel();
        sender
            .send(Scan {
                groups:           Vec::new(),
                sccache:          SccacheServer::Stopped,
                root_status:      capture.root_status,
                shared_directory: capture.shared_directory,
            })
            .expect("retained scan receiver");
        drain_scans(app, &scans)
    }

    /// A measured idle row isolates scan delivery from live process collection.
    fn scan_process(invocation_id: InvocationId, state: CaptureLookup) -> CargoProcess {
        CargoProcess {
            path: "/runner/project".to_owned(),
            directory_identity: WorkingDirectoryIdentity::Absolute("/runner/project".into()),
            pid: 11,
            invocation_id,
            capture_membership: CaptureMembership::Outside,
            provenance: RowProvenance::Uncaptured,
            parent: VisibleParent::None,
            start: "10:00".to_owned(),
            started: RunStart::Known(0),
            duration: "00:01".to_owned(),
            cpu: Measurement::Reading("0%".to_owned()),
            subtree_cpu: Measurement::Reading("0%".to_owned()),
            compiler: CompilerObservation::None,
            state,
            managed: Measurement::Reading(0),
            nested: false,
            command: CommandText::of("cargo", &["test"]),
        }
    }

    /// PID reuse arrives through the channel as a new invocation and a second tile.
    #[test]
    fn reused_pid_scan_delivery_keeps_both_invocations() {
        let mut app = App::new_for_test().expect("test app");
        let mut first = scan_process(
            InvocationId::for_test(TEST_INVOCATION_PID),
            CaptureLookup::Unregistered,
        );
        first.pid = TEST_INVOCATION_PID;
        let replacement = CargoProcess {
            invocation_id: InvocationId::Process(ProcessIdentity::Known {
                pid:      TEST_INVOCATION_PID,
                lifetime: ProcessLifetime::for_test(TEST_REPLACEMENT_LIFETIME),
            }),
            ..first.clone()
        };
        let (sender, scans) = mpsc::channel();
        for process in [&first, &replacement] {
            sender
                .send(Scan {
                    groups:           vec![CargoGroup {
                        lead:     process.clone(),
                        rest:     Vec::new(),
                        ancestry: Vec::new(),
                    }],
                    sccache:          SccacheServer::Stopped,
                    root_status:      Vec::new(),
                    shared_directory: crate::root_scan::SharedCaptureDirectory::default(),
                })
                .expect("scan receiver is alive");
            assert!(drain_scans(&mut app, &scans));
        }
        assert_eq!(app.roster.groups().len(), 2);
        assert!(app.roster.groups()[0].lead.is_ended());
        assert_eq!(app.roster.groups()[1].lead.process, replacement);
        assert_eq!(
            app.roster.tiled_ids(&[]),
            vec![first.invocation_id, replacement.invocation_id]
        );
    }

    /// Availability changes must redraw and reach the roster without a stale value.
    #[test]
    fn unavailable_measurements_replace_readings_through_the_scan_channel() {
        let mut app = App::new_for_test().expect("test app");
        let reading = scan_process(InvocationId::for_test(11), CaptureLookup::Unregistered);
        let unavailable = CargoProcess {
            cpu: Measurement::Unavailable(MeasurementAbsence::ReadFailed),
            compiler: CompilerObservation::Unknown,
            managed: Measurement::Unavailable(MeasurementAbsence::Unproven),
            ..reading.clone()
        };
        let (sender, scans) = mpsc::channel();

        for (process, redraw) in [
            (&reading, true),
            (&unavailable, true),
            (&unavailable, false),
            (&reading, true),
        ] {
            sender
                .send(Scan {
                    groups:           vec![CargoGroup {
                        lead:     process.clone(),
                        rest:     Vec::new(),
                        ancestry: Vec::new(),
                    }],
                    sccache:          SccacheServer::Stopped,
                    root_status:      Vec::new(),
                    shared_directory: crate::root_scan::SharedCaptureDirectory::default(),
                })
                .expect("scan receiver is alive");

            assert_eq!(drain_scans(&mut app, &scans), redraw);
            assert_eq!(&app.roster.groups()[0].lead.process, process);
        }
    }

    #[test]
    fn missing_account_directory_appears_and_redraws_an_unchanged_empty_grid() {
        let mut app = App::new_for_test().expect("test app");
        let parent = TempDir::new().expect("fixture directory");
        let path = parent.path().join("later");
        let roots = CaptureRoots::for_test(&[&path]);
        let observe = |pid| KernelObservation::for_test(pid, Observation::Unknown);
        assert!(deliver_capture(
            &mut app,
            Capture::take_roots(&roots, &observe)
        ));
        assert!(matches!(
            app.root_status[0].state,
            crate::progress::capture_roots::RootReadStatus::Unavailable(_)
        ));
        fs::create_dir_all(path.join(CAPTURE_LIVE_RUNS_DIR)).expect("create account directory");
        assert!(deliver_capture(
            &mut app,
            Capture::take_roots(&roots, &observe)
        ));
        assert_eq!(
            app.root_status[0].state,
            crate::progress::capture_roots::RootReadStatus::Readable
        );
        assert!(app.roster.groups().is_empty());
        // Recovery conservatively disables one sweep; allow that observation to settle.
        deliver_capture(&mut app, Capture::take_roots(&roots, &observe));
        assert!(!deliver_capture(
            &mut app,
            Capture::take_roots(&roots, &observe)
        ));
    }

    #[test]
    fn unreadable_capture_recovers_and_redraws_without_any_process_row() {
        let mut app = App::new_for_test().expect("test app");
        let root = TempDir::new().expect("root fixture");
        let registrations = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        fs::create_dir_all(&registrations).expect("registration directory");
        fs::write(
            registrations.join("10.live"),
            [
                "cargo-tile-v2",
                "live",
                "boot",
                "100",
                "exact.log",
                "/work",
                "/home",
                "0",
                "",
            ]
            .join("\0"),
        )
        .expect("versioned record");
        let log = root
            .path()
            .canonicalize()
            .expect("resolved capture root")
            .join("exact.log");
        std::os::unix::fs::symlink("inaccessible", &log).expect("refused log target");
        let observe = |pid| {
            let observation = match crate::birth_stamp::BirthStamp::from_fields("boot", "100") {
                IdentityEvidence::Available(stamp) => Observation::Present(stamp),
                IdentityEvidence::Unavailable => Observation::Unknown,
            };
            KernelObservation::for_test(pid, observation)
        };
        assert!(deliver_capture(
            &mut app,
            Capture::take_from(root.path(), observe)
        ));
        assert!(app.root_status[0].diagnostics.iter().any(|diagnostic| matches!(diagnostic, crate::progress::capture_diagnostic::CaptureDiagnostic::LogUnreadable(failure) if failure.path == log)));
        assert!(app.roster.groups().is_empty());
        fs::remove_file(&log).expect("remove refused target");
        fs::write(&log, "readable").expect("recover log");
        assert!(deliver_capture(
            &mut app,
            Capture::take_from(root.path(), observe)
        ));
        assert_eq!(app.root_status[0].confirmed, 1);
        assert!(app.root_status[0].diagnostics.is_empty());
        assert!(app.roster.groups().is_empty());
        assert!(!deliver_capture(
            &mut app,
            Capture::take_from(root.path(), observe)
        ));
    }

    #[test]
    fn capture_selection_and_account_changes_redraw_even_without_process_rows() {
        let mut app = App::new_for_test().expect("test app");
        let root = TempDir::new().expect("root");
        let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        fs::create_dir_all(&markers).expect("registration directory");
        for pid in [10, 11] {
            fs::write(markers.join(pid.to_string()), "/work\tcargo build").expect("registration");
        }
        let observe = |pid| KernelObservation::for_test(pid, Observation::Unknown);
        let mut capture = Capture::take_from(root.path(), observe);
        let keys: Vec<_> = [10, 11]
            .into_iter()
            .flat_map(|pid| capture.keys(pid))
            .collect();
        let association = CaptureAssociation {
            pid:       10,
            selection: AssociationSelection::Ambiguous {
                candidates: keys.clone(),
            },
        };
        capture.root_status[0]
            .associations
            .push(association.clone());
        assert!(deliver_capture(&mut app, capture));
        assert_eq!(app.root_status[0].associations, [association]);
        let mut recovered = Capture::take_from(root.path(), observe);
        recovered.root_status[0]
            .associations
            .push(CaptureAssociation {
                pid:       10,
                selection: AssociationSelection::Selected {
                    key:    keys[0].clone(),
                    proof:  SelectedProof::Unconfirmed,
                    unused: Vec::new(),
                },
            });
        recovered.root_status[0].account = AccountName::Resolved("runner".into());
        let expected = recovered.root_status.clone();
        assert!(deliver_capture(&mut app, recovered));
        assert_eq!(app.root_status, expected);
        assert!(app.roster.groups().is_empty());
        let mut unchanged = Capture::take_from(root.path(), observe);
        unchanged.root_status = expected;
        assert!(!deliver_capture(&mut app, unchanged));
    }

    #[test]
    fn an_account_capture_reaches_the_roster_through_the_scan_channel() {
        let mut app = App::new_for_test().expect("test app should build");
        let directory = TempDir::new().expect("temporary capture root");
        let markers = directory.path().join(CAPTURE_LIVE_RUNS_DIR);
        fs::create_dir_all(&markers).expect("registration directory");
        fs::write(markers.join("10"), "/runner/project\tcargo test").expect("legacy registration");
        fs::write(
            directory.path().join("run-generation-10.log"),
            "    Blocking waiting for file lock on build directory",
        )
        .expect("captured output");
        let roots = CaptureRoots::for_test(&[directory.path()]);
        let capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        let key = capture
            .keys(10)
            .find(|key| key.root == CaptureRootIndex(0))
            .expect("account directory retains its full capture key");
        let state = capture.read(&key);
        assert_eq!(
            state,
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
        );
        let process = scan_process(InvocationId::for_test(11), state);
        let (sender, scans) = mpsc::channel();
        sender
            .send(Scan {
                groups:           vec![CargoGroup {
                    lead:     process.clone(),
                    rest:     Vec::new(),
                    ancestry: Vec::new(),
                }],
                sccache:          SccacheServer::Stopped,
                root_status:      capture.root_status,
                shared_directory: capture.shared_directory,
            })
            .expect("scan receiver is alive");

        assert!(drain_scans(&mut app, &scans));
        let received = app
            .roster
            .groups()
            .first()
            .expect("received group")
            .rows()
            .next()
            .expect("received process");
        assert_eq!(received.process, process);
        assert!(!drain_scans(&mut app, &scans));
    }

    #[test]
    fn app_modal_consumes_app_and_framework_globals_until_escape() {
        let mut app = App::new_for_test().expect("test app should build");
        dispatch_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
        );
        assert!(app.favorites_overlay.is_open());

        dispatch_key(&mut app, key(KeyCode::Char('f')));
        dispatch_key(&mut app, key(KeyCode::Char('?')));
        assert_eq!(app.updates, Updates::Live);
        assert_eq!(app.framework.overlay(), None);
        assert!(app.favorites_overlay.is_open());

        dispatch_key(&mut app, key(KeyCode::Esc));
        assert!(!app.favorites_overlay.is_open());
    }

    #[test]
    fn unmapped_modal_key_cancels_delete_confirmation_without_writing() {
        let mut app = App::new_for_test().expect("test app should build");
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = directory.path().join("favorites.toml");
        fs::write(&path, FAVORITE_ROW).expect("favorite fixture should be written");
        let original = fs::read(&path).expect("favorite fixture should be readable");
        let rows = favorites::parse_rows_for_overlay_test(FAVORITE_ROW)
            .expect("favorite fixture should parse");
        let current_parameters = app.attract.current_settings().into();
        let keymap = Rc::clone(&app.keymap);
        app.favorites_overlay.open_file_state(
            FavoritesFileState::Loaded {
                path: path.clone(),
                rows,
            },
            current_parameters,
            &keymap,
        );
        let mut terminal =
            Terminal::new(TestBackend::new(100, 30)).expect("test terminal should be created");
        terminal
            .draw(|frame| app.favorites_overlay.render(frame))
            .expect("favorites overlay should render");

        dispatch_key(&mut app, key(KeyCode::Char('x')));
        assert!(
            app.favorites_overlay
                .deletion_confirmation_is_armed_for_test()
        );
        assert!(
            app.favorites_overlay
                .deletion_confirmation_notice_is_visible_for_test()
        );
        dispatch_key(&mut app, key(KeyCode::Char('z')));

        assert!(
            !app.favorites_overlay
                .deletion_confirmation_is_armed_for_test()
        );
        assert!(
            !app.favorites_overlay
                .deletion_confirmation_notice_is_visible_for_test()
        );
        assert_eq!(
            fs::read(&path).expect("favorite fixture should remain readable"),
            original
        );
    }

    #[test]
    fn coalesced_resize_refreshes_currency_after_attract_reclamping() {
        let mut app = App::new_for_test().expect("test app should build");
        let rows = favorites::parse_rows_for_overlay_test(FAVORITE_ROW)
            .expect("favorite fixture should parse");
        let favorite_settings = rows
            .recognized()
            .next()
            .expect("favorite fixture should have a recognized row")
            .settings;
        app.attract.record_terminal_resize(Rect::new(0, 0, 80, 24));
        assert_eq!(
            app.attract.apply_settings(favorite_settings),
            SettingsApplicationOutcome::AppliedExactly
        );
        let current_parameters = app.attract.current_settings().into();
        let keymap = Rc::clone(&app.keymap);
        app.favorites_overlay.open_file_state(
            FavoritesFileState::Loaded {
                path: PathBuf::from("/tmp/favorites.toml"),
                rows,
            },
            current_parameters,
            &keymap,
        );
        let mut terminal =
            Terminal::new(TestBackend::new(100, 30)).expect("test terminal should be created");
        terminal
            .draw(|frame| app.favorites_overlay.render(frame))
            .expect("favorites overlay should render");
        let initial_rows = rendered_buffer_lines(terminal.backend().buffer());
        assert!(initial_rows.iter().any(|line| line.contains("▸● ")));

        app.resized(Rect::new(0, 0, 5, 4));
        terminal
            .draw(|frame| app.favorites_overlay.render(frame))
            .expect("favorites overlay should render before resize refresh");
        let before_refresh = rendered_buffer_lines(terminal.backend().buffer());
        assert!(before_refresh.iter().any(|line| line.contains("▸● ")));

        app.resize_settled();
        assert_ne!(app.attract.current_settings(), favorite_settings);
        terminal
            .draw(|frame| app.favorites_overlay.render(frame))
            .expect("favorites overlay should render after resize refresh");
        let refreshed_rows = rendered_buffer_lines(terminal.backend().buffer());
        assert!(refreshed_rows.iter().any(|line| line.contains("▸  ")));
    }

    #[test]
    fn x_leaves_each_framework_overlay_open_while_escape_closes_it() {
        let mut app = App::new_for_test().expect("test app should build");
        for (overlay, opener) in [
            (FrameworkOverlayId::Settings, key(KeyCode::Char('s'))),
            (
                FrameworkOverlayId::Keymap,
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
            ),
            (FrameworkOverlayId::GlobalShortcuts, key(KeyCode::Char('?'))),
        ] {
            dispatch_key(&mut app, opener);
            assert_eq!(app.framework.overlay(), Some(overlay));
            dispatch_key(&mut app, key(KeyCode::Char('x')));
            assert_eq!(app.framework.overlay(), Some(overlay));
            dispatch_key(&mut app, key(KeyCode::Esc));
            assert_eq!(app.framework.overlay(), None);
        }
    }

    #[test]
    fn framework_modal_prevents_a_second_app_modal_from_opening() {
        let mut app = App::new_for_test().expect("test app should build");
        dispatch_key(&mut app, key(KeyCode::Char('s')));
        assert_eq!(app.framework.overlay(), Some(FrameworkOverlayId::Settings));

        dispatch_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.framework.overlay(), Some(FrameworkOverlayId::Settings));
        assert!(!app.favorites_overlay.is_open());
    }
}
