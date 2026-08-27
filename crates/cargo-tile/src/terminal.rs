//! Terminal lifecycle and the input dispatch ladder.

use std::ffi::OsString;
use std::io;
use std::io::Stdout;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crossterm::event;
use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableMouseCapture;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use crossterm::execute;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::FRAME_POLL_MILLIS;
use tui_pane::FrameworkOverlayId;
use tui_pane::GlobalAction;
use tui_pane::Globals;
use tui_pane::KeyBind;
use tui_pane::KeyOutcome;
use tui_pane::Keymap;
use tui_pane::Navigation;
use tui_pane::OverlayAction;
use tui_pane::ToastId;
use tui_pane::ToastSettings;
use tui_pane::matches_open_overlay_toggle;
use tui_pane::overlay_is_in_text_mode;
use tui_pane::toast_body_width;

use crate::app::App;
use crate::app::AppPaneId;
use crate::app::Updates;
use crate::config;
use crate::constants::ATTRACT_FRAME_INTERVAL;
use crate::constants::BINARY_NAME;
use crate::constants::FULL_REPAINT_SECONDS;
use crate::constants::PROBE_THRESHOLD;
use crate::constants::REPAINT_SENTINEL;
use crate::favorites;
use crate::favorites_overlay::FavoritesOverlayFrameOutcome;
use crate::globals::AppGlobalAction;
use crate::interaction;
use crate::iterm2;
use crate::iterm2::ProfileSwitch;
use crate::navigation::AppNavigation;
use crate::probe;
use crate::probe::Counted;
use crate::probe::Phase;
use crate::processes;
use crate::processes::Scan;
use crate::render;
use crate::sccache;
use crate::sccache::SccacheSummary;
use crate::settings;
use crate::settings::Step;
use crate::theme;

/// Extra line step that keeps a `ToastVisualTimeline` active after its
/// corresponding framework animation.
///
/// `Toasts::push_timed` samples its own instant, so `pushed_at`, sampled before
/// that call, makes every app-owned boundary fractionally early.
const TOAST_VISUAL_TIMELINE_SLACK_LINE_STEPS: u32 = 1;

/// The terminal backend, with everything written to it counted on the
/// way out. See [`probe::Counted`].
type Backend = CrosstermBackend<Counted<Stdout>>;

/// Phase whose time boundary determines the next frame for one timed toast.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToastVisualPhase {
    /// The toast is growing into view.
    Entering,
    /// The toast is fully visible and needs no frames before expiry.
    Static,
    /// The toast is leaving the screen from this instant.
    Exiting { started_at: Instant },
}

/// Timing inputs and current visual phase for one timed toast.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ToastVisualTimeline {
    /// Toast whose rendered lifecycle these times describe.
    toast_id:          ToastId,
    /// Instant recorded beside the id returned by `Toasts::push_timed`.
    pushed_at:         Instant,
    /// Time spent asking for entrance frames.
    entrance_duration: Duration,
    /// Instant at which `Toasts::prune` starts the exit phase.
    expires_at:        Instant,
    /// Time spent asking for exit frames.
    exit_duration:     Duration,
    /// Current portion of the rendered lifecycle.
    phase:             ToastVisualPhase,
}

impl ToastVisualTimeline {
    fn new(
        toast_id: ToastId,
        pushed_at: Instant,
        visible_duration: Duration,
        body_text: &str,
        min_interior_lines: usize,
        settings: &ToastSettings,
    ) -> Self {
        let target_height = toast_target_height(body_text, min_interior_lines, settings);
        let renderer_entrance_line_steps = u32::from(target_height.saturating_sub(1));
        let entrance_line_steps =
            renderer_entrance_line_steps.saturating_add(TOAST_VISUAL_TIMELINE_SLACK_LINE_STEPS);
        let entrance_duration = settings
            .animation
            .entrance_duration
            .get()
            .saturating_mul(entrance_line_steps);
        let renderer_exit_line_steps = u32::from(target_height);
        let exit_line_steps =
            renderer_exit_line_steps.saturating_add(TOAST_VISUAL_TIMELINE_SLACK_LINE_STEPS);
        let exit_duration = settings
            .animation
            .exit_duration
            .get()
            .saturating_mul(exit_line_steps);
        Self {
            toast_id,
            pushed_at,
            entrance_duration,
            expires_at: pushed_at + visible_duration,
            exit_duration,
            phase: ToastVisualPhase::Entering,
        }
    }

