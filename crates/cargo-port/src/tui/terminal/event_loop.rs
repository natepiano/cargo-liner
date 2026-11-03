use std::collections::HashSet;
use std::io;
use std::io::Stdout;
use std::path::Path;
use std::rc::Rc;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crossterm::event::Event;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::backend::CrosstermBackend;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use tui_pane::Keymap;
use tui_pane::PERF_LOG_TARGET;
use tui_pane::ToastCountdownState;
use tui_pane::ToastId;
use tui_pane::ToastLingerState;
use tui_pane::ToastStyle;
use tui_pane::ToastVisualDeadline;
use tui_pane::TrackedItemActivity;
use tui_pane::TrackedItemKey;
use tui_pane::TrackedItemLifecycle;

use super::frame_metrics::FrameMetrics;
use super::frame_metrics::FrameSpeed;
use super::processes;
use super::run;
use super::tree_state;
use crate::channel;
use crate::channel::Receiver;
use crate::channel::Select;
use crate::channel::TryRecvError;
use crate::config::CargoPortConfig;
use crate::project::AbsolutePath;
use crate::tui::app::App;
use crate::tui::app::PollBackgroundStats;
use crate::tui::input;
use crate::tui::project_list::VisibleRow;
use crate::tui::render;
use crate::tui::state::OwnedRunLaunchStart;

pub(super) fn spawn_input_thread() -> Receiver<Event> {
    let (event_sender, event_receiver) = channel::unbounded();
    thread::spawn(move || {
        while let Ok(event) = crossterm::event::read() {
            if event_sender.send(event).is_err() {
                break;
            }
        }
    });
    event_receiver
}

/// Outcome of draining the input channel for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum InputChannelState {
    #[default]
    Connected,
    Disconnected,
}

impl InputChannelState {
    const fn is_disconnected(self) -> bool { matches!(self, Self::Disconnected) }
}

struct InputDrain {
    count:         usize,
    elapsed:       Duration,
    /// The input thread dropped its sender (a crossterm read error ended
    /// `spawn_input_thread`). A TUI that can no longer read input is
    /// dead, so the loop exits. The event-driven design relies on
    /// detecting this: `Select` reports a *disconnected* crossbeam
    /// receiver as permanently ready, so without this guard the loop
    /// would busy-spin at 100% CPU on the dead input channel (PD8).
    channel_state: InputChannelState,
}

