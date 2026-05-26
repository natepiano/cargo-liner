use std::collections::HashSet;
use std::io;
use std::io::BufReader;
use std::io::Read;
use std::io::Stdout;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::ExitCode;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crossterm::event::DisableFocusChange;
use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableFocusChange;
use crossterm::event::EnableMouseCapture;
use crossterm::event::Event;
use crossterm::execute;
use crossterm::terminal::Clear;
use crossterm::terminal::ClearType;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tui_pane::SLOW_FRAME_MS;
use tui_pane::TrackedItemKey;

use super::app::App;
use super::app::PendingClean;
use super::app::PollBackgroundStats;
use super::constants::PERF_LOG_FILE;
use super::constants::PREVIOUS_PERF_LOG_FILE;
use super::input;
use super::panes::CiFetchKind;
use super::panes::PendingCiFetch;
use super::panes::PendingExampleRun;
use super::panes::RunTargetKind;
use super::render;
use super::settings;
use crate::channel;
use crate::channel::Receiver;
use crate::channel::Select;
use crate::channel::Sender;
use crate::channel::TryRecvError;
use crate::ci;
use crate::config;
use crate::http::HttpClient;
use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::project::WorkspaceMetadataStore;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::CiFetchResult;

pub(super) enum ExampleMsg {
    Output(String),
    /// Carriage-return line — replaces the last output line (cargo progress).
    Progress(String),
    Finished,
}

/// Message sent when a background CI fetch completes.
pub(super) enum CiFetchMsg {
    /// The fetch completed with updated runs for the given project path.
    Complete {
        path:   String,
        result: CiFetchResult,
        kind:   CiFetchKind,
    },
}

pub(super) enum CleanMsg {
    Finished(AbsolutePath),
}

#[derive(Clone, Copy)]
struct FrameMetrics {
    frame_elapsed:       Duration,
    input_elapsed:       Duration,
    bg_elapsed:          Duration,
    cpu_elapsed:         Duration,
    run_targets_elapsed: Duration,
    rows_elapsed:        Duration,
    disk_elapsed:        Duration,
    fit_elapsed:         Duration,
    detail_elapsed:      Duration,
    draw_elapsed:        Duration,
    input_count:         usize,
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    rearm_input_modes()?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

pub(super) fn rearm_input_modes() -> io::Result<()> {
    execute!(io::stdout(), EnableMouseCapture, EnableFocusChange)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableFocusChange
    )?;
    Ok(())
}

/// Report a fatal startup/teardown error to both the perf log and
/// stderr. The tracing subscriber writes to a temp file (see
/// [`tui_pane::init_perf_log`]), not the terminal, so a bare
/// `tracing::error!` exits silently from the user's point of view.
/// Echoing to stderr ensures the console shows *something* before the
/// process exits. Call only after the terminal has been restored to
/// the normal screen.
fn report_fatal(message: &str) {
    tracing::error!("{message}");
    eprintln!("cargo-port: {message}");
}

