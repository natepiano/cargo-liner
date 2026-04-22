use std::io;
use std::io::BufReader;
use std::io::Read;
use std::io::Stdout;
use std::path::Path;
use std::process::ExitCode;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
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

use super::app::App;
use super::app::PendingClean;
use super::app::PollBackgroundStats;
use super::constants::FRAME_POLL_MILLIS;
use super::input;
use super::panes::CiFetchKind;
use super::panes::PendingCiFetch;
use super::panes::PendingExampleRun;
use super::panes::RunTargetKind;
use super::render;
use crate::ci;
use crate::config;
use crate::http::HttpClient;
use crate::perf_log;
use crate::project::AbsolutePath;
use crate::project::RootItem;
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
    frame_elapsed:  Duration,
    input_elapsed:  Duration,
    bg_elapsed:     Duration,
    rows_elapsed:   Duration,
    disk_elapsed:   Duration,
    fit_elapsed:    Duration,
    detail_elapsed: Duration,
    draw_elapsed:   Duration,
    idle_elapsed:   Duration,
    input_count:    usize,
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

pub fn run() -> ExitCode {
    let cfg = match config::try_load() {
        Ok(cfg) => cfg,
        Err(err) => {
            tracing::error!("Error: {err}");
            return ExitCode::FAILURE;
        },
    };
    config::set_active_config(&cfg);
    let perf_log_path = perf_log::init();

    let Ok(rt) = tokio::runtime::Runtime::new() else {
        tracing::error!("Error: failed to create async runtime");
        return ExitCode::FAILURE;
    };
    let Some(http_client) = HttpClient::new(rt.handle().clone()) else {
        tracing::error!("Error: failed to create HTTP client");
        return ExitCode::FAILURE;
    };
    let scan_started_at = std::time::Instant::now();
    tracing::info!(kind = "initial", run = 1, "scan_start");
    let scan_dirs = scan::resolve_include_dirs(&cfg.tui.include_dirs);
    let metadata_store = Arc::new(Mutex::new(crate::project::WorkspaceMetadataStore::new()));
    let (bg_tx, bg_rx) = scan::spawn_streaming_scan(
        scan_dirs,
        &cfg.tui.inline_dirs,
        cfg.tui.include_non_rust,
        http_client.clone(),
        Arc::clone(&metadata_store),
    );
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
            tracing::error!("Error: failed to initialize terminal: {e}");
            return ExitCode::FAILURE;
        },
    };

    let mut app = App::new(
        &projects,
        bg_tx,
        bg_rx,
        &cfg,
        http_client,
        scan_started_at,
        metadata_store,
    );
    tracing::info!(perf_log = %perf_log_path.display(), "tui_ready");
    let input_rx = spawn_input_thread();

    let result = event_loop(&mut terminal, &mut app, &input_rx);

    let should_restart = app.should_restart();
    let _ = restore_terminal(&mut terminal);

    if should_restart {
        restart_self();
    }

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("Error: {e}");
            ExitCode::FAILURE
        },
    }
}

/// Replace the current process with a fresh instance of the same binary.
fn restart_self() {
    use std::os::unix::process::CommandExt;
    let exe = AbsolutePath::from(
        std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("cargo-port")),
    );
    let args: Vec<String> = std::env::args().skip(1).collect();
    let err = std::process::Command::new(exe.as_path()).args(&args).exec();
    tracing::error!("Failed to restart: {err}");
}

fn spawn_input_thread() -> mpsc::Receiver<Event> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        while let Ok(event) = crossterm::event::read() {
            if tx.send(event).is_err() {
                break;
            }
        }
    });
    rx
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    input_rx: &mpsc::Receiver<Event>,
) -> io::Result<()> {
    let mut rearmed_after_first_draw = false;
    loop {
        let frame_started = Instant::now();

        let (input_count, input_elapsed) = process_input_frame(app, input_rx);
        if app.should_quit() {
            return Ok(());
        }

        let (bg_stats, bg_elapsed) = poll_background_frame(app);
        app.poll_cpu_if_due(Instant::now());
        app.prune_discovery_shimmers(Instant::now());
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

        if app.should_quit() {
            flush_pending_selection(app);
            break;
        }

        spawn_pending_background_tasks(app);
        let idle_elapsed = idle_if_no_input(input_count);
        log_slow_frame(
            app,
            &bg_stats,
            &FrameMetrics {
                frame_elapsed: frame_started.elapsed(),
                input_elapsed,
                bg_elapsed,
                rows_elapsed,
                disk_elapsed,
                fit_elapsed,
                detail_elapsed,
                draw_elapsed,
                idle_elapsed,
                input_count,
            },
        );
    }
    Ok(())
}

fn process_input_frame(app: &mut App, input_rx: &mpsc::Receiver<Event>) -> (usize, Duration) {
    let started = Instant::now();
    let mut input_count = 0usize;
    while let Ok(event) = input_rx.try_recv() {
        input_count += 1;
        tracing::info!(event = %input::event_label(&event), "input_event_received");
        input::handle_event(app, &event);
        if app.should_quit() {
            break;
        }
    }
    if input_count == 0 {
        flush_deferred_selection(app);
    }
    (input_count, started.elapsed())
}