    fn next_deadline(self, now: Instant, frame_period: Duration) -> VisualDeadline {
        match self.phase {
            ToastVisualPhase::Entering => VisualDeadline::At(
                (now + frame_period)
                    .min(self.pushed_at + self.entrance_duration)
                    .min(self.expires_at),
            ),
            ToastVisualPhase::Static => VisualDeadline::At(self.expires_at),
            ToastVisualPhase::Exiting { started_at } => {
                VisualDeadline::At((now + frame_period).min(started_at + self.exit_duration))
            },
        }
    }

    fn advance(&mut self, now: Instant) -> ToastTimelineUpdate {
        match self.phase {
            ToastVisualPhase::Entering | ToastVisualPhase::Static if now >= self.expires_at => {
                self.phase = ToastVisualPhase::Exiting { started_at: now };
                ToastTimelineUpdate::Repaint
            },
            ToastVisualPhase::Entering if now >= self.pushed_at + self.entrance_duration => {
                self.phase = ToastVisualPhase::Static;
                ToastTimelineUpdate::Repaint
            },
            ToastVisualPhase::Static => ToastTimelineUpdate::Quiet,
            ToastVisualPhase::Exiting { started_at } if now >= started_at + self.exit_duration => {
                ToastTimelineUpdate::Finished
            },
            ToastVisualPhase::Entering | ToastVisualPhase::Exiting { .. } => {
                ToastTimelineUpdate::Repaint
            },
        }
    }
}

/// Result of advancing one toast's visual timeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToastTimelineUpdate {
    /// No frame is needed while the toast remains static.
    Quiet,
    /// The entrance, expiry, or exit needs a frame.
    Repaint,
    /// The exit is complete and one frame must erase the toast.
    Finished,
}

/// All timed toasts whose visual transitions still need event-loop wakes.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) enum ToastVisualSchedule {
    /// No timed toast has an outstanding visual transition.
    #[default]
    Idle,
    /// Timelines for the timed toasts still entering, visible, or exiting.
    Timelines(Vec<ToastVisualTimeline>),
}

impl ToastVisualSchedule {
    /// Record one timed toast using the same body and layout settings
    /// that its framework toast renders with.
    pub(crate) fn record_timed_toast(
        &mut self,
        toast_id: ToastId,
        pushed_at: Instant,
        visible_duration: Duration,
        body_text: &str,
        min_interior_lines: usize,
        settings: &ToastSettings,
    ) {
        self.record(ToastVisualTimeline::new(
            toast_id,
            pushed_at,
            visible_duration,
            body_text,
            min_interior_lines,
            settings,
        ));
    }

    fn record(&mut self, timeline: ToastVisualTimeline) {
        match self {
            Self::Idle => *self = Self::Timelines(vec![timeline]),
            Self::Timelines(timelines) => {
                timelines.retain(|existing| existing.toast_id != timeline.toast_id);
                timelines.push(timeline);
            },
        }
    }

    pub(crate) fn next_deadline(&self, now: Instant, frame_period: Duration) -> VisualDeadline {
        match self {
            Self::Idle => VisualDeadline::NoVisualChangeScheduled,
            Self::Timelines(timelines) => timelines.iter().fold(
                VisualDeadline::NoVisualChangeScheduled,
                |deadline, timeline| deadline.earlier(timeline.next_deadline(now, frame_period)),
            ),
        }
    }

    pub(crate) fn request_frame(&mut self, now: Instant) -> VisualFrameRequest {
        let Self::Timelines(timelines) = self else {
            return VisualFrameRequest::NotNeeded;
        };
        let mut request = VisualFrameRequest::NotNeeded;
        timelines.retain_mut(|timeline| match timeline.advance(now) {
            ToastTimelineUpdate::Quiet => true,
            ToastTimelineUpdate::Repaint => {
                request = VisualFrameRequest::Needed;
                true
            },
            ToastTimelineUpdate::Finished => {
                request = VisualFrameRequest::Needed;
                false
            },
        });
        if timelines.is_empty() {
            *self = Self::Idle;
        }
        request
    }
}

fn toast_target_height(
    body_text: &str,
    min_interior_lines: usize,
    settings: &ToastSettings,
) -> u16 {
    let width = toast_body_width(settings).max(1);
    let body_lines = body_text
        .lines()
        .map(|line| (line.chars().count().max(1).saturating_sub(1) / width) + 1)
        .sum::<usize>()
        .max(1);
    let interior_lines = min_interior_lines.max(body_lines);
    u16::try_from(interior_lines + 2).unwrap_or(u16::MAX)
}

/// Earliest app-owned visual transition that can require another frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VisualDeadline {
    /// No app-owned transition is waiting on time alone.
    NoVisualChangeScheduled,
    /// Wake no later than this instant.
    At(Instant),
}

