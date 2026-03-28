use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Stdout;
use std::path::PathBuf;
use std::process::ExitCode;
use std::process::Stdio;
use std::thread;
use std::time::Duration;

use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableMouseCapture;
use crossterm::execute;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use super::app::App;

const FRAME_POLL_MILLIS: u64 = 16;
const CI_FETCH_DISPLAY_COUNT: u32 = 5;
use super::detail::PendingCiFetch;
use super::detail::PendingExampleRun;
use super::detail::RunTargetKind;
use super::input;
use super::render;
use crate::config;
use crate::project::GitInfo;
use crate::project::RustProject;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::CiFetchResult;

pub enum ExampleMsg {
    Output(String),
    Finished,
}

/// Message sent when a background CI fetch completes.
pub enum CiFetchMsg {
    /// The fetch completed with updated runs for the given project path.
    Complete {
        path:   String,
        result: CiFetchResult,
    },
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    Ok(())
}

pub fn run(path: PathBuf) -> ExitCode {
    let Ok(scan_root) = path.canonicalize() else {
        eprintln!("Error: cannot resolve path '{}'", path.display());
        return ExitCode::FAILURE;
    };

    let cfg = config::load();
    let (bg_tx, bg_rx) = scan::spawn_streaming_scan(
        &scan_root,
        cfg.tui.ci_run_count,
        &cfg.tui.exclude_dirs,
        cfg.tui.include_non_rust,
    );
    let projects: Vec<RustProject> = Vec::new();

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original_hook(panic_info);
    }));

    let mut terminal = match setup_terminal() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Error: failed to initialize terminal: {e}");
            return ExitCode::FAILURE;
        },
    };

    let mut app = App::new(scan_root, projects, bg_tx, bg_rx, &cfg);

    let result = event_loop(&mut terminal, &mut app);

    let should_restart = app.should_restart;
    let _ = restore_terminal(&mut terminal);

    if should_restart {
        restart_self();
    }

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        },
    }
}

/// Replace the current process with a fresh instance of the same binary.
fn restart_self() {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("cargo-port"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let err = std::process::Command::new(&exe).args(&args).exec();
    eprintln!("Failed to restart: {err}");
}

fn event_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> io::Result<()> {
    loop {
        app.poll_background();
        app.spinner_tick = app.spinner_tick.wrapping_add(1);
        app.ensure_visible_rows_cached();
        app.ensure_disk_cache();
        app.ensure_detail_cached();
        terminal.draw(|frame| render::ui(frame, app))?;

        // Wait for at least one event (up to 16ms for ~60fps)
        if crossterm::event::poll(Duration::from_millis(FRAME_POLL_MILLIS))? {
            input::handle_event(app, crossterm::event::read()?);

            // Drain any additional queued events without waiting
            while crossterm::event::poll(Duration::ZERO)? {
                input::handle_event(app, crossterm::event::read()?);
                if app.should_quit {
                    return Ok(());
                }
            }
        } else if app.selection_changed {
            // No events this frame — flush deferred selection save to disk
            if let Some(path) = &app.last_selected_path {
                save_last_selected(path);
            }
            app.selection_changed = false;
        }

        if app.should_quit {
            // Flush any pending selection save
            if app.selection_changed
                && let Some(path) = &app.last_selected_path
            {
                save_last_selected(path);
            }
            break;
        }

        // Spawn a pending example as a background process
        if let Some(run) = app.pending_example_run.take() {
            spawn_example_process(app, &run);
        }

        // Spawn a pending CI fetch as a background process
        if let Some(fetch) = app.pending_ci_fetch.take() {
            // Transition to Fetching state, preserving visible runs
            let existing_runs = app
                .ci_state
                .remove(&fetch.project_path)
                .map(|s| match s {
                    super::app::CiState::Fetching { runs, .. }
                    | super::app::CiState::Loaded { runs, .. } => runs,
                })
                .unwrap_or_default();
            app.ci_state.insert(
                fetch.project_path.clone(),
                super::app::CiState::Fetching {
                    runs:  existing_runs,
                    count: CI_FETCH_DISPLAY_COUNT,
                },
            );
            app.data_generation += 1;
            spawn_ci_fetch(app, &fetch);
        }
    }
    Ok(())
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
    if run.release {
        cmd.arg("--release");
    }
    if let Some(pkg) = &run.package_name {
        cmd.arg("-p").arg(pkg);
    }
    cmd.current_dir(&run.abs_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            app.example_output = vec![format!("Failed to start: {e}")];
            app.example_running = Some(run.target_name.clone());
            return;
        },
    };

    // Store PID so we can kill from the main thread
    let pid = child.id();
    *app.example_child
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pid);

    let name = run.target_name.clone();
    let mode = if run.release { " (release)" } else { "" };
    app.example_output = vec![format!("Building {name}{mode}...")];
    app.example_running = Some(format!("{name}{mode}"));

    // Take ownership of pipes before moving child to thread
    let stderr = child.stderr.take();
    let stdout = child.stdout.take();

    let pid_holder = app.example_child.clone();
    let tx = app.example_tx.clone();
    thread::spawn(move || {
        // Read stderr (cargo output goes here)
        if let Some(stderr) = stderr {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let _ = tx.send(ExampleMsg::Output(line));
            }
        }
        // Read stdout
        if let Some(stdout) = stdout {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                let _ = tx.send(ExampleMsg::Output(line));
            }
        }

        // Wait for the child to finish and clear the PID
        let _ = child.wait();
        *pid_holder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        let _ = tx.send(ExampleMsg::Finished);
    });
}