pub fn run() -> ExitCode {
    let startup_settings = match settings::load_cargo_port_settings_for_startup() {
        Ok(settings) => settings,
        Err(err) => {
            report_fatal(&err);
            return ExitCode::FAILURE;
        },
    };
    let cfg = startup_settings.config.clone();
    config::set_active_config(&cfg);
    let perf_log_path = std::env::temp_dir().join(PERF_LOG_FILE);
    let previous_perf_log_path = std::env::temp_dir().join(PREVIOUS_PERF_LOG_FILE);
    tui_pane::init_perf_log(&perf_log_path, &previous_perf_log_path);

    let Ok(rt) = tokio::runtime::Runtime::new() else {
        report_fatal("failed to create async runtime");
        return ExitCode::FAILURE;
    };
    let Some(http_client) = HttpClient::new(rt.handle().clone()) else {
        report_fatal("failed to create HTTP client");
        return ExitCode::FAILURE;
    };
    let scan_started_at = std::time::Instant::now();
    tracing::info!(kind = "initial", run = 1, "scan_start");
    let scan_dirs = scan::resolve_include_dirs(&cfg.tui.include_dirs);
    let metadata_store = Arc::new(Mutex::new(WorkspaceMetadataStore::new()));
    let (background_tx, background_rx) = scan::spawn_streaming_scan(
        scan_dirs,
        &cfg.tui.inline_dirs,
        cfg.tui.include_non_rust,
        http_client.clone(),
        Arc::clone(&metadata_store),
    );
    let appearance_tx = background_tx.clone();
    tui_pane::spawn_appearance_poller(rt.handle(), move |appearance| {
        let _ = appearance_tx.send(scan::BackgroundMsg::AppearanceChanged(appearance));
    });
    let projects: Vec<RootItem> = Vec::new();

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableFocusChange
        );
        original_hook(panic_info);
    }));

    let mut terminal = match setup_terminal() {
        Ok(t) => t,
        Err(e) => {
            report_fatal(&format!("failed to initialize terminal: {e}"));
            return ExitCode::FAILURE;
        },
    };

    let mut app = match App::new(
        &projects,
        background_tx,
        background_rx,
        startup_settings,
        http_client,
        scan_started_at,
        metadata_store,
    ) {
        Ok(app) => app,
        Err(e) => {
            let _ = restore_terminal(&mut terminal);
            report_fatal(&format!("failed to initialize app: {e:#}"));
            return ExitCode::FAILURE;
        },
    };
    tracing::info!(perf_log = %perf_log_path.display(), "tui_ready");
    let input_rx = spawn_input_thread();

    let result = event_loop(&mut terminal, &mut app, &input_rx);

    let should_restart = app.framework.restart_requested();
    let _ = restore_terminal(&mut terminal);

    if should_restart {
        restart_self();
    }

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            report_fatal(&format!("{e}"));
            ExitCode::FAILURE
        },
    }
}

/// Replace the current process with a fresh instance of the same binary.
fn restart_self() {
    let exe = AbsolutePath::from(
        std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("cargo-port")),
    );
    let args: Vec<String> = std::env::args().skip(1).collect();

    #[cfg(unix)]
    {
        let err = std::process::Command::new(exe.as_path()).args(&args).exec();
        tracing::error!("Failed to restart: {err}");
    }

    #[cfg(windows)]
    {
        match std::process::Command::new(exe.as_path())
            .args(&args)
            .spawn()
        {
            Ok(_) => std::process::exit(0),
            Err(err) => tracing::error!("Failed to restart: {err}"),
        }
    }
}

fn spawn_input_thread() -> Receiver<Event> {
    let (tx, rx) = channel::unbounded();
    thread::spawn(move || {
        while let Ok(event) = crossterm::event::read() {
            if tx.send(event).is_err() {
                break;
            }
        }
    });
    rx
}