impl VisualDeadline {
    pub(crate) fn earlier(self, other: Self) -> Self {
        match (self, other) {
            (Self::NoVisualChangeScheduled, deadline)
            | (deadline, Self::NoVisualChangeScheduled) => deadline,
            (Self::At(left), Self::At(right)) => Self::At(left.min(right)),
        }
    }

    fn limit_wait(self, now: Instant, wait: Duration) -> Duration {
        match self {
            Self::NoVisualChangeScheduled => wait,
            Self::At(deadline) => wait.min(deadline.saturating_duration_since(now)),
        }
    }
}

/// Whether a time-driven visual transition asks the event loop for a frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VisualFrameRequest {
    /// No frame is needed.
    NotNeeded,
    /// A frame is needed now.
    Needed,
}

/// Load configuration, install the theme, build the keymap, and run the
/// event loop with the terminal in the alternate screen.
pub(crate) fn run() -> ExitCode {
    let loaded_config = config::load();
    let startup_note = theme::install(&loaded_config.config, config::themes_dir().as_deref());
    // Read before the config is handed to the app, which takes it.
    let iterm2_profile = loaded_config.config.appearance.iterm2_profile.clone();
    let mut app = match App::new(loaded_config, startup_note) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("cargo-tile: keymap: {error}");
            return ExitCode::FAILURE;
        },
    };

    iterm2::install_panic_restore();
    let (mut terminal, profile_switch) = match setup_terminal(&iterm2_profile) {
        Ok(started) => started,
        Err(error) => {
            eprintln!("cargo-tile: terminal setup: {error}");
            return ExitCode::FAILURE;
        },
    };
    let loop_result = event_loop(&mut terminal, &mut app);
    let restart_requested = app.framework.restart_requested();
    let restore_result = restore_terminal(&mut terminal, profile_switch.as_ref());

    if restart_requested && loop_result.is_ok() && restore_result.is_ok() {
        restart_self();
    }

    match loop_result.and(restore_result) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cargo-tile: {error}");
            ExitCode::FAILURE
        },
    }
}

/// Enter raw mode and the alternate screen so the app owns the whole
/// terminal, and turn mouse reporting on so a click can pick a cell.
///
/// The iTerm2 profile is taken last, once the terminal is otherwise
/// ready: a failure before that point returns without having changed
/// anything the caller would then have to put back.
fn setup_terminal(iterm2_profile: &str) -> io::Result<(Terminal<Backend>, Option<ProfileSwitch>)> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let profile_switch = ProfileSwitch::enter(iterm2_profile, &mut stdout)?;
    Ok((
        Terminal::new(CrosstermBackend::new(probe::Counted::new(stdout)))?,
        profile_switch,
    ))
}

/// Undo [`setup_terminal`], leaving the shell as it was found.
///
/// The profile goes back after the alternate screen does, so the shell
/// coming back into view is already wearing it.
fn restore_terminal(
    terminal: &mut Terminal<Backend>,
    profile_switch: Option<&ProfileSwitch>,
) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    if let Some(switch) = profile_switch {
        switch.leave(terminal.backend_mut())?;
    }
    terminal.show_cursor()
}

