use std::collections::HashSet;
use std::io;
use std::io::Stdout;
use std::path::Path;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crossterm::event::Event;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tui_pane::PERF_LOG_TARGET;
use tui_pane::TrackedItemKey;

use super::frame_metrics::ForegroundFrameInstrumentation;
use super::frame_metrics::FrameMetrics;
use super::processes;
use super::run;
use super::tree_state;
use crate::channel;
use crate::channel::Receiver;
use crate::channel::Select;
use crate::channel::TryRecvError;
use crate::process_observation::ProcessRefreshDeadline;
use crate::process_observation::ProcessRefreshResultReceiver;
use crate::project::AbsolutePath;
use crate::tui::app::App;
use crate::tui::app::PollBackgroundStats;
use crate::tui::input;
use crate::tui::process_refresh::ObserverRefreshTiming;
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
/// Each iteration drains every ready source, renders one frame, then
/// blocks in [`wait_for_event`] until something happens — input, a
/// background message, a new CPU sample, or the animation heartbeat. The
/// full drain runs on *every* wake regardless of which channel fired, so
/// the mtime-polled config/keymap/theme reload in [`poll_background_frame`]
/// and the 1s running-targets poll stay alive while idle (PD1).
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
        let cpu_elapsed = measure(|| app.panes.cpu_tick());
        let process_refresh_started = Instant::now();
        let observer_refresh_timing = app.process_refresh_tick(tick_now);
        let process_refresh_elapsed = process_refresh_started.elapsed();
        app.scan.prune_shimmers(tick_now);

        let rows_elapsed = measure(|| app.ensure_visible_rows_cached());
        let disk_elapsed = measure(|| app.ensure_disk_cache());
        let fit_elapsed = measure(|| app.ensure_fit_widths_cached());
        let detail_elapsed = measure(|| app.ensure_detail_cached());
        let draw_elapsed = draw_frame(terminal, app)?;
        if !rearmed_after_first_draw {
            let _ = run::rearm_input_modes();
            rearmed_after_first_draw = true;
        }

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
                process_refresh_elapsed,
                observer_refresh_timing,
                rows_elapsed,
                disk_elapsed,
                fit_elapsed,
                detail_elapsed,
                draw_elapsed,
                input_count: input.count,
            },
        );

        // Block until the next event or animation tick. `frame_ms` above
        // measures real work only — the wait is deliberately excluded.
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
    let timeout = event_loop_timeout(app, Instant::now());
    let mut select = Select::new();
    select.recv(input_rx);
    select.recv(app.background.background_receiver());
    select.recv(app.background.ci_fetch_rx());
    select.recv(app.background.clean_rx());
    select.recv(app.background.example_rx());
    if app.panes.cpu.is_sampling() {
        select.recv(app.panes.cpu.sample_rx());
    }
    if let ProcessRefreshResultReceiver::DedicatedWorker(process_refresh_result_receiver) =
        app.process_refresh_result_receiver()
    {
        select.recv(process_refresh_result_receiver);
    }
    // The fired index is ignored: the loop body drains every source.
    let _ = select.ready_timeout(timeout);
}

fn event_loop_timeout(app: &App, now: Instant) -> Duration {
    minimum_event_loop_timeout(
        app.animation_timeout(),
        app.process_refresh_next_deadline(),
        now,
    )
}

fn minimum_event_loop_timeout(
    animation_timeout: Duration,
    process_refresh_deadline: ProcessRefreshDeadline,
    now: Instant,
) -> Duration {
    match process_refresh_deadline {
        ProcessRefreshDeadline::At(deadline) => {
            animation_timeout.min(deadline.saturating_duration_since(now))
        },
        ProcessRefreshDeadline::AwaitingWorker | ProcessRefreshDeadline::NotScheduled => {
            animation_timeout
        },
    }
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

fn draw_frame(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> io::Result<Duration> {
    let started = Instant::now();
    terminal.draw(|frame| render::ui(frame, app))?;
    Ok(started.elapsed())
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
    let frame_instrumentation_plan = metrics.instrumentation_plan();
    match frame_instrumentation_plan.observer_refresh_timing {
        ObserverRefreshTiming::NoCompletedRefresh => {},
        ObserverRefreshTiming::Completed(observer_elapsed) => {
            tracing::trace!(
                target: PERF_LOG_TARGET,
                observer_ms = tui_pane::perf_log_ms(observer_elapsed.as_millis()),
                "observer_refresh_completed"
            );
        },
    }
    match frame_instrumentation_plan.foreground_frame {
        ForegroundFrameInstrumentation::BelowSlowThreshold => {},
        ForegroundFrameInstrumentation::SlowFrame => log_slow_frame(app, bg_stats, metrics),
    }
}

fn log_slow_frame(app: &App, bg_stats: &PollBackgroundStats, metrics: &FrameMetrics) {
    tracing::trace!(
        target: PERF_LOG_TARGET,
        frame_ms = tui_pane::perf_log_ms(metrics.frame_elapsed.as_millis()),
        input_ms = tui_pane::perf_log_ms(metrics.input_elapsed.as_millis()),
        bg_ms = tui_pane::perf_log_ms(metrics.bg_elapsed.as_millis()),
        cpu_ms = tui_pane::perf_log_ms(metrics.cpu_elapsed.as_millis()),
        process_refresh_ms = tui_pane::perf_log_ms(metrics.process_refresh_elapsed.as_millis()),
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
        example_msgs = bg_stats.example_msgs,
        tree_results = bg_stats.tree_results,
        fit_results = bg_stats.fit_results,
        disk_results = bg_stats.disk_results,
        needs_rebuild = bg_stats.rebuild_status.needs_rebuild(),
        items = app.project_list.len(),
        scan_complete = app.scan.is_complete(),
        "slow_frame"
    );
}

#[cfg(test)]
mod process_deadline_tests {
    use super::*;

    #[test]
    fn process_deadline_shortens_animation_wait() {
        let now = Instant::now();

        assert_eq!(
            minimum_event_loop_timeout(
                Duration::from_secs(1),
                ProcessRefreshDeadline::At(now + Duration::from_millis(200)),
                now,
            ),
            Duration::from_millis(200)
        );
    }

    #[test]
    fn worker_wait_is_not_an_animation_deadline() {
        let now = Instant::now();

        assert_eq!(
            minimum_event_loop_timeout(
                Duration::from_millis(80),
                ProcessRefreshDeadline::AwaitingWorker,
                now,
            ),
            Duration::from_millis(80)
        );
    }
}
