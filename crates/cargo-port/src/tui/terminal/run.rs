use std::io;
use std::io::Stdout;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::Mutex;

use crossterm::event::DisableFocusChange;
use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableFocusChange;
use crossterm::event::EnableMouseCapture;
use crossterm::execute;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use terminal_colorsaurus::QueryOptions;
use terminal_colorsaurus::ThemeMode;
use tui_pane::Appearance;

use super::event_loop;
use super::tree_state;
use crate::config;
use crate::http::HttpClient;
use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::project::WorkspaceMetadataStore;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::tui::app::App;
use crate::tui::constants::PERF_LOG_FILE;
use crate::tui::constants::PREVIOUS_PERF_LOG_FILE;
use crate::tui::settings;

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

/// Probe the terminal for its actual background appearance via an OSC 11
/// query. Returns `None` when the terminal doesn't answer —
/// `terminal-colorsaurus` fails fast for terminals that don't support the
/// query, so this doesn't block on the timeout in that case. Call this in
/// the window after raw mode is enabled but before the input thread
/// starts, so the query response isn't consumed by `crossterm::event::read`.
fn detect_terminal_appearance() -> Option<Appearance> {
    match terminal_colorsaurus::theme_mode(QueryOptions::default()) {
        Ok(ThemeMode::Dark) => Some(Appearance::Dark),
        Ok(ThemeMode::Light) => Some(Appearance::Light),
        Err(err) => {
            tracing::debug!(error = %err, "terminal background detection unavailable");
            None
        },
    }
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
    let cargo_port_config = startup_settings.config.clone();
    config::set_active_config(&cargo_port_config);
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
    tracing::trace!(
        target: tui_pane::PERF_LOG_TARGET,
        kind = "initial",
        run = 1,
        "scan_start"
    );
    let scan_dirs = scan::resolve_include_dirs(&cargo_port_config.tui.include_dirs);
    let metadata_store = Arc::new(Mutex::new(WorkspaceMetadataStore::new()));
    let (background_tx, background_rx) = scan::spawn_streaming_scan(
        scan_dirs,
        &cargo_port_config.tui.inline_dirs,
        cargo_port_config.tui.include_non_rust,
        http_client.clone(),
        Arc::clone(&metadata_store),
    );
    let appearance_tx = background_tx.clone();
    tui_pane::spawn_appearance_poller(rt.handle(), move |appearance| {
        let _ = appearance_tx.send(BackgroundMsg::AppearanceChanged(appearance));
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
    // Probe the terminal background while no thread is reading input yet,
    // so the OSC 11 response can't be swallowed by `spawn_input_thread`.
    app.set_terminal_appearance(detect_terminal_appearance());
    let input_rx = event_loop::spawn_input_thread();

    let result = event_loop::event_loop(&mut terminal, &mut app, &input_rx);

    // Persist tree UI state on exit — including a restart, so the relaunched
    // process restores the same selection and expansions.
    tree_state::save_tree_state(&app);

    let should_restart = app.framework.restart_requested();
    let _ = restore_terminal(&mut terminal);

    if should_restart {
        restart_self();
    }

    let status = match result {
        Ok(()) => 0,
        Err(e) => {
            report_fatal(&format!("{e}"));
            1
        },
    };
    std::process::exit(status);
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