/// Draw until [`GlobalAction::Quit`] or [`GlobalAction::Restart`] sets
/// the matching lifecycle flag.
///
/// Drawing is demand-driven: a frame is painted only when an event
/// arrived or the process scan came back different. With nothing
/// building and nobody typing there is nothing to repaint, so an idle
/// app costs essentially nothing.
fn event_loop(terminal: &mut Terminal<Backend>, app: &mut App) -> io::Result<()> {
    let input = spawn_input_thread();
    let scans = processes::spawn();
    // Each due read runs on a worker of its own and replies here, so a
    // server that has wedged parks that one thread rather than the loop.
    let (sccache_reads, sccache_replies) = mpsc::channel();
    let mut dirty = true;
    let mut repainted = Instant::now();
    let mut previous = Instant::now();
    let mut attracted = Instant::now();
    let period = Duration::from_millis(FRAME_POLL_MILLIS);
    let mut deadline = Instant::now() + period;
    while !app.framework.quit_requested() && !app.framework.restart_requested() {
        let started = Instant::now();
        probe::frame(started.duration_since(previous), PROBE_THRESHOLD);
        previous = started;
        // Before the frame rather than inside it: settling the window
        // costs several round trips to the window server, which is far
        // longer than a frame, and `terminal.draw` is no place to spend
        // them.
        app.attract.identify();
        if dirty {
            // Re-borrowed every frame: rebinding a key in the keymap
            // overlay swaps the whole map out from under the loop.
            let keymap = Rc::clone(&app.keymap);
            probe::timed(Phase::Draw, || {
                terminal.draw(|frame| render::draw(frame, app, &keymap))
            })?;
            dirty = false;
        }
        // When the next frame is due, carried forward from when the
        // last one was due rather than worked out afresh from the top
        // of this one.
        //
        // The wait below is a condvar timeout, and the system grants it
        // late by a varying couple of milliseconds. Measured from the
        // top of the current frame, that lateness becomes the next
        // frame's starting point and is never given back: the loop asks
        // for its interval, is woken well past it, and settles there --
        // except on the frames where the wake happens to be prompt,
        // which arrive on time. A period alternating between the two is
        // what the eye reads as stop motion. Against a fixed deadline
        // the same lateness merely shortens the following wait.
        deadline += period;
        let now = Instant::now();
        // Far enough behind that catching up would mean a run of frames
        // with no wait at all between them -- a long draw, or a write
        // the emulator held on to. Start the schedule again from here.
        if deadline < now {
            deadline = now + period;
        }
        let visual_deadline = app
            .toast_visual_deadline(now, period)
            .earlier(app.favorites_overlay.visual_deadline(now, period));
        let remaining = visual_deadline.limit_wait(now, deadline.saturating_duration_since(now));
        match input.recv_timeout(remaining) {
            Ok(event) => {
                let mut resized = apply_event(app, &event);
                // Drain the rest of the burst before drawing again: an
                // iTerm2 resize drag delivers many events, and one repaint
                // at the settled size beats a repaint per intermediate width.
                while let Ok(event) = input.try_recv() {
                    if apply_event(app, &event) == Resized::Yes {
                        resized = Resized::Yes;
                    }
                }
                if resized == Resized::Yes {
                    force_repaint(terminal);
                }
                dirty = true;
            },
            Err(RecvTimeoutError::Timeout) => (),
            // The reader hit an unrecoverable crossterm error. A TUI that
            // cannot read input is dead, and without this the loop would
            // spin on a disconnected channel.
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        let now = Instant::now();
        app.framework.toasts.prune(now);
        if app.toast_visual_frame_request(now) == VisualFrameRequest::Needed {
            dirty = true;
        }
        match app.favorites_overlay.advance(now) {
            FavoritesOverlayFrameOutcome::Quiet => {},
            FavoritesOverlayFrameOutcome::Repaint => dirty = true,
            FavoritesOverlayFrameOutcome::CommitRemoval(favorite_id) => {
                let result = favorites::remove(favorite_id);
                app.favorites_overlay.finish_removal(favorite_id, result);
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
            discard_scans(&scans, &sccache_replies);
        } else {
            if drain_scans(app, &scans) {
                dirty = true;
            }
            // The scan above is what says whether a server is up, so
            // the read is claimed after it rather than before: on the
            // first pass that ordering is the difference between the
            // border filling in straight away and waiting out an
            // interval for the next tick.
            if drain_sccache(app, &sccache_replies) {
                dirty = true;
            }
            sccache::refresh_if_due(&mut app.sccache, &sccache_reads, Instant::now());
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
            if app.attract.showing() && attracted.elapsed() >= ATTRACT_FRAME_INTERVAL {
                attracted = Instant::now();
                dirty = true;
            }
            if app.attract.due_back(Instant::now()) {
                dirty = true;
            }
        }
        // Whatever else has written to this terminal is written over
        // here, because a difference-based draw would leave it standing.
        //
        // Never while the attract screen is up. Marking every cell for
        // redraw puts a write of the whole grid into one frame of an
        // animation, which the terminal shows as a tear rather than as
        // a repaint -- and there is nothing to write over anyway, since
        // the strip already paints every cell it covers. The moment the
        // strip leaves, the wait is long overdue and one fires.
        if !app.attract.showing()
            && repainted.elapsed() >= Duration::from_secs(FULL_REPAINT_SECONDS)
        {
            force_repaint(terminal);
            repainted = Instant::now();
            dirty = true;
        }
    }
    Ok(())
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
    app.roster.observe(scan.groups, Instant::now())
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

/// Read events on their own thread and forward them to the render loop.
///
/// [`event::read`] blocks whenever the bytes crossterm has buffered do not
/// yet form a whole event — a partial escape sequence parks it until the
/// rest arrives. On the render thread that stalls drawing *and* the
/// per-frame terminal size query, which is how a resize ends up invisible.
/// Here it stalls nothing but itself.
fn spawn_input_thread() -> Receiver<Event> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        while let Ok(event) = event::read() {
            if sender.send(event).is_err() {
                return;
            }
        }
    });
    receiver
}