/// Event-driven render loop.
///
/// Each iteration drains every ready source, renders if visible state changed, then
/// blocks in [`wait_for_event`] until something happens — input, a
/// background message, a new CPU sample, or the animation heartbeat. The
/// full drain runs on *every* wake regardless of which channel fired, so
/// the mtime-polled config/keymap/theme reload in [`poll_background_frame`]
/// stay alive while idle (PD1).
///
/// Loop contracts (PD9): quit/restart are set only from input dispatch,
/// which is a `Select` source — so a request always wakes the loop. The
/// first frame is drawn before the first block, then scan/CPU wakes
/// update it; if a future background handler sets quit, it must also be a
/// `Select` source or it will not wake the loop.
pub(super) fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    input_rx: &Receiver<Event>,
) -> io::Result<()> {
    let mut rearmed_after_first_draw = false;
    let mut frame_state = FrameState::default();
    let mut frame_counter = DrawnFrameCounter::new();
    let mut frame_output = FrameOutput::default();
    loop {
        let frame_started = Instant::now();

        let input = process_input_frame(app, input_rx);
        if input.channel_state.is_disconnected() {
            tracing::error!("input channel disconnected; exiting event loop");
            return Ok(());
        }
        if app.framework.quit_requested() || app.framework.restart_requested() {
            return Ok(());
        }

        let (bg_stats, bg_elapsed) = poll_background_frame(app);
        let tick_now = Instant::now();
        let cpu_started = Instant::now();
        let cpu_changed = app.panes.cpu_tick();
        let cpu_elapsed = cpu_started.elapsed();
        app.scan.prune_shimmers(tick_now);

        let changed = frame_state.needs_draw(app, tick_now);
        let inspect_frame = changed
            || input.count > 0
            || cpu_changed
            || bg_stats.bg_msgs > 0
            || bg_stats.ci_msgs > 0
            || bg_stats.owned_run_events > 0;
        let mut rows_elapsed = Duration::ZERO;
        let mut disk_elapsed = Duration::ZERO;
        let mut fit_elapsed = Duration::ZERO;
        let mut detail_elapsed = Duration::ZERO;
        let mut draw_elapsed = Duration::ZERO;
        if inspect_frame {
            rows_elapsed = measure(|| app.ensure_visible_rows_cached());
            disk_elapsed = measure(|| app.ensure_disk_cache());
            fit_elapsed = measure(|| app.ensure_fit_widths_cached());
            detail_elapsed = measure(|| app.ensure_detail_cached());
            if let FrameDraw::Drawn(elapsed) = draw_frame(terminal, app, &mut frame_output)? {
                draw_elapsed = elapsed;
                frame_counter.drawn();
            }
            frame_state.record_draw(app, tick_now);
            if !rearmed_after_first_draw {
                let _ = run::enable_input_modes();
                rearmed_after_first_draw = true;
            }
        }
        frame_counter.log_if_due();

        if app.framework.quit_requested() || app.framework.restart_requested() {
            flush_pending_selection(app);
            break;
        }

        spawn_pending_background_tasks(app);
        log_performance(
            app,
            &bg_stats,
            &FrameMetrics {
                frame_elapsed: frame_started.elapsed(),
                input_elapsed: input.elapsed,
                bg_elapsed,
                cpu_elapsed,
                rows_elapsed,
                disk_elapsed,
                fit_elapsed,
                detail_elapsed,
                draw_elapsed,
                input_count: input.count,
            },
        );

        // `wait_for_event` blocks until an event or animation tick; `FrameMetrics::frame_elapsed`
        // measures only the preceding work, deliberately excluding this wait.
        wait_for_event(app, input_rx);
    }
    Ok(())
}

/// Block until one of the render-loop's channels is ready, or until the
/// animation heartbeat elapses. [`Select::ready_timeout`] signals
/// readiness *without consuming* — the per-source drain in [`event_loop`]
/// does the receiving and runs in full on every wake.
///
/// The `Select` is rebuilt every call because `swap_background_channel`
/// (rescan) replaces the background receiver wholesale. The CPU-sample
/// receiver is registered only while the monitor is sampling: a failed
/// worker spawn leaves the sample sender dropped, and a disconnected
/// crossbeam receiver is reported permanently ready, which would
/// busy-spin the loop (PD8). The four App-held background senders never
/// disconnect (App keeps a clone of each), so only input and CPU samples
/// are at risk; input disconnect is handled in [`process_input_frame`].
fn wait_for_event(app: &App, input_rx: &Receiver<Event>) {
    let timeout = app.animation_timeout();
    let mut select = Select::new();
    select.recv(input_rx);
    select.recv(app.background.background_receiver());
    select.recv(app.background.ci_fetch_rx());
    select.recv(app.background.clean_rx());
    select.recv(app.background.owned_run_receiver());
    if app.panes.cpu.is_sampling() {
        select.recv(app.panes.cpu.sample_rx());
    }
    // Ignore the index from `select.ready_timeout(timeout)` because `event_loop` drains every
    // source.
    let _ = select.ready_timeout(timeout);
}

fn process_input_frame(app: &mut App, input_rx: &Receiver<Event>) -> InputDrain {
    let started = Instant::now();
    let mut count = 0usize;
    let mut channel_state = InputChannelState::Connected;
    loop {
        match input_rx.try_recv() {
            Ok(event) => {
                count += 1;
                tracing::trace!(
                    target: PERF_LOG_TARGET,
                    event = %tui_pane::event_label(&event),
                    "input_event_received"
                );
                input::handle_event(app, &event);
                if app.framework.quit_requested() || app.framework.restart_requested() {
                    break;
                }
            },
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                channel_state = InputChannelState::Disconnected;
                break;
            },
        }
    }
    if count == 0 {
        flush_deferred_selection(app);
    }
    InputDrain {
        count,
        elapsed: started.elapsed(),
        channel_state,
    }
}