fn spawn_ci_fetch(app: &App, fetch: &PendingCiFetch) {
    // Derive (repo_url, owner, repo) from local git info — no network needed
    let Some(git) = app.git_info.get(&fetch.project_path) else {
        return;
    };
    let Some(repo_url) = &git.url else {
        return;
    };
    let Some((owner, repo)) = crate::ci::parse_owner_repo(repo_url) else {
        return;
    };

    let tx = app.ci_fetch_tx.clone();
    let abs_path = fetch.abs_path.clone();
    let project_path = fetch.project_path.clone();
    let current_count = fetch.current_count;
    let url = repo_url.clone();

    thread::spawn(move || {
        let repo_dir = PathBuf::from(&abs_path);
        let result = scan::fetch_older_runs(&repo_dir, &url, &owner, &repo, current_count);
        let _ = tx.send(CiFetchMsg::Complete {
            path: project_path,
            result,
        });
    });
}

fn last_selected_path_file() -> Option<PathBuf> {
    scan::cache_dir().map(|d| d.join("last_selected.txt"))
}

pub(super) fn load_last_selected() -> Option<String> {
    let path = last_selected_path_file()?;
    std::fs::read_to_string(path).ok().filter(|s| !s.is_empty())
}

fn save_last_selected(project_path: &str) {
    if let Some(path) = last_selected_path_file() {
        let _ = std::fs::write(path, project_path);
    }
}

/// Update the last selected path when the user navigates.
/// If the scan is still running and the selected project doesn't have details yet,
/// spawn a priority fetch to load its data immediately.
pub(super) fn track_selection(app: &mut App) {
    if let Some(project) = app.selected_project() {
        let path = project.path.clone();
        if app.last_selected_path.as_ref() != Some(&path) {
            app.data_generation += 1;
            app.last_selected_path = Some(path);
            // Disk write deferred to save_selection_on_idle / quit
            app.selection_changed = true;
            app.maybe_priority_fetch();
        }
    }
}

/// Spawn a background thread to fetch details for a single project ahead of the main scan.
pub(super) fn spawn_priority_fetch(app: &App, path: &str, abs_path: &str, name: Option<&String>) {
    let tx = app.bg_tx.clone();
    let project_path = path.to_string();
    let abs = PathBuf::from(abs_path);
    let has_git = abs.join(".git").exists();
    let ci_run_count = app.ci_run_count;
    let project_name = name.cloned();

    // Git info is local and instant — also provides the repo URL for CI
    let git_info = if has_git { GitInfo::detect(&abs) } else { None };
    if let Some(ref info) = git_info {
        let _ = tx.send(BackgroundMsg::GitInfo {
            path: project_path.clone(),
            info: info.clone(),
        });
    }

    // CI runs from cache — uses local repo URL, never network
    if let Some(ref repo_url) = git_info.as_ref().and_then(|g| g.url.clone())
        && let Some((owner, repo)) = crate::ci::parse_owner_repo(repo_url)
    {
        let tx_ci = tx.clone();
        let path_ci = project_path.clone();
        let abs_ci = abs.clone();
        let url = repo_url.clone();
        thread::spawn(move || {
            let result = scan::fetch_ci_runs_cached(&abs_ci, &url, &owner, &repo, ci_run_count);
            let runs = match result {
                CiFetchResult::Loaded(runs) | CiFetchResult::CacheOnly(runs) => runs,
            };
            let _ = tx_ci.send(BackgroundMsg::CiRuns {
                path: path_ci,
                runs,
            });
        });
    }

    // Disk + crates.io on another thread (slower)
    thread::spawn(move || {
        let bytes = scan::dir_size(&abs);
        let _ = tx.send(BackgroundMsg::DiskUsage {
            path: project_path.clone(),
            bytes,
        });

        if let Some(name) = project_name.as_ref()
            && let Some(version) = scan::fetch_crates_io_version(name)
        {
            let _ = tx.send(BackgroundMsg::CratesIoVersion {
                path: project_path,
                version,
            });
        }
    });
}