/// Outcome of draining the input channel for one frame.
struct InputDrain {
    count:        usize,
    elapsed:      Duration,
    /// The input thread dropped its sender (a crossterm read error ended
    /// `spawn_input_thread`). A TUI that can no longer read input is
    /// dead, so the loop exits. The event-driven design relies on
    /// detecting this: `Select` reports a *disconnected* crossbeam
    /// receiver as permanently ready, so without this guard the loop
    /// would busy-spin at 100% CPU on the dead input channel (PD8).
    disconnected: bool,
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
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    input_rx: &Receiver<Event>,
) -> io::Result<()> {
    let mut rearmed_after_first_draw = false;
    loop {
        let frame_started = Instant::now();

        let input = process_input_frame(app, input_rx);
        if input.disconnected {
            tracing::error!("input channel disconnected; exiting event loop");
            return Ok(());
        }
        if app.framework.quit_requested() || app.framework.restart_requested() {
            return Ok(());
        }

        let (bg_stats, bg_elapsed) = poll_background_frame(app);
        let tick_now = Instant::now();
        let cpu_elapsed = measure(|| app.panes.cpu_tick());
        let run_targets_elapsed = measure(|| app.running_targets_tick(tick_now));
        app.scan.prune_shimmers(tick_now);
        clear_terminal_if_dirty(terminal, app)?;

        let rows_elapsed = measure(|| app.ensure_visible_rows_cached());
        let disk_elapsed = measure(|| app.ensure_disk_cache());
        let fit_elapsed = measure(|| app.ensure_fit_widths_cached());
        let detail_elapsed = measure(|| app.ensure_detail_cached());
        let draw_elapsed = draw_frame(terminal, app)?;
        if !rearmed_after_first_draw {
            let _ = rearm_input_modes();
            rearmed_after_first_draw = true;
        }

        if app.framework.quit_requested() || app.framework.restart_requested() {
            flush_pending_selection(app);
            break;
        }

        spawn_pending_background_tasks(app);
        log_slow_frame(
            app,
            &bg_stats,
            &FrameMetrics {
                frame_elapsed: frame_started.elapsed(),
                input_elapsed: input.elapsed,
                bg_elapsed,
                cpu_elapsed,
                run_targets_elapsed,
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
    let timeout = app.animation_timeout();
    let mut select = Select::new();
    select.recv(input_rx);
    select.recv(app.background.background_receiver());
    select.recv(app.background.ci_fetch_rx());
    select.recv(app.background.clean_rx());
    select.recv(app.background.example_rx());
    if app.panes.cpu.is_sampling() {
        select.recv(app.panes.cpu.sample_rx());
    }
    // The fired index is ignored: the loop body drains every source.
    let _ = select.ready_timeout(timeout);
}

fn process_input_frame(app: &mut App, input_rx: &Receiver<Event>) -> InputDrain {
    let started = Instant::now();
    let mut count = 0usize;
    let mut disconnected = false;
    loop {
        match input_rx.try_recv() {
            Ok(event) => {
                count += 1;
                tracing::info!(event = %tui_pane::event_label(&event), "input_event_received");
                input::handle_event(app, &event);
                if app.framework.quit_requested() || app.framework.restart_requested() {
                    break;
                }
            },
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                disconnected = true;
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
        disconnected,
    }
}

fn flush_deferred_selection(app: &mut App) {
    if app.project_list.sync().is_changed()
        && let Some(path) = app.project_list.last_selected_path()
    {
        save_last_selected(path);
        app.project_list.mark_sync_stable();
    }
}

fn flush_pending_selection(app: &App) {
    if app.project_list.sync().is_changed()
        && let Some(path) = app.project_list.last_selected_path()
    {
        save_last_selected(path);
    }
}

fn poll_background_frame(app: &mut App) -> (PollBackgroundStats, Duration) {
    let started = Instant::now();
    app.maybe_reload_config_from_disk();
    app.maybe_reload_keymap_from_disk();
    app.maybe_reload_themes_from_disk();
    let stats = app.poll_background();
    (stats, started.elapsed())
}

fn clear_terminal_if_dirty(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    if app.scan.terminal_is_dirty() {
        app.scan.clear_terminal_dirty();
        execute!(terminal.backend_mut(), Clear(ClearType::All))?;
        terminal.clear()?;
    }
    Ok(())
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
    if let Some(run) = app.inflight.take_pending_example_run() {
        spawn_example_process(app, &run);
    }

    if let Some(pending) = app.inflight.pending_cleans_mut().pop_front() {
        spawn_clean_process(app, &pending);
    }

    if let Some(fetch) = app.inflight.take_pending_ci_fetch() {
        let abs_path = AbsolutePath::from(Path::new(&fetch.project_path));
        if spawn_ci_fetch(app, &fetch) {
            app.ci.fetch_tracker.start(abs_path);
            app.scan.bump_generation();
        } else if let Some(task_id) = app.ci.take_fetch_toast() {
            let empty: HashSet<TrackedItemKey> = HashSet::new();
            app.framework.toasts.complete_missing_items(task_id, &empty);
        }
    }
}

fn log_slow_frame(app: &App, bg_stats: &PollBackgroundStats, metrics: &FrameMetrics) {
    if metrics.frame_elapsed.as_millis() < SLOW_FRAME_MS {
        return;
    }
    tracing::info!(
        frame_ms = tui_pane::perf_log_ms(metrics.frame_elapsed.as_millis()),
        input_ms = tui_pane::perf_log_ms(metrics.input_elapsed.as_millis()),
        bg_ms = tui_pane::perf_log_ms(metrics.bg_elapsed.as_millis()),
        cpu_ms = tui_pane::perf_log_ms(metrics.cpu_elapsed.as_millis()),
        run_targets_ms = tui_pane::perf_log_ms(metrics.run_targets_elapsed.as_millis()),
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
        ci_msgs = bg_stats.ci_msgs,
        example_msgs = bg_stats.example_msgs,
        tree_results = bg_stats.tree_results,
        fit_results = bg_stats.fit_results,
        disk_results = bg_stats.disk_results,
        needs_rebuild = bg_stats.needs_rebuild,
        items = app.project_list.len(),
        scan_complete = app.scan.is_complete(),
        "slow_frame"
    );
}

fn spawn_example_process(app: &mut App, run: &PendingExampleRun) {
    let mut cmd = std::process::Command::new("cargo");
    match run.kind {
        RunTargetKind::Binary => {
            cmd.arg("run");
        },
        RunTargetKind::Example => {
            cmd.arg("run").arg("--example").arg(&run.target_name);
        },
        RunTargetKind::Bench => {
            cmd.arg("bench").arg("--bench").arg(&run.target_name);
        },
    }
    if run.build_mode.is_release() {
        cmd.arg("--release");
    }
    if let Some(pkg) = &run.package_name {
        cmd.arg("-p").arg(pkg);
    }
    cmd.current_dir(&run.abs_path)
        .arg("--color=always")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            app.set_example_output(vec![format!("Failed to start: {e}")]);
            app.inflight
                .set_example_running(Some(run.target_name.clone()));
            return;
        },
    };

    // Store PID so we can kill from the main thread
    let pid = child.id();
    *app.inflight
        .example_child()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pid);

    let name = run.target_name.clone();
    let mode = run.build_mode.label();
    app.set_example_output(vec![format!("Building {name}{mode}...")]);
    app.inflight
        .set_example_running(Some(format!("{name}{mode}")));

    // Take ownership of pipes before moving child to thread
    let stderr = child.stderr.take();
    let stdout = child.stdout.take();

    let pid_holder = app.inflight.example_child();
    let tx = app.background.example_sender();
    thread::spawn(move || {
        // Read stderr with \r handling for cargo progress lines
        if let Some(stderr) = stderr {
            read_with_progress(&tx, stderr);
        }
        // Read stdout (typically just program output, plain lines)
        if let Some(stdout) = stdout {
            read_with_progress(&tx, stdout);
        }

        // Wait for the child to finish and clear the PID.
        // Disk usage is updated automatically by the filesystem watcher.
        let _ = child.wait();
        *pid_holder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;

        let _ = tx.send(ExampleMsg::Finished);
    });
}

/// Read a stream byte-by-byte, splitting on `\n` (new line) and `\r` (progress update).
/// `\r`-terminated chunks are sent as `Progress` so the UI replaces the last line.
fn read_with_progress(tx: &Sender<ExampleMsg>, stream: impl io::Read) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];

    while reader.read_exact(&mut byte).is_ok() {
        match byte[0] {
            b'\n' => {
                let line = String::from_utf8_lossy(&buf).to_string();
                let _ = tx.send(ExampleMsg::Output(line));
                buf.clear();
            },
            b'\r' => {
                if !buf.is_empty() {
                    let line = String::from_utf8_lossy(&buf).to_string();
                    let _ = tx.send(ExampleMsg::Progress(line));
                    buf.clear();
                }
            },
            b => buf.push(b),
        }
    }
    // Flush any remaining data
    if !buf.is_empty() {
        let line = String::from_utf8_lossy(&buf).to_string();
        let _ = tx.send(ExampleMsg::Output(line));
    }
}