/// Whether a drained event changed the terminal size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Resized {
    /// The terminal changed size.
    Yes,
    /// It did not.
    No,
}

/// Apply one event from the input thread.
fn apply_event(app: &mut App, event: &Event) -> Resized {
    match *event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            handle_key(app, key);
            Resized::No
        },
        Event::Mouse(mouse) => {
            handle_mouse(app, mouse);
            Resized::No
        },
        Event::Resize(width, height) => {
            app.attract
                .record_terminal_resize(Rect::new(0, 0, width, height));
            Resized::Yes
        },
        _ => Resized::No,
    }
}

/// Apply one mouse event.
///
/// Only a left press does anything. The position is recorded from every
/// event regardless, which is what lets the framework answer where the
/// pointer was without an event of its own to ask.
fn handle_mouse(app: &mut App, mouse: MouseEvent) {
    tui_pane::record_mouse_pos(mouse.column, mouse.row);
    if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
        return;
    }
    interaction::handle_click(app, Position::new(mouse.column, mouse.row));
}

/// Reset the buffer the next frame is compared against, so that draw
/// writes every cell.
///
/// Neither [`Terminal::clear`] nor [`Terminal::resize`]: both blank the
/// screen before the repaint lands, and at this cadence that blank is a
/// visible blink every couple of seconds. Blanking is also the half of
/// the job that is not needed -- what forces the redraw is the *other*
/// thing those two do, resetting the buffer the next frame is compared
/// against.
///
/// That reset is reachable on its own. Marking every cell of the buffer
/// the next frame will be compared against with [`REPAINT_SENTINEL`]
/// makes all of them differ, so the next draw writes all of them, in
/// one pass, over what is already there. Same repaint, no blank in
/// front of it.
///
/// [`Terminal::swap_buffers`] is what moves the filled buffer into the
/// comparison slot; the frame renders into the other one.
fn force_repaint(terminal: &mut Terminal<Backend>) {
    let buffer = terminal.current_buffer_mut();
    let area = buffer.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buffer[(x, y)].modifier.insert(REPAINT_SENTINEL);
        }
    }
    terminal.swap_buffers();
}

/// Dispatch ladder: an open overlay short-circuits, otherwise the key
/// resolves against the framework globals.
fn handle_key(app: &mut App, key: KeyEvent) {
    let keymap = Rc::clone(&app.keymap);
    let bind = KeyBind::from(key);
    if app.favorites_overlay.is_open() {
        let _ = keymap.dispatch_app_pane(AppPaneId::Favorites, &bind, app);
        return;
    }
    if let Some(overlay) = app.framework.overlay() {
        let in_text_mode = overlay_is_in_text_mode(&app.framework, overlay);
        if let Some(action) = keymap.framework_globals().action_for(&bind)
            && !in_text_mode
            && matches_open_overlay_toggle(action, overlay)
        {
            keymap.dispatch_framework_global(action, app);
            return;
        }
        dispatch_overlay_key(app, &keymap, overlay, bind);
        return;
    }
    // An attract screen that is what the display is showing owns the
    // keyboard, and it owns it ahead of everything else: the keys that
    // steer it are the arrows and `+` `-`, which the grid underneath
    // spends on focus and on opening and closing a tile. A band that
    // could not be steered because a grid nobody can see moved its
    // focus ring would not be steerable at all. Only the keys it
    // actually binds are taken -- `q` still quits, `f` still freezes,
    // and `a` gives the grid back.
    if let Some(attract) = app.attract.keyed_mode()
        && keymap.dispatch_app_pane(AppPaneId::Attract(attract), &bind, app) == KeyOutcome::Consumed
    {
        return;
    }
    if let Some(action) = keymap.framework_globals().action_for(&bind) {
        keymap.dispatch_framework_global(action, app);
        return;
    }
    if let Some(action) = keymap
        .globals::<AppGlobalAction>()
        .and_then(|scope| scope.action_for(&bind))
    {
        AppGlobalAction::dispatcher()(action, app);
    }
}

/// Route a key the open overlay owns.
///
/// Capture comes first: while the keymap overlay is waiting for a
/// replacement binding, every key is the candidate rather than a
/// command.
fn dispatch_overlay_key(
    app: &mut App,
    keymap: &Keymap<App>,
    overlay: FrameworkOverlayId,
    bind: KeyBind,
) {
    if overlay == FrameworkOverlayId::Keymap && app.framework.keymap_pane.is_capturing() {
        let command = app.framework.keymap_pane.handle_capture_key(bind);
        tui_pane::handle_keymap_capture_command(app, keymap, command);
        return;
    }
    match overlay {
        FrameworkOverlayId::Settings => dispatch_settings_key(app, keymap, bind),
        FrameworkOverlayId::Keymap => dispatch_keymap_key(app, keymap, bind),
        FrameworkOverlayId::GlobalShortcuts => dispatch_global_shortcuts_key(app, keymap, bind),
    }
}

