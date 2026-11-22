//! Starting the grid: load its configuration, build the app, and run it
//! under [`tui_pane::run_terminal`] with appearance changes, scans and
//! sccache reads as the work the loop polls.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::time::Instant;

use tui_pane::Appearance;
use tui_pane::PollWork;
use tui_pane::Repaint;
use tui_pane::install_theme;
use tui_pane::run_terminal;

use crate::app::App;
use crate::app::Updates;
use crate::capture;
use crate::census;
use crate::census::CargoGroup;
use crate::census::CensusCadence;
use crate::census::CensusScope;
use crate::census::ExcludedCommands;
#[cfg(test)]
use crate::census::cpu_observation::CpuScanPublication;
use crate::census::invocation_cpu_accounting::CompileMemoryShares;
use crate::census::scan::FreeDiskSpace;
#[cfg(test)]
use crate::census::slot_lease::SlotObservation;
use crate::config::CargoTile;
use crate::config::LoadedConfig;
use crate::constants::CAPTURE_ROOT;
use crate::progress::capture_roots::AccountCaptureDirectory;
use crate::render::SummaryDiskSpace;
use crate::root_scan::SharedCaptureDirectory;
use crate::roster::MemorySharesChange;
use crate::roster::RosterScanTime;
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
    /// The native scan that supplied this test snapshot.
    #[cfg(test)]
    pub(crate) cpu_scan_publication: CpuScanPublication,
    /// The commands running, newest first.
    pub(crate) groups:               Vec<CargoGroup>,
    /// Compiler memory published beside the cargo rows.
    pub(crate) memory_shares:        CompileMemoryShares,
    /// Whether a process named [`crate::constants::SCCACHE_BINARY`] was among them.
    pub(crate) sccache:              SccacheServer,
    /// Available disk bytes read with this census.
    pub(crate) free_disk_space:      FreeDiskSpace,
    /// Settings reads these observations without reopening any capture path.
    pub(crate) root_status:          Vec<AccountCaptureDirectory>,
    pub(crate) shared_directory:     SharedCaptureDirectory,
}

/// Whether tile positions travel between layouts or reach the new layout on the next poll.
#[derive(Clone, Copy)]
pub(crate) enum GridMotion {
    Animated,
    #[cfg(test)]
    Immediate,
}

impl GridMotion {
    #[cfg(not(test))]
    const fn settle_if_immediate(self, _: &mut App) {
        match self {
            Self::Animated => {},
        }
    }

    #[cfg(test)]
    fn settle_if_immediate(self, app: &mut App) {
        match self {
            Self::Animated => {},
            Self::Immediate => app.tiles.settle_for_test(),
        }
    }
}

/// Load configuration, install the theme, build the keymap, and run the
/// event loop with the terminal in the alternate screen.
pub(crate) fn run() -> ExitCode {
    run_with_capture_parent(
        PathBuf::from(CAPTURE_ROOT),
        CensusCadence::default(),
        CensusScope::default(),
        GridMotion::Animated,
        || {},
    )
}

/// The executable supplies the fixed parent, the production cadence and
/// every cargo on the machine; PTY tests supply an isolated path, a shorter
/// cadence and only their own processes.
/// `before_first_scan` lets a test start the reader during fixture setup.
pub(crate) fn run_with_capture_parent(
    parent: PathBuf,
    cadence: CensusCadence,
    scope: CensusScope,
    grid_motion: GridMotion,
    before_first_scan: impl FnOnce(),
) -> ExitCode {
    run_with_scanner(
        move |excluded| {
            before_first_scan();
            census::spawn_with_resolver(excluded, cadence, scope, move || {
                crate::progress::capture_roots::CaptureRoots::from_parent(&parent)
            })
            .0
        },
        grid_motion,
    )
}

fn run_with_scanner(
    spawn: impl FnOnce(ExcludedCommands) -> Receiver<Scan>,
    grid_motion: GridMotion,
) -> ExitCode {
    let loaded_config = LoadedConfig::load::<CargoTile>();
    let mut startup_note = install_theme(
        &loaded_config.config.appearance,
        theme::builtins::builtins(),
    );
    let (appearance_changes, appearances) = mpsc::channel();
    let startup_appearance = tui_pane::spawn_appearance_watcher(move |appearance| {
        let _ = appearance_changes.send(appearance);
    });
    if let Some(appearance) = startup_appearance {
        startup_note =
            tui_pane::apply_system_appearance(appearance, &loaded_config.config.appearance);
    }
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
        Workers::new(
            spawn(app.excluded_commands.clone()),
            appearances,
            grid_motion,
        )
    })
}