fn flush_deferred_selection(app: &mut App) {
    if app.project_list.sync().is_changed() {
        tree_state::save_tree_state(app);
        app.project_list.mark_sync_stable();
    }
}

fn flush_pending_selection(app: &App) {
    if app.project_list.sync().is_changed() {
        tree_state::save_tree_state(app);
    }
}

fn poll_background_frame(app: &mut App) -> (PollBackgroundStats, Duration) {
    let started = Instant::now();
    app.maybe_reload_config_from_disk();
    app.maybe_reload_keymap_from_disk();
    app.maybe_reload_themes_from_disk();
    let stats = app.poll_background();
    app.tick_startup_panel();
    (stats, started.elapsed())
}

fn measure(action: impl FnOnce()) -> Duration {
    let started = Instant::now();
    action();
    started.elapsed()
}

/// Render a candidate once, then write it only if cells or cursor changed.
/// Input receipt is a reason to inspect the screen, never proof it changed.
fn draw_frame<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    output: &mut FrameOutput,
) -> Result<FrameDraw, B::Error> {
    let started = Instant::now();
    let size = terminal.size()?;
    let mut candidate = Terminal::new(TestBackend::new(size.width, size.height))
        .unwrap_or_else(|never| match never {});
    candidate
        .draw(|frame| render::ui(frame, app))
        .unwrap_or_else(|never| match never {});
    let screen = RenderedScreen {
        cells:  candidate.backend().buffer().clone(),
        cursor: if candidate.backend().cursor_visible() {
            ScreenCursor::Visible(candidate.backend().cursor_position())
        } else {
            ScreenCursor::Hidden
        },
    };
    if matches!(output, FrameOutput::Drawn(previous) if previous == &screen) {
        return Ok(FrameDraw::Unchanged);
    }
    terminal.draw(|frame| {
        frame.buffer_mut().clone_from(&screen.cells);
        if let ScreenCursor::Visible(position) = screen.cursor {
            frame.set_cursor_position(position);
        }
    })?;
    *output = FrameOutput::Drawn(screen);
    Ok(FrameDraw::Drawn(started.elapsed()))
}

#[derive(Default)]
enum FrameOutput {
    #[default]
    FirstFrame,
    Drawn(RenderedScreen),
}

#[derive(PartialEq)]
struct RenderedScreen {
    cells:  Buffer,
    cursor: ScreenCursor,
}

#[derive(Clone, Copy, PartialEq)]
enum ScreenCursor {
    Hidden,
    Visible(Position),
}

enum FrameDraw {
    Unchanged,
    Drawn(Duration),
}

fn spawn_pending_background_tasks(app: &mut App) {
    if let OwnedRunLaunchStart::Starting(owned_run_id) = app.inflight.begin_owned_run_launch() {
        processes::spawn_owned_run_process(app, owned_run_id);
    }

    if let Some(pending) = app.inflight.pending_cleans_mut().pop_front() {
        processes::spawn_clean_process(app, &pending);
    }

    if let Some(fetch) = app.inflight.take_pending_ci_fetch() {
        let abs_path = AbsolutePath::from(Path::new(&fetch.project_path));
        if processes::spawn_ci_fetch(app, &fetch) {
            app.ci.fetch_tracker.start(abs_path);
            app.scan.bump_generation();
        } else if let Some(task_id) = app.ci.take_fetch_toast() {
            let empty: HashSet<TrackedItemKey> = HashSet::new();
            app.framework.toasts.complete_missing_items(task_id, &empty);
        }
    }
}

fn log_performance(app: &App, bg_stats: &PollBackgroundStats, metrics: &FrameMetrics) {
    match metrics.speed() {
        FrameSpeed::BelowSlowThreshold => {},
        FrameSpeed::Slow => log_slow_frame(app, bg_stats, metrics),
    }
}