fn spawn_clean_process(app: &mut App, pending: &PendingClean) {
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("clean")
        .current_dir(&pending.abs_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            app.clean_spawn_failed(&pending.abs_path);
            app.show_timed_toast("cargo clean failed", e.to_string());
            return;
        },
    };
    let tx = app.background.clean_sender();
    let abs_path = pending.abs_path.clone();
    thread::spawn(move || {
        let _ = child.wait();
        let _ = tx.send(CleanMsg::Finished(abs_path));
    });
}

fn spawn_ci_fetch(app: &App, fetch: &PendingCiFetch) -> bool {
    // Derive (repo_url, owner, repo) from local git info — no network needed.
    // Use `fetch_url_for` so a worktree without upstream tracking still resolves.
    let path = Path::new(&fetch.project_path);
    let Some(repo_url) = app.project_list.fetch_url_for(path) else {
        return false;
    };
    let Some(owner_repo) = ci::parse_owner_repo(&repo_url) else {
        return false;
    };

    let tx = app.background.ci_fetch_sender();
    let background_tx = app.background.background_sender();
    let client = app.net.http_client();
    let project_path = fetch.project_path.clone();
    let ci_run_count = fetch.ci_run_count;
    let oldest_created_at = fetch.oldest_created_at.clone();
    let kind = fetch.kind;
    let url = repo_url;

    thread::spawn(move || {
        let (result, network) = match kind {
            CiFetchKind::FetchOlder => {
                let oldest = oldest_created_at
                    .as_deref()
                    .unwrap_or("1970-01-01T00:00:00Z");
                scan::fetch_older_runs(
                    &client,
                    &url,
                    owner_repo.owner(),
                    owner_repo.repo(),
                    oldest,
                    ci_run_count,
                )
            },
            CiFetchKind::Sync => {
                let (result, _meta, signal) = scan::fetch_ci_runs_cached(
                    &client,
                    &url,
                    owner_repo.owner(),
                    owner_repo.repo(),
                    ci_run_count,
                );
                (result, signal)
            },
        };
        scan::emit_service_signal(&background_tx, network);
        let _ = tx.send(CiFetchMsg::Complete {
            path: project_path,
            result,
            kind,
        });
    });
    true
}