/// What the loop folds in on every pass besides input: the process
/// scans, the sccache reads, the favorites overlay's removal fade, the
/// roster's fades, the grid's motion and the attract screen's frames.
struct Workers {
    /// System appearance changes waiting to be applied on the terminal thread.
    appearances:     Receiver<Appearance>,
    /// Process scans from the census worker.
    scans:           Receiver<Scan>,
    /// Each due read runs on a worker of its own and replies here, so a
    /// server that has wedged parks that one thread rather than the loop.
    sccache_reads:   Sender<SccacheSummary>,
    /// Where those reads reply.
    sccache_replies: Receiver<SccacheSummary>,
    /// PTY regressions settle motion so their assertions wait on data, not presentation time.
    grid_motion:     GridMotion,
}

impl Workers {
    /// Hold the watcher channels, with a fresh channel for the sccache reads.
    fn new(
        scans: Receiver<Scan>,
        appearances: Receiver<Appearance>,
        grid_motion: GridMotion,
    ) -> Self {
        let (sccache_reads, sccache_replies) = mpsc::channel();
        Self {
            appearances,
            scans,
            sccache_reads,
            sccache_replies,
            grid_motion,
        }
    }
}

impl PollWork<App> for Workers {
    fn poll(&mut self, app: &mut App, now: Instant) -> Repaint {
        #[cfg(feature = "attract")]
        let mut dirty = tui_pane::poll_favorites(app, now) == Repaint::Needed;
        #[cfg(not(feature = "attract"))]
        let mut dirty = false;
        dirty |= drain_appearances(app, &self.appearances);
        // Discard scans while paused. The first accepted scan after resume is current.
        if app.updates == Updates::Paused {
            discard_scans(&self.scans, &self.sccache_replies);
        } else {
            let display_time = app.sample_display_time(now);
            let scan_time = RosterScanTime {
                observed: now,
                display:  display_time,
            };
            if drain_scans(app, &self.scans, scan_time) {
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
            sccache::refresh_if_due(&mut app.sccache, &self.sccache_reads, now);
            // A finished row walks its grey toward the ground it is
            // drawn on for the configured spell and then goes, taking
            // its cell with it. Nothing external announces either the
            // steps or the moment, so the poll is what carries them.
            if app
                .roster
                .advance(display_time, app.loaded_config.config.tiles.fade())
            {
                dirty = true;
            }
            // Advance motion even when two polls resolve to equal geometry.
            // Only a change from the last body paint requests another frame.
            dirty |= app.grid_view.advance(display_time) == Repaint::Needed;
            self.grid_motion.settle_if_immediate(app);
            if app.tiles.tick() {
                dirty = true;
            }
            // The attract screen is the other: it asks for frames on its
            // own cadence while it is on the screen, and for one more at
            // the end of the quiet it waits out before coming back. See
            // [`tui_pane::Attract::frame_due`].
            #[cfg(feature = "attract")]
            if app.attract.frame_due() == Repaint::Needed {
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

/// Apply the newest queued system appearance and request a frame for it.
fn drain_appearances(app: &mut App, appearances: &Receiver<Appearance>) -> bool {
    let mut latest = None;
    while let Ok(appearance) = appearances.try_recv() {
        latest = Some(appearance);
    }
    let Some(appearance) = latest else {
        return false;
    };
    app.apply_system_appearance(appearance);
    true
}

/// Empty the worker channels while keeping the displayed observations.
fn discard_scans(scans: &Receiver<Scan>, sccache_replies: &Receiver<SccacheSummary>) {
    while scans.try_recv().is_ok() {}
    while sccache_replies.try_recv().is_ok() {}
}

/// Take the newest process scan, reporting whether it changed anything.
///
/// Only the newest matters: an older scan queued behind it describes a
/// world that has already moved on.
fn drain_scans(app: &mut App, scans: &Receiver<Scan>, scan_time: RosterScanTime) -> bool {
    let mut latest: Option<Scan> = None;
    while let Ok(scan) = scans.try_recv() {
        latest = Some(scan);
    }
    let Some(scan) = latest else {
        return false;
    };
    app.sccache.observe_server(scan.sccache);
    let memory_changed = app.roster.observe_memory_shares(scan.memory_shares);
    let disk_changed =
        SummaryDiskSpace::from(app.free_disk_space) != SummaryDiskSpace::from(scan.free_disk_space);
    app.free_disk_space = scan.free_disk_space;
    let status_changed =
        app.root_status != scan.root_status || app.shared_directory != scan.shared_directory;
    app.shared_directory = scan.shared_directory;
    app.root_status = scan.root_status;
    #[cfg(test)]
    census::cpu_observation::accept(scan.cpu_scan_publication);
    app.roster.observe_at(scan.groups, scan_time)
        || status_changed
        || disk_changed
        || memory_changed == MemorySharesChange::Changed
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
    use std::time::Duration;

    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::buffer::Cell;
    use ratatui::layout::Rect;
    use tempfile::TempDir;
    #[cfg(feature = "attract")]
    use tui_pane::ColorMode;
    #[cfg(feature = "attract")]
    use tui_pane::FavoritesFileState;
    use tui_pane::FrameworkOverlayId;
    #[cfg(feature = "attract")]
    use tui_pane::SettingsApplicationOutcome;
    use tui_pane::TerminalApp;
    use tui_pane::TileAction;
    use tui_pane::VisualDeadline;
    use tui_pane::dispatch_key;

    use super::*;
    use crate::app::GridView;
    use crate::birth_stamp::IdentityEvidence;
    use crate::birth_stamp::KernelObservation;
    use crate::birth_stamp::Observation;
    use crate::birth_stamp::ProcessLifetime;
    use crate::census::CargoGroup;
    use crate::census::CargoProcess;
    use crate::census::CompilerObservation;
    use crate::census::InvocationId;
    use crate::census::Measurement;
    use crate::census::ProcessOwner;
    use crate::census::RowProvenance;
    use crate::census::SelectedProof;
    use crate::census::VisibleParent;
    use crate::census::command_text::CommandText;
    use crate::census::cpu_observation::CpuScanPublication;
    use crate::census::invocation_cpu_accounting::MeasurementAbsence;
    use crate::census::process_identity::CaptureMembership;
    use crate::census::process_identity::ProcessIdentity;
    use crate::census::scan::AncestryWalk;
    use crate::census::scan::InvocationExecution;
    use crate::census::scan::InvocationResourceUse;
    use crate::census::scan::OriginStart;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::TEST_INVOCATION_PID;
    use crate::constants::TEST_REPLACEMENT_LIFETIME;
    use crate::progress::capture::Capture;
    use crate::progress::capture::CaptureRootIndex;
    use crate::progress::capture_diagnostic::CaptureDiagnostic;
    use crate::progress::capture_read::CaptureLookup;
    use crate::progress::capture_read::CaptureRead;
    use crate::progress::capture_read::RunState;
    use crate::progress::capture_roots::AccountName;
    use crate::progress::capture_roots::CaptureRoots;
    use crate::registration::WorkingDirectoryIdentity;
    use crate::sccache::SccacheServer;
    use crate::settings::AssociationSelection;
    use crate::settings::CaptureAssociation;

    #[test]
    fn idle_visual_tile_draws_an_appearance_change_without_a_scan() {
        let mut app = App::new_for_test().expect("test app");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).expect("terminal");
        let keymap = std::rc::Rc::clone(&app.keymap);
        for _ in 0..2 {
            terminal
                .draw(|frame| app.draw(frame, &keymap))
                .expect("frame");
            app.tiles.settle_for_test();
        }
        let mut workers = Workers::new(mpsc::channel().1, mpsc::channel().1, GridMotion::Animated);
        let tint = tui_pane::focused_pane_tint_enabled();
        tui_pane::set_focused_pane_tint(!tint);
        let changed = workers.poll(&mut app, Instant::now());
        tui_pane::set_focused_pane_tint(tint);
        assert_eq!(changed, Repaint::Needed, "appearance needs a body frame");
    }

    #[cfg(feature = "attract")]
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

    #[cfg(feature = "attract")]
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
                memory_shares:        CompileMemoryShares::default(),
                free_disk_space:      FreeDiskSpace::Unavailable,
                cpu_scan_publication: CpuScanPublication::Unobserved,
                groups:               Vec::new(),
                sccache:              SccacheServer::Stopped,
                root_status:          capture.root_status,
                shared_directory:     capture.shared_directory,
            })
            .expect("retained scan receiver");
        drain_scans(app, &scans, RosterScanTime::live(Instant::now()))
    }

    /// A measured idle row isolates scan delivery from live process collection.
    fn scan_process(invocation_id: InvocationId, state: CaptureLookup) -> CargoProcess {
        CargoProcess {
            slot: SlotObservation::default(),
            path: "/runner/project".to_owned(),
            directory_identity: WorkingDirectoryIdentity::Absolute("/runner/project".into()),
            pid: 11,
            invocation_id,
            execution: InvocationExecution::Command,
            capture_membership: CaptureMembership::Outside,
            provenance: RowProvenance::Uncaptured(ProcessOwner::Unavailable),
            parent: VisibleParent::None,
            origin_start: OriginStart::Process {
                epoch:   0,
                elapsed: std::time::Duration::from_secs(1),
            },
            resource_use: InvocationResourceUse {
                cpu:      Measurement::Reading(0.0),
                memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
                compiler: CompilerObservation::None,
            },
            subtree_resource_use: InvocationResourceUse {
                cpu:      Measurement::Reading(0.0),
                memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
                compiler: CompilerObservation::None,
            },
            state,
            managed: Measurement::Reading(0),
            nested: false,
            command: CommandText::of("cargo", &["test"]),
        }
    }

    #[test]
    fn idle_visual_tile_skips_equal_polls_but_keeps_content_fades() {
        let mut app = App::new_for_test().expect("test app");
        let process = scan_process(
            InvocationId::for_test(TEST_INVOCATION_PID),
            CaptureLookup::Unregistered,
        );
        let now = Instant::now();
        app.roster.observe(
            vec![CargoGroup {
                lead:     process,
                rest:     Vec::new(),
                ancestry: crate::census::scan::AncestryWalk::Ended(Vec::new()),
            }],
            now,
        );
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
        let keymap = std::rc::Rc::clone(&app.keymap);
        for _ in 0..2 {
            terminal
                .draw(|frame| app.draw(frame, &keymap))
                .expect("body paint");
            app.tiles.settle_for_test();
        }
        let mut workers = Workers::new(mpsc::channel().1, mpsc::channel().1, GridMotion::Animated);
        let mut frames = 0;
        for _ in 0..32 {
            frames += usize::from(workers.poll(&mut app, now) == Repaint::Needed);
        }
        assert_eq!(frames, 0, "unchanged content and geometry skip frames");
        app.roster.observe(Vec::new(), now);
        let mut fades = 0;
        let fade = app.loaded_config.config.tiles.fade();
        for step in 1..=3 {
            fades += usize::from(workers.poll(&mut app, now + fade * step / 4) == Repaint::Needed);
            terminal
                .draw(|frame| app.draw(frame, &keymap))
                .expect("fade paint");
            app.tiles.settle_for_test();
        }
        assert_eq!(
            fades, 3,
            "settled geometry must not suppress fading content"
        );
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
                    memory_shares:        CompileMemoryShares::default(),
                    free_disk_space:      FreeDiskSpace::Unavailable,
                    cpu_scan_publication: CpuScanPublication::Unobserved,
                    groups:               vec![CargoGroup {
                        lead:     process.clone(),
                        rest:     Vec::new(),
                        ancestry: crate::census::scan::AncestryWalk::Ended(Vec::new()),
                    }],
                    sccache:              SccacheServer::Stopped,
                    root_status:          Vec::new(),
                    shared_directory:     crate::root_scan::SharedCaptureDirectory::default(),
                })
                .expect("scan receiver is alive");
            assert!(drain_scans(
                &mut app,
                &scans,
                RosterScanTime::live(Instant::now())
            ));
        }
        assert_eq!(app.roster.groups().len(), 2);
        assert!(app.roster.groups()[0].lead.is_ended());
        assert_eq!(app.roster.groups()[1].lead.process, replacement);
        assert_eq!(
            app.roster.tiled_ids(&[]),
            vec![first.invocation_id, replacement.invocation_id]
        );
    }

    #[test]
    fn space_pauses_and_unpauses_the_display() {
        let mut app = App::new_for_test().expect("test app");
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        let (sender, scans) = mpsc::channel();
        let mut workers = Workers::new(scans, mpsc::channel().1, GridMotion::Animated);
        let keymap = std::rc::Rc::clone(&app.keymap);
        let mut status = |app: &mut App| {
            terminal
                .draw(|frame| app.draw(frame, &keymap))
                .expect("frame");
            let buffer = terminal.backend().buffer();
            (0..buffer.area.width)
                .map(|x| buffer[(x, buffer.area.bottom() - 1)].symbol())
                .collect::<String>()
        };
        let send_disk = |app: &App, free_disk_space| {
            sender
                .send(Scan {
                    memory_shares: CompileMemoryShares::default(),
                    free_disk_space,
                    cpu_scan_publication: CpuScanPublication::Unobserved,
                    groups: Vec::new(),
                    sccache: SccacheServer::Stopped,
                    root_status: app.root_status.clone(),
                    shared_directory: app.shared_directory.clone(),
                })
                .expect("scan receiver is alive");
        };

        app.free_disk_space = FreeDiskSpace::Unavailable;
        assert!(!status(&mut app).contains("paused"));
        dispatch_key(&mut app, key(KeyCode::Char(' ')));
        assert!(status(&mut app).contains("paused"));
        send_disk(&app, FreeDiskSpace::Available(1 << 30));
        workers.poll(&mut app, Instant::now());
        assert_eq!(app.free_disk_space, FreeDiskSpace::Unavailable);

        dispatch_key(&mut app, key(KeyCode::Char(' ')));
        assert!(!status(&mut app).contains("paused"));
        workers.poll(&mut app, Instant::now());
        assert_eq!(app.free_disk_space, FreeDiskSpace::Unavailable);
        let current = FreeDiskSpace::Available(2 << 30);
        send_disk(&app, current);
        workers.poll(&mut app, Instant::now());
        assert_eq!(app.free_disk_space, current);
    }

    #[derive(Clone, Copy, Debug)]
    enum PauseView {
        Grid,
        Opening,
        Expanding,
        Expanded,
        Collapsing,
    }

    struct PauseFixture {
        app:      App,
        workers:  Workers,
        scans:    Sender<Scan>,
        terminal: Terminal<TestBackend>,
        group:    CargoGroup,
    }

    impl PauseFixture {
        fn new() -> Self {
            let app = App::new_for_test().expect("test app");
            let terminal = Terminal::new(TestBackend::new(160, 36)).expect("terminal");
            let (scans, incoming) = mpsc::channel();
            let workers = Workers::new(incoming, mpsc::channel().1, GridMotion::Animated);
            let lead = scan_process(InvocationId::for_test(11), CaptureLookup::Unregistered);
            let group = CargoGroup {
                lead,
                rest: Vec::new(),
                ancestry: AncestryWalk::Ended(Vec::new()),
            };
            Self {
                app,
                workers,
                scans,
                terminal,
                group,
            }
        }

        fn send(&self, group: CargoGroup, free_disk_space: FreeDiskSpace) {
            self.scans
                .send(Scan {
                    memory_shares: CompileMemoryShares::default(),
                    free_disk_space,
                    cpu_scan_publication: CpuScanPublication::Unobserved,
                    groups: vec![group],
                    sccache: SccacheServer::Stopped,
                    root_status: self.app.root_status.clone(),
                    shared_directory: self.app.shared_directory.clone(),
                })
                .expect("scan receiver is alive");
        }

        fn poll(&mut self, now: Instant) { self.workers.poll(&mut self.app, now); }

        fn status_tick(&mut self, earliest: Instant) -> Result<Instant, &'static str> {
            let VisualDeadline::At(deadline) = self.app.framework.status_visual_deadline() else {
                return Err("status deadline was not scheduled");
            };
            let now = deadline.max(earliest);
            assert_eq!(self.app.framework.tick_status(now), Repaint::Needed);
            self.poll(now);
            Ok(now)
        }

        fn draw(&mut self) -> Buffer {
            let keymap = self.app.keymap();
            self.terminal
                .draw(|frame| self.app.draw(frame, &keymap))
                .expect("frame");
            self.terminal.backend().buffer().clone()
        }

        fn fading_amount(&self) -> u8 {
            self.app
                .roster
                .groups()
                .iter()
                .flat_map(crate::roster::TrackedGroup::rows)
                .find(|row| row.is_ended())
                .expect("the fixture has a fading row")
                .faded()
        }

        fn positions(&self) -> Vec<Rect> {
            if let Some(cell) = self.app.grid_view.cell() {
                return vec![cell.displayed()];
            }
            let area = self.terminal.backend().buffer().area;
            self.app
                .tiles
                .placements(
                    Rect {
                        height: area.height - 1,
                        ..area
                    },
                    self.app.loaded_config.config.tiles.growth(),
                )
                .iter()
                .map(|placement| placement.frame.clip())
                .collect()
        }

        fn place_view(&mut self, view: PauseView, started: Instant) -> Instant {
            let travel = self.app.tiles.cell_travel_duration();
            match view {
                PauseView::Grid => {
                    let growth = self.app.loaded_config.config.tiles.growth();
                    self.app.tiles.apply(TileAction::Add, growth);
                    assert!(self.app.tiles.has_pending_motion_for_test());
                    started + travel / 2
                },
                PauseView::Opening => {
                    self.app.grid_view.expand(started);
                    assert!(matches!(self.app.grid_view, GridView::Opening { .. }));
                    started
                },
                PauseView::Expanding => {
                    self.app.grid_view.expand(started);
                    self.draw();
                    assert!(matches!(self.app.grid_view, GridView::Expanding { .. }));
                    started + travel / 2
                },
                PauseView::Expanded | PauseView::Collapsing => {
                    self.app.grid_view.expand(started);
                    self.draw();
                    let expanded = started + travel;
                    self.poll(expanded);
                    self.draw();
                    assert!(matches!(self.app.grid_view, GridView::Expanded { .. }));
                    if matches!(view, PauseView::Collapsing) {
                        self.app.grid_view.collapse(expanded);
                        assert!(matches!(self.app.grid_view, GridView::Collapsing { .. }));
                        expanded + travel / 2
                    } else {
                        expanded
                    }
                },
            }
        }
    }

    fn body(buffer: &Buffer) -> &[Cell] {
        &buffer.content[..usize::from(buffer.area.width) * usize::from(buffer.area.height - 1)]
    }

    fn body_text(buffer: &Buffer) -> String {
        body(buffer)
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn a_command_that_runs_through_pause_shows_its_full_elapsed_time() -> Result<(), &'static str> {
        let mut fixture = PauseFixture::new();
        let mut group = fixture.group.clone();
        group.lead.execution = InvocationExecution::RegistrationOnly;
        group.lead.origin_start = OriginStart::Registration {
            epoch:   100,
            elapsed: Duration::from_secs(7),
        };
        let pause_at = fixture.app.started + Duration::from_millis(10);
        fixture.send(group.clone(), FreeDiskSpace::Unavailable);
        fixture.poll(pause_at);
        fixture.draw();
        fixture.app.tiles.settle_for_test();
        assert!(body_text(&fixture.draw()).contains("00:07"));
        fixture.app.toggle_display_pause(pause_at);

        group.lead.origin_start = OriginStart::Unavailable;
        fixture.send(group.clone(), FreeDiskSpace::Unavailable);
        fixture.status_tick(pause_at + Duration::from_secs(1))?;
        assert!(body_text(&fixture.draw()).contains("00:07"));

        let resume_at = pause_at + Duration::from_secs(30);
        fixture.app.toggle_display_pause(resume_at);
        fixture.send(group, FreeDiskSpace::Unavailable);
        fixture.poll(resume_at);
        assert_eq!(
            fixture.app.roster.groups()[0].lead.process.duration(),
            "00:37"
        );
        assert!(body_text(&fixture.draw()).contains("00:37"));
        Ok(())
    }

    #[test]
    fn a_paused_grid_draws_the_same_cells_across_status_ticks() -> Result<(), &'static str> {
        for view in [
            PauseView::Grid,
            PauseView::Opening,
            PauseView::Expanding,
            PauseView::Expanded,
            PauseView::Collapsing,
        ] {
            let mut fixture = PauseFixture::new();
            let disk = FreeDiskSpace::Available(1 << 30);
            let mut live = fixture.group.clone();
            let mut child = scan_process(InvocationId::for_test(12), CaptureLookup::Unregistered);
            child.pid = 12;
            child.parent = VisibleParent::Invocation {
                id:  live.lead.invocation_id.clone(),
                pid: live.lead.pid,
            };
            live.rest.push(child);
            let initial = fixture.app.started + Duration::from_millis(10);
            fixture.send(live, disk);
            fixture.poll(initial);
            for _ in 0..3 {
                fixture.draw();
                fixture.app.tiles.settle_for_test();
            }
            let started = initial + Duration::from_millis(10);
            fixture.send(fixture.group.clone(), disk);
            fixture.poll(started);
            let held_at = fixture.place_view(view, started);
            fixture.poll(held_at);
            let before = if matches!(view, PauseView::Opening) {
                None
            } else {
                Some(fixture.draw())
            };
            fixture.app.toggle_display_pause(held_at);
            let held = fixture.draw();
            let held_fade = fixture.fading_amount();
            let positions = fixture.positions();
            if let Some(before) = before {
                assert_eq!(body(&held), body(&before), "{view:?} changed on pause");
            }
            let mut resumed_at = held_at;
            for queued in [
                FreeDiskSpace::Available(2 << 30),
                FreeDiskSpace::Unavailable,
            ] {
                fixture.send(fixture.group.clone(), queued);
                resumed_at = fixture.status_tick(resumed_at + Duration::from_secs(1))?;
                assert_eq!(fixture.app.free_disk_space, disk);
                assert_eq!(
                    body(&fixture.draw()),
                    body(&held),
                    "{view:?} changed while paused"
                );
            }

            fixture.app.toggle_display_pause(resumed_at);
            fixture.poll(resumed_at);
            assert_eq!(
                body(&fixture.draw()),
                body(&held),
                "{view:?} jumped on resume"
            );
            let moved_at = resumed_at + fixture.app.loaded_config.config.tiles.fade() / 3;
            fixture.poll(moved_at);
            fixture.draw();
            assert_eq!(fixture.app.free_disk_space, disk);
            assert!(
                fixture.fading_amount() > held_fade,
                "{view:?} fade did not resume"
            );
            if !matches!(view, PauseView::Expanded) {
                assert_ne!(
                    fixture.positions(),
                    positions,
                    "{view:?} motion did not resume"
                );
            }

            let next_disk = FreeDiskSpace::Available(3 << 30);
            fixture.send(fixture.group.clone(), next_disk);
            fixture.poll(moved_at);
            assert_eq!(fixture.app.free_disk_space, next_disk);
            let current = fixture.draw();
            if matches!(view, PauseView::Grid | PauseView::Expanded) {
                assert!(
                    body_text(&current).contains("disk 3.0G"),
                    "{view:?} kept the old footer"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn disk_scans_mark_changes_only_when_the_shown_value_changes() {
        let mut app = App::new_for_test().expect("test app");
        app.free_disk_space = FreeDiskSpace::Unavailable;
        let (sender, scans) = mpsc::channel();
        let gibibyte = 1_u64 << 30;

        for (free_disk_space, changed) in [
            (FreeDiskSpace::Available(gibibyte), true),
            (FreeDiskSpace::Available(gibibyte + 1), false),
            (FreeDiskSpace::Available(2 * gibibyte), true),
            (FreeDiskSpace::Unavailable, true),
            (FreeDiskSpace::Unavailable, false),
        ] {
            sender
                .send(Scan {
                    memory_shares: CompileMemoryShares::default(),
                    free_disk_space,
                    cpu_scan_publication: CpuScanPublication::Unobserved,
                    groups: Vec::new(),
                    sccache: SccacheServer::Stopped,
                    root_status: app.root_status.clone(),
                    shared_directory: app.shared_directory.clone(),
                })
                .expect("scan receiver is alive");

            assert_eq!(
                drain_scans(&mut app, &scans, RosterScanTime::live(Instant::now())),
                changed
            );
            assert_eq!(app.free_disk_space, free_disk_space);
        }
    }

    /// Availability changes must redraw and reach the roster without a stale value.
    #[test]
    fn unavailable_measurements_replace_readings_through_the_scan_channel() {
        let mut app = App::new_for_test().expect("test app");
        let reading = scan_process(InvocationId::for_test(11), CaptureLookup::Unregistered);
        let unavailable = CargoProcess {
            resource_use: InvocationResourceUse {
                cpu:      Measurement::Unavailable(MeasurementAbsence::ReadFailed),
                memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
                compiler: CompilerObservation::Unknown,
            },
            subtree_resource_use: InvocationResourceUse {
                cpu:      Measurement::Unavailable(MeasurementAbsence::ReadFailed),
                memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
                compiler: CompilerObservation::Unknown,
            },
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
                    memory_shares:        CompileMemoryShares::default(),
                    free_disk_space:      FreeDiskSpace::Unavailable,
                    cpu_scan_publication: CpuScanPublication::Unobserved,
                    groups:               vec![CargoGroup {
                        lead:     process.clone(),
                        rest:     Vec::new(),
                        ancestry: crate::census::scan::AncestryWalk::Ended(Vec::new()),
                    }],
                    sccache:              SccacheServer::Stopped,
                    root_status:          Vec::new(),
                    shared_directory:     crate::root_scan::SharedCaptureDirectory::default(),
                })
                .expect("scan receiver is alive");

            assert_eq!(
                drain_scans(&mut app, &scans, RosterScanTime::live(Instant::now())),
                redraw
            );
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
        assert_eq!(app.root_status[0].diagnostics, [] as [CaptureDiagnostic; 0]);
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
                memory_shares:        CompileMemoryShares::default(),
                free_disk_space:      FreeDiskSpace::Unavailable,
                cpu_scan_publication: CpuScanPublication::Unobserved,
                groups:               vec![CargoGroup {
                    lead:     process.clone(),
                    rest:     Vec::new(),
                    ancestry: crate::census::scan::AncestryWalk::Ended(Vec::new()),
                }],
                sccache:              SccacheServer::Stopped,
                root_status:          capture.root_status,
                shared_directory:     capture.shared_directory,
            })
            .expect("scan receiver is alive");

        assert!(drain_scans(
            &mut app,
            &scans,
            RosterScanTime::live(Instant::now())
        ));
        let received = app
            .roster
            .groups()
            .first()
            .expect("received group")
            .rows()
            .next()
            .expect("received process");
        assert_eq!(received.process, process);
        assert!(!drain_scans(
            &mut app,
            &scans,
            RosterScanTime::live(Instant::now())
        ));
    }

    #[test]
    #[cfg(feature = "attract")]
    fn app_modal_consumes_app_and_framework_globals_until_escape() {
        let mut app = App::new_for_test().expect("test app should build");
        dispatch_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
        );
        assert!(app.favorites_overlay.is_open());

        dispatch_key(&mut app, key(KeyCode::Char(' ')));
        dispatch_key(&mut app, key(KeyCode::Char('?')));
        assert_eq!(app.updates, Updates::Live);
        assert_eq!(app.framework.overlay(), None);
        assert!(app.favorites_overlay.is_open());

        dispatch_key(&mut app, key(KeyCode::Esc));
        assert!(!app.favorites_overlay.is_open());
    }

    #[test]
    #[cfg(feature = "attract")]
    fn coalesced_resize_refreshes_currency_after_attract_reclamping() {
        let mut app = App::new_for_test().expect("test app should build");
        let rows = tui_pane::parse_favorite_rows_for_test(FAVORITE_ROW)
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
        tui_pane::open_favorites_on_state_for_test(
            &mut app,
            FavoritesFileState::Loaded {
                path: PathBuf::from("/tmp/favorites.toml"),
                rows,
            },
        );
        let mut terminal =
            Terminal::new(TestBackend::new(100, 30)).expect("test terminal should be created");
        terminal
            .draw(|frame| app.favorites_overlay.render(frame, ColorMode::Colored))
            .expect("favorites overlay should render");
        let initial_rows = rendered_buffer_lines(terminal.backend().buffer());
        assert!(initial_rows.iter().any(|line| line.contains("▸● ")));

        app.resized(Rect::new(0, 0, 5, 4));
        terminal
            .draw(|frame| app.favorites_overlay.render(frame, ColorMode::Colored))
            .expect("favorites overlay should render before resize refresh");
        let before_refresh = rendered_buffer_lines(terminal.backend().buffer());
        assert!(before_refresh.iter().any(|line| line.contains("▸● ")));

        app.resize_settled();
        assert_ne!(app.attract.current_settings(), favorite_settings);
        terminal
            .draw(|frame| app.favorites_overlay.render(frame, ColorMode::Colored))
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
    #[cfg(feature = "attract")]
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