fn flush_deferred_selection(app: &mut App) {
    if app.selection_changed()
        && let Some(path) = app.last_selected_path()
    {
        save_last_selected(path);
        app.clear_selection_changed();
    }
}

fn flush_pending_selection(app: &App) {
    if app.selection_changed()
        && let Some(path) = app.last_selected_path()
    {
        save_last_selected(path);
    }
}

fn poll_background_frame(app: &mut App) -> (PollBackgroundStats, Duration) {
    let started = Instant::now();
    app.maybe_reload_config_from_disk();
    app.maybe_reload_keymap_from_disk();
    let stats = app.poll_background();
    (stats, started.elapsed())
}

fn clear_terminal_if_dirty(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    if app.terminal_is_dirty() {
        app.clear_terminal_dirty();
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
    if let Some(run) = app.take_pending_example_run() {
        spawn_example_process(app, &run);
    }

    if let Some(pending) = app.pending_cleans_mut().pop_front() {
        spawn_clean_process(app, &pending);
    }

    if let Some(fetch) = app.take_pending_ci_fetch() {
        let abs_path = Path::new(&fetch.project_path);
        app.start_ci_fetch_for(AbsolutePath::from(abs_path));
        app.increment_data_generation();
        spawn_ci_fetch(app, &fetch);
    }
}

fn idle_if_no_input(input_count: usize) -> Duration {
    let started = Instant::now();
    if input_count == 0 {
        thread::sleep(Duration::from_millis(FRAME_POLL_MILLIS));
    }
    started.elapsed()
}

fn log_slow_frame(app: &App, bg_stats: &PollBackgroundStats, metrics: &FrameMetrics) {
    if metrics.frame_elapsed.as_millis() < crate::perf_log::SLOW_FRAME_MS {
        return;
    }
    tracing::info!(
        elapsed_ms = crate::perf_log::ms(metrics.frame_elapsed.as_millis()),
        input_ms = crate::perf_log::ms(metrics.input_elapsed.as_millis()),
        bg_ms = crate::perf_log::ms(metrics.bg_elapsed.as_millis()),
        rows_ms = crate::perf_log::ms(metrics.rows_elapsed.as_millis()),
        disk_ms = crate::perf_log::ms(metrics.disk_elapsed.as_millis()),
        fit_ms = crate::perf_log::ms(metrics.fit_elapsed.as_millis()),
        detail_ms = crate::perf_log::ms(metrics.detail_elapsed.as_millis()),
        draw_ms = crate::perf_log::ms(metrics.draw_elapsed.as_millis()),
        idle_ms = crate::perf_log::ms(metrics.idle_elapsed.as_millis()),
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
        items = app.projects().len(),
        scan_complete = app.is_scan_complete(),
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
            app.set_example_running(Some(run.target_name.clone()));
            return;
        },
    };

    // Store PID so we can kill from the main thread
    let pid = child.id();
    *app.example_child()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pid);

    let name = run.target_name.clone();
    let mode = run.build_mode.label();
    app.set_example_output(vec![format!("Building {name}{mode}...")]);
    app.set_example_running(Some(format!("{name}{mode}")));

    // Take ownership of pipes before moving child to thread
    let stderr = child.stderr.take();
    let stdout = child.stdout.take();

    let pid_holder = app.example_child();
    let tx = app.example_tx();
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
fn read_with_progress(tx: &std::sync::mpsc::Sender<ExampleMsg>, stream: impl io::Read) {
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
    let tx = app.clean_tx();
    let abs_path = pending.abs_path.clone();
    thread::spawn(move || {
        let _ = child.wait();
        let _ = tx.send(CleanMsg::Finished(abs_path));
    });
}

fn spawn_ci_fetch(app: &App, fetch: &PendingCiFetch) {
    // Derive (repo_url, owner, repo) from local git info — no network needed
    let path = Path::new(&fetch.project_path);
    let Some(repo_url) = app.primary_url_for(path) else {
        return;
    };
    let Some(owner_repo) = ci::parse_owner_repo(repo_url) else {
        return;
    };

    let tx = app.ci_fetch_tx();
    let bg_tx = app.bg_tx();
    let client = app.http_client();
    let project_path = fetch.project_path.clone();
    let ci_run_count = fetch.ci_run_count;
    let oldest_created_at = fetch.oldest_created_at.clone();
    let kind = fetch.kind;
    let url = repo_url.to_string();

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
        scan::emit_service_signal(&bg_tx, network);
        let _ = tx.send(CiFetchMsg::Complete {
            path: project_path,
            result,
            kind,
        });
    });
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
    let tx = app.bg_tx();
    let client = app.http_client();
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
            let (info, signal) = client.fetch_crates_io_info(name);
            scan::emit_service_signal(&tx, signal);
            if let Some(info) = info {
                let _ = tx.send(BackgroundMsg::CratesIoVersion {
                    path,
                    version: info.version,
                    downloads: info.downloads,
                });
            }
        }
    });
}