fn last_selected_path_file() -> AbsolutePath { scan::cache_dir().join("last_selected.txt").into() }

pub(super) fn load_last_selected() -> Option<AbsolutePath> {
    let path = last_selected_path_file();
    let raw = std::fs::read_to_string(&*path).ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty() && Path::new(trimmed).is_absolute()).then(|| AbsolutePath::from(trimmed))
}

fn save_last_selected(project_path: &AbsolutePath) {
    let _ = std::fs::write(last_selected_path_file(), project_path.to_string());
}

/// Spawn a background thread to fetch details for a single project ahead of the main scan.
pub(super) fn spawn_priority_fetch(app: &App, _path: &str, abs_path: &str, name: Option<&String>) {
    let tx = app.background.background_sender();
    let client = app.net.http_client();
    let abs = AbsolutePath::from(abs_path);
    let project_name = name.cloned();

    thread::spawn(move || {
        let path: AbsolutePath = abs.clone();
        scan::emit_git_info(&tx, &abs);

        let bytes = scan::dir_size(&abs);
        let _ = tx.send(BackgroundMsg::DiskUsage {
            path: path.clone(),
            bytes,
        });

        if let Some(name) = project_name.as_ref() {
            let _ = tx.send(BackgroundMsg::CratesIoFetchQueued { name: name.clone() });
            let (info, signal) = client.fetch_crates_io_info(name);
            scan::emit_service_signal(&tx, signal);
            if let Some(info) = info {
                let _ = tx.send(BackgroundMsg::CratesIoVersion {
                    path,
                    version: info.version,
                    downloads: info.downloads,
                });
            }
            let _ = tx.send(BackgroundMsg::CratesIoFetchComplete { name: name.clone() });
        }
    });
}