fn log_slow_frame(app: &App, bg_stats: &PollBackgroundStats, metrics: &FrameMetrics) {
    tracing::trace!(
        target: PERF_LOG_TARGET,
        frame_ms = tui_pane::perf_log_ms(metrics.frame_elapsed.as_millis()),
        input_ms = tui_pane::perf_log_ms(metrics.input_elapsed.as_millis()),
        bg_ms = tui_pane::perf_log_ms(metrics.bg_elapsed.as_millis()),
        cpu_ms = tui_pane::perf_log_ms(metrics.cpu_elapsed.as_millis()),
        rows_ms = tui_pane::perf_log_ms(metrics.rows_elapsed.as_millis()),
        disk_ms = tui_pane::perf_log_ms(metrics.disk_elapsed.as_millis()),
        fit_ms = tui_pane::perf_log_ms(metrics.fit_elapsed.as_millis()),
        detail_ms = tui_pane::perf_log_ms(metrics.detail_elapsed.as_millis()),
        draw_ms = tui_pane::perf_log_ms(metrics.draw_elapsed.as_millis()),
        input_count = metrics.input_count,
        bg_msgs = bg_stats.bg_msgs,
        disk_usage_msgs = bg_stats.disk_usage_msgs,
        git_info_msgs = bg_stats.git_info_msgs,
        lint_status_msgs = bg_stats.lint_status_msgs,
        language_progress_msgs = bg_stats.language_progress_msgs,
        ci_msgs = bg_stats.ci_msgs,
        example_msgs = bg_stats.owned_run_events,
        tree_results = bg_stats.tree_results,
        fit_results = bg_stats.fit_results,
        disk_results = bg_stats.disk_results,
        needs_rebuild = bg_stats.rebuild_status.needs_rebuild(),
        items = app.project_list.len(),
        scan_complete = app.scan.is_complete(),
        "slow_frame"
    );
}

/// Inputs shared by a rendered frame. Time is represented by a deadline,
/// so an idle heartbeat does not create a new input on its own.
#[derive(Clone, PartialEq)]
struct FrameKey {
    row_generation:      u64,
    detail_generation:   u64,
    selected_row:        FrameSelection,
    scan:                ScanFrameState,
    lint_animation:      LintAnimation,
    operation_animation: OperationAnimation,
    config:              CargoPortConfig,
    keymap:              FrameKeymap,
    sccache:             SccacheFrameNote,
    toasts:              Vec<ToastFrameKey>,
}

/// Retain the installed keymap so a reload cannot reuse its identity.
#[derive(Clone)]
struct FrameKeymap(Rc<Keymap<App>>);

impl PartialEq for FrameKeymap {
    fn eq(&self, other: &Self) -> bool { Rc::ptr_eq(&self.0, &other.0) }
}

#[derive(Clone, PartialEq)]
enum SccacheFrameNote {
    Unavailable,
    Reported { label: String, value: String },
}

#[derive(Clone, PartialEq, Eq)]
enum ScanFrameState {
    Running,
    /// Discovery shimmers can fade after scanning finishes.
    DiscoveryShimmer,
    Complete,
}

#[derive(Clone, PartialEq, Eq)]
enum LintAnimation {
    Idle,
    Running,
}

#[derive(Clone, PartialEq, Eq)]
enum OperationAnimation {
    Idle,
    Running,
}