/// Keys the keymap overlay owns. The framework runs the whole editor —
/// selection, capture, conflict checks, and the write back to
/// `keymap.toml` — through [`App`]'s
/// [`KeymapEditContext`](tui_pane::KeymapEditContext) impl.
fn dispatch_keymap_key(app: &mut App, keymap: &Keymap<App>, bind: KeyBind) {
    if let Some(action) = keymap.overlay().action_for(&bind) {
        tui_pane::dispatch_keymap_action(action, app, keymap);
        return;
    }
    tui_pane::handle_keymap_navigation_key(app, keymap, bind.code);
}

/// Keys the `?` overlay owns. Editing a row here hands off to the full
/// keymap editor with that row already selected.
fn dispatch_global_shortcuts_key(app: &mut App, keymap: &Keymap<App>, bind: KeyBind) {
    match keymap.overlay().action_for(&bind) {
        Some(OverlayAction::StartEdit) => tui_pane::edit_selected_global_shortcut(app, keymap),
        Some(OverlayAction::Cancel) => keymap.dispatch_framework_global(GlobalAction::Dismiss, app),
        None => app
            .framework
            .global_shortcuts_pane
            .handle_navigation_key(bind.code),
    }
}

/// Keys the settings overlay owns: move the selection, step the value.
///
/// Movement comes from the navigation scope, so the keys that walk
/// this list are the ones `keymap.toml` says they are. Enter and space
/// stay here: they are this overlay's own way of saying "the next
/// value", not a direction anyone would rebind.
///
/// Nothing here reaches [`tui_pane::SettingsPane::handle_key`]: that
/// would put the pane into its text-edit state, which this binary has
/// no commit path for.
fn dispatch_settings_key(app: &mut App, keymap: &Keymap<App>, bind: KeyBind) {
    if keymap.overlay().action_for(&bind) == Some(OverlayAction::Cancel) {
        keymap.dispatch_framework_global(GlobalAction::Dismiss, app);
        return;
    }
    if let Some(action) = keymap
        .navigation()
        .and_then(|scope| scope.action_for(&bind))
    {
        let focused = *app.framework.focused();
        AppNavigation::dispatcher()(action, focused, app);
        return;
    }
    if matches!(bind.code, KeyCode::Enter | KeyCode::Char(' ')) {
        settings::cycle(app, Step::Next);
    }
}

