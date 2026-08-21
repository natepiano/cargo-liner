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

use crossterm::event;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::execute;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tui_pane::FRAME_POLL_MILLIS;
use tui_pane::FrameworkOverlayId;
use tui_pane::GlobalAction;
use tui_pane::Globals;
use tui_pane::KeyBind;
use tui_pane::Keymap;
use tui_pane::OverlayAction;
use tui_pane::matches_open_overlay_toggle;
use tui_pane::overlay_is_in_text_mode;

use crate::app::App;
use crate::config;
use crate::constants::BINARY_NAME;
use crate::globals::AppGlobalAction;
use crate::processes;
use crate::processes::CargoProcess;
use crate::render;
use crate::settings;
use crate::settings::Step;
use crate::theme;

/// Load configuration, install the theme, build the keymap, and run the
/// event loop with the terminal in the alternate screen.
pub(crate) fn run() -> ExitCode {
    let loaded_config = config::load();
    let startup_note = theme::install(&loaded_config.config, config::themes_dir().as_deref());
    let mut app = match App::new(loaded_config, startup_note) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("cargo-tile: keymap: {error}");
            return ExitCode::FAILURE;
        },
    };

    let mut terminal = match setup_terminal() {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!("cargo-tile: terminal setup: {error}");
            return ExitCode::FAILURE;
        },
    };
    let loop_result = event_loop(&mut terminal, &mut app);
    let restart_requested = app.framework.restart_requested();
    let restore_result = restore_terminal(&mut terminal);

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
/// terminal.
fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

/// Undo [`setup_terminal`], leaving the shell as it was found.
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()
}

/// Draw until [`GlobalAction::Quit`] or [`GlobalAction::Restart`] sets
/// the matching lifecycle flag.
///
/// Drawing is demand-driven: a frame is painted only when an event
/// arrived or the process scan came back different. With nothing
/// building and nobody typing there is nothing to repaint, so an idle
/// app costs essentially nothing.
fn event_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> io::Result<()> {
    let input = spawn_input_thread();
    let scans = processes::spawn();
    let mut dirty = true;
    while !app.framework.quit_requested() && !app.framework.restart_requested() {
        if dirty {
            // Re-borrowed every frame: rebinding a key in the keymap
            // overlay swaps the whole map out from under the loop.
            let keymap = Rc::clone(&app.keymap);
            terminal.draw(|frame| render::draw(frame, app, &keymap))?;
            dirty = false;
        }
        match input.recv_timeout(Duration::from_millis(FRAME_POLL_MILLIS)) {
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
                    force_repaint(terminal)?;
                }
                dirty = true;
            },
            Err(RecvTimeoutError::Timeout) => (),
            // The reader hit an unrecoverable crossterm error. A TUI that
            // cannot read input is dead, and without this the loop would
            // spin on a disconnected channel.
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        if drain_scans(app, &scans) {
            dirty = true;
        }
        // A grid in motion repaints every poll until it settles, which
        // is the one thing here that draws without an event behind it.
        if app.tiles.tick() {
            dirty = true;
        }
    }
    Ok(())
}

/// Take the newest process scan, reporting whether it changed anything.
///
/// Only the newest matters: an older scan queued behind it describes a
/// world that has already moved on.
fn drain_scans(app: &mut App, scans: &Receiver<Vec<CargoProcess>>) -> bool {
    let mut latest: Option<Vec<CargoProcess>> = None;
    while let Ok(scan) = scans.try_recv() {
        latest = Some(scan);
    }
    match latest {
        Some(scan) if scan != app.processes => {
            app.processes = scan;
            true
        },
        _ => false,
    }
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
        Event::Resize(..) => Resized::Yes,
        _ => Resized::No,
    }
}

/// Clear the screen and reset the back buffer so the next draw writes
/// every cell.
///
/// [`Terminal::resize`] rather than [`Terminal::clear`]: `clear` opens
/// with a `get_cursor_position` query, which writes a cursor-report
/// escape and then blocks reading the answer out of the same stdin the
/// input thread is reading.
fn force_repaint(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    let area = terminal.size()?.into();
    terminal.resize(area)
}

/// Dispatch ladder: an open overlay short-circuits, otherwise the key
/// resolves against the framework globals.
fn handle_key(app: &mut App, key: KeyEvent) {
    let keymap = Rc::clone(&app.keymap);
    let bind = KeyBind::from(key);
    if let Some(overlay) = app.framework.overlay() {
        let in_text_mode = overlay_is_in_text_mode(&app.framework, overlay);
        if let Some(action) = keymap.framework_globals().action_for(&bind)
            && !in_text_mode
            && (matches_open_overlay_toggle(action, overlay)
                || matches!(action, GlobalAction::Dismiss))
        {
            keymap.dispatch_framework_global(action, app);
            return;
        }
        dispatch_overlay_key(app, &keymap, overlay, bind);
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
/// Nothing here reaches [`tui_pane::SettingsPane::handle_key`]: that
/// would put the pane into its text-edit state, which this binary has
/// no commit path for.
fn dispatch_settings_key(app: &mut App, keymap: &Keymap<App>, bind: KeyBind) {
    if keymap.overlay().action_for(&bind) == Some(OverlayAction::Cancel) {
        keymap.dispatch_framework_global(GlobalAction::Dismiss, app);
        return;
    }
    match bind.code {
        KeyCode::Up => app.framework.settings_pane.viewport_mut().up(),
        KeyCode::Down => app.framework.settings_pane.viewport_mut().down(),
        KeyCode::Left => settings::cycle(app, Step::Prev),
        KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => settings::cycle(app, Step::Next),
        _ => (),
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