impl FrameKey {
    fn from_app(app: &App, now: Instant) -> Self {
        Self {
            row_generation:      app.project_list.row_generation,
            detail_generation:   app.scan.generation(),
            selected_row:        app
                .project_list
                .selected_row()
                .map_or(FrameSelection::Empty, FrameSelection::Selected),
            scan:                if !app.scan.is_complete() {
                ScanFrameState::Running
            } else if app.scan.has_active_shimmers() {
                ScanFrameState::DiscoveryShimmer
            } else {
                ScanFrameState::Complete
            },
            lint_animation:      if app.project_list.has_running_lints() {
                LintAnimation::Running
            } else {
                LintAnimation::Idle
            },
            operation_animation: if app.inflight.needs_animation() {
                OperationAnimation::Running
            } else {
                OperationAnimation::Idle
            },
            config:              app.config.current().clone(),
            keymap:              FrameKeymap(Rc::clone(&app.framework_keymap)),
            sccache:             app.sccache_status_line.note().map_or(
                SccacheFrameNote::Unavailable,
                |note| SccacheFrameNote::Reported {
                    label: note.label,
                    value: note.value,
                },
            ),
            toasts:              app
                .framework
                .toasts
                .active_views(now)
                .iter()
                .map(|toast| ToastFrameKey {
                    id:         toast.id(),
                    title:      toast.title().to_owned(),
                    body:       toast.body().to_owned(),
                    style:      toast.style(),
                    has_action: toast.has_action(),
                    countdown:  toast.countdown(),
                    linger:     toast.linger(),
                    items:      toast
                        .tracked_items()
                        .iter()
                        .map(|item| ToastItemFrameKey {
                            label:     item.label.clone(),
                            activity:  item.activity,
                            lifecycle: item.lifecycle,
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

#[derive(Default)]
enum FrameState {
    #[default]
    FirstFrame,
    Drawn {
        key:                Box<FrameKey>,
        next_visual_change: ToastVisualDeadline,
    },
}

impl FrameState {
    /// The caller supplies time so frame inputs and elapsed animation deadlines
    /// can be checked independently of scheduler delays.
    fn needs_draw(&mut self, app: &App, now: Instant) -> bool {
        let key = FrameKey::from_app(app, now);
        let changed = match self {
            Self::FirstFrame => true,
            Self::Drawn {
                key: previous,
                next_visual_change,
            } => {
                previous.as_ref() != &key
                    || matches!(next_visual_change, ToastVisualDeadline::At(deadline) if now >= *deadline)
            },
        };
        if changed {
            self.record_draw(app, now);
        }
        changed
    }

    fn record_draw(&mut self, app: &App, now: Instant) {
        let timeout = app.animation_timeout_at(now);
        let next_visual_change = if timeout < Duration::from_secs(1) {
            ToastVisualDeadline::At(now + timeout)
        } else {
            app.framework.toasts.next_visual_change_deadline(now)
        };
        *self = Self::Drawn {
            key: Box::new(FrameKey::from_app(app, now)),
            next_visual_change,
        };
    }
}

/// Count all terminal draws, including fast ones and zero-draw idle windows.
struct DrawnFrameCounter {
    window_started: Instant,
    frames:         u64,
}

impl DrawnFrameCounter {
    fn new() -> Self {
        Self {
            window_started: Instant::now(),
            frames:         0,
        }
    }

    const fn drawn(&mut self) { self.frames += 1; }

    fn log_if_due(&mut self) {
        let elapsed = self.window_started.elapsed();
        if elapsed < Duration::from_secs(1) {
            return;
        }
        tracing::trace!(target: PERF_LOG_TARGET, drawn_frames = self.frames,
            window_ms = tui_pane::perf_log_ms(elapsed.as_millis()),
            frames_per_second = f64::from(u32::try_from(self.frames).unwrap_or(u32::MAX)) / elapsed.as_secs_f64(), "drawn_frame_count");
        self.frames = 0;
        self.window_started = Instant::now();
    }
}

#[cfg(test)]
#[path = "event_loop_tests.rs"]
mod event_loop_tests;

#[derive(Clone, PartialEq, Eq)]
enum FrameSelection {
    Empty,
    Selected(VisibleRow),
}

#[derive(Clone, PartialEq)]
struct ToastFrameKey {
    id:         ToastId,
    title:      String,
    body:       String,
    style:      ToastStyle,
    has_action: bool,
    countdown:  ToastCountdownState,
    linger:     ToastLingerState,
    items:      Vec<ToastItemFrameKey>,
}

#[derive(Clone, PartialEq)]
struct ToastItemFrameKey {
    label:     String,
    activity:  TrackedItemActivity,
    lifecycle: TrackedItemLifecycle,
}