/// Relaunch this binary with the arguments it was started with.
///
/// On unix this replaces the process, so the shell that started
/// `cargo run -p cargo-tile` keeps waiting on the same job.
fn restart_self() {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(BINARY_NAME));
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();

    #[cfg(unix)]
    {
        // `exec` only returns on failure.
        let error = Command::new(&exe).args(&args).exec();
        eprintln!("cargo-tile: restart: {error}");
    }

    #[cfg(windows)]
    match Command::new(&exe).args(&args).spawn() {
        Ok(_) => std::process::exit(0),
        Err(error) => eprintln!("cargo-tile: restart: {error}"),
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use crossterm::event::KeyModifiers;
    use tui_pane::Toasts;

    use super::*;

    fn matching_toast_heights(toasts: &Toasts<App>, toast_id: ToastId, now: Instant) -> Vec<u16> {
        toasts
            .active_views(now)
            .into_iter()
            .filter(|view| view.id() == toast_id)
            .map(|view| view.desired_height())
            .collect()
    }

    fn key(code: KeyCode) -> KeyEvent { KeyEvent::new(code, KeyModifiers::NONE) }

    #[test]
    fn app_modal_consumes_app_and_framework_globals_until_escape() {
        let mut app = App::new_for_test().expect("test app should build");
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
        );
        assert!(app.favorites_overlay.is_open());

        handle_key(&mut app, key(KeyCode::Char('f')));
        handle_key(&mut app, key(KeyCode::Char('?')));
        assert_eq!(app.updates, Updates::Live);
        assert_eq!(app.framework.overlay(), None);
        assert!(app.favorites_overlay.is_open());

        handle_key(&mut app, key(KeyCode::Esc));
        assert!(!app.favorites_overlay.is_open());
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
            handle_key(&mut app, opener);
            assert_eq!(app.framework.overlay(), Some(overlay));
            handle_key(&mut app, key(KeyCode::Char('x')));
            assert_eq!(app.framework.overlay(), Some(overlay));
            handle_key(&mut app, key(KeyCode::Esc));
            assert_eq!(app.framework.overlay(), None);
        }
    }

    #[test]
    fn framework_modal_prevents_a_second_app_modal_from_opening() {
        let mut app = App::new_for_test().expect("test app should build");
        handle_key(&mut app, key(KeyCode::Char('s')));
        assert_eq!(app.framework.overlay(), Some(FrameworkOverlayId::Settings));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.framework.overlay(), Some(FrameworkOverlayId::Settings));
        assert!(!app.favorites_overlay.is_open());
    }

    #[test]
    fn multi_line_toast_schedule_requests_only_transition_frames() {
        const FRAME: Duration = Duration::from_millis(10);
        const MIN_INTERIOR_LINES: usize = 1;

        let settings = ToastSettings::default();
        let entrance_line = settings.animation.entrance_duration.get();
        let exit_line = settings.animation.exit_duration.get();
        let body = "x".repeat(toast_body_width(&settings) * 4);
        let target_height = toast_target_height(&body, MIN_INTERIOR_LINES, &settings);
        let renderer_entrance_line_steps = u32::from(target_height.saturating_sub(1));
        let renderer_entrance = entrance_line.saturating_mul(renderer_entrance_line_steps);
        let entrance = entrance_line.saturating_mul(
            renderer_entrance_line_steps.saturating_add(TOAST_VISUAL_TIMELINE_SLACK_LINE_STEPS),
        );
        let renderer_exit_line_steps = u32::from(target_height);
        let exit = exit_line.saturating_mul(
            renderer_exit_line_steps.saturating_add(TOAST_VISUAL_TIMELINE_SLACK_LINE_STEPS),
        );
        let visible = entrance + FRAME.saturating_mul(4);
        let pushed_at = Instant::now();
        let mut toasts = Toasts::<App>::with_settings(settings.clone());
        let toast_id = toasts.push_timed(
            "Favorite not saved",
            body.clone(),
            visible,
            MIN_INTERIOR_LINES,
        );
        let renderer_heights_at_last_entrance_step =
            matching_toast_heights(&toasts, toast_id, Instant::now() + renderer_entrance);
        let renderer_entrance_ends_at = pushed_at + renderer_entrance;
        let entrance_ends_at = pushed_at + entrance;
        let entrance_before_end = pushed_at + entrance.saturating_sub(FRAME);
        let expires_at = pushed_at + visible;
        let exit_ends_at = expires_at + exit;
        let exit_before_end = expires_at + exit.saturating_sub(FRAME);
        let static_midpoint = entrance_ends_at + FRAME;
        let mut schedule = ToastVisualSchedule::Idle;
        schedule.record(ToastVisualTimeline::new(
            toast_id,
            pushed_at,
            visible,
            &body,
            MIN_INTERIOR_LINES,
            &settings,
        ));

        assert!(target_height >= 5);
        assert_eq!(renderer_heights_at_last_entrance_step, vec![target_height]);
        assert!(entrance > renderer_entrance);
        assert_eq!(
            schedule.request_frame(pushed_at),
            VisualFrameRequest::Needed,
        );
        assert_eq!(
            schedule.next_deadline(pushed_at, FRAME),
            VisualDeadline::At(pushed_at + FRAME),
        );
        assert_eq!(
            schedule.request_frame(renderer_entrance_ends_at),
            VisualFrameRequest::Needed,
        );
        assert_eq!(
            schedule.request_frame(entrance_before_end),
            VisualFrameRequest::Needed,
        );
        assert_eq!(
            schedule.request_frame(entrance_ends_at),
            VisualFrameRequest::Needed,
        );
        assert_eq!(
            schedule.next_deadline(entrance_ends_at, FRAME),
            VisualDeadline::At(expires_at),
        );
        assert_eq!(
            schedule.request_frame(static_midpoint),
            VisualFrameRequest::NotNeeded,
        );
        assert_eq!(
            schedule.next_deadline(static_midpoint, FRAME),
            VisualDeadline::At(expires_at),
        );
        assert_eq!(
            schedule.request_frame(expires_at),
            VisualFrameRequest::Needed,
        );
        assert_eq!(
            schedule.next_deadline(expires_at, FRAME),
            VisualDeadline::At(expires_at + FRAME),
        );
        assert_eq!(
            schedule.request_frame(exit_before_end),
            VisualFrameRequest::Needed,
        );
        assert_eq!(
            schedule.request_frame(exit_ends_at),
            VisualFrameRequest::Needed,
        );
        assert_eq!(schedule, ToastVisualSchedule::Idle);
        assert_eq!(
            schedule.next_deadline(exit_ends_at, FRAME),
            VisualDeadline::NoVisualChangeScheduled,
        );
    }

    #[test]
    fn single_line_toast_schedule_becomes_quiet_before_expiry() {
        const FRAME: Duration = Duration::from_millis(10);
        const MIN_INTERIOR_LINES: usize = 1;

        let settings = ToastSettings::default();
        let entrance_line = settings.animation.entrance_duration.get();
        let body = "Favorite saved";
        let target_height = toast_target_height(body, MIN_INTERIOR_LINES, &settings);
        let min_height = u16::try_from(MIN_INTERIOR_LINES + 2).unwrap_or(u16::MAX);
        let renderer_entrance_line_steps = u32::from(target_height.saturating_sub(1));
        let entrance = entrance_line.saturating_mul(
            renderer_entrance_line_steps.saturating_add(TOAST_VISUAL_TIMELINE_SLACK_LINE_STEPS),
        );
        let visible = Duration::from_secs(5);
        let pushed_at = Instant::now();
        let mut toasts = Toasts::<App>::with_settings(settings.clone());
        let toast_id = toasts.push_timed("Favorite saved", body, visible, MIN_INTERIOR_LINES);
        let initial_heights = matching_toast_heights(&toasts, toast_id, Instant::now());
        let final_renderer_entrance_heights = matching_toast_heights(
            &toasts,
            toast_id,
            Instant::now() + entrance_line.saturating_mul(renderer_entrance_line_steps),
        );
        let entrance_ends_at = pushed_at + entrance;
        let quiet_at = entrance_ends_at + FRAME;
        let expires_at = pushed_at + visible;
        let mut schedule = ToastVisualSchedule::Idle;
        schedule.record(ToastVisualTimeline::new(
            toast_id,
            pushed_at,
            visible,
            body,
            MIN_INTERIOR_LINES,
            &settings,
        ));

        assert_eq!(target_height, min_height);
        assert_eq!(initial_heights, vec![min_height]);
        assert_eq!(final_renderer_entrance_heights, vec![min_height]);
        assert!(quiet_at < expires_at);
        assert_eq!(
            schedule.request_frame(entrance_ends_at),
            VisualFrameRequest::Needed,
        );
        assert_eq!(
            schedule.request_frame(quiet_at),
            VisualFrameRequest::NotNeeded
        );
        assert_eq!(
            schedule.next_deadline(quiet_at, FRAME),
            VisualDeadline::At(expires_at),
        );
    }

    #[test]
    fn toast_exit_schedule_outlives_renderer_line_steps() {
        const MIN_INTERIOR_LINES: usize = 1;

        let settings = ToastSettings::default();
        let exit_line = settings.animation.exit_duration.get();
        let body = "x".repeat(toast_body_width(&settings) * 3);
        let target_height = toast_target_height(&body, MIN_INTERIOR_LINES, &settings);
        let renderer_exit_line_steps = u32::from(target_height);
        let renderer_exit = exit_line.saturating_mul(renderer_exit_line_steps);
        let scheduled_exit = exit_line.saturating_mul(
            renderer_exit_line_steps.saturating_add(TOAST_VISUAL_TIMELINE_SLACK_LINE_STEPS),
        );
        let visible = Duration::from_secs(2);
        let pushed_at = Instant::now();
        let expires_at = pushed_at + visible;
        let last_renderer_line_at =
            expires_at + exit_line.saturating_mul(u32::from(target_height.saturating_sub(1)));
        let renderer_finished_at = expires_at + renderer_exit;
        let schedule_finished_at = expires_at + scheduled_exit;
        let timeline = ToastVisualTimeline::new(
            ToastId(8),
            pushed_at,
            visible,
            &body,
            MIN_INTERIOR_LINES,
            &settings,
        );

        assert_eq!(timeline.exit_duration, scheduled_exit);
        assert!(timeline.exit_duration >= renderer_exit);
        let mut schedule = ToastVisualSchedule::Idle;
        schedule.record(timeline);
        assert_eq!(
            schedule.request_frame(expires_at),
            VisualFrameRequest::Needed,
        );
        assert_eq!(
            schedule.request_frame(last_renderer_line_at),
            VisualFrameRequest::Needed,
        );
        assert_eq!(
            schedule.request_frame(renderer_finished_at),
            VisualFrameRequest::Needed,
        );
        assert!(matches!(schedule, ToastVisualSchedule::Timelines(_)));
        assert_eq!(
            schedule.request_frame(schedule_finished_at),
            VisualFrameRequest::Needed,
        );
        assert_eq!(schedule, ToastVisualSchedule::Idle);
    }
}
