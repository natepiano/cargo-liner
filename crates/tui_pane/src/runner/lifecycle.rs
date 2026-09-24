//! Taking the terminal, handing it back, and relaunching the binary.

use std::ffi::OsString;
use std::io;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitCode;

use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableMouseCapture;
use crossterm::execute;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use super::app::FrameProbe;
use super::app::PollWork;
use super::app::TerminalApp;
use super::event_loop;
use super::event_loop::ProbeBackend;
use super::iterm2;
use super::iterm2::ProfileSwitch;
use crate::AppIdentity;

/// Run `app` in the alternate screen until it quits or restarts.
///
/// Puts the iTerm2 profile back on a panic, takes the terminal (raw
/// mode, the alternate screen, mouse reporting, and `iterm2_profile`
/// where the session is iTerm2), then calls `start` for the work the
/// loop polls -- after the setup, so nothing `start` spawns runs
/// before the terminal is the app's. The loop runs until a quit or a
/// restart; [`TerminalApp::before_exit`] runs, the terminal is handed
/// back, and a requested restart relaunches the binary with the
/// arguments it was started with.
///
/// Every failure is printed to standard error prefixed with
/// [`AppIdentity::BINARY_NAME`], and makes the exit code a failure.
pub fn run_terminal<A: TerminalApp, W: PollWork<A>>(
    app: &mut A,
    iterm2_profile: &str,
    start: impl FnOnce(&A) -> W,
) -> ExitCode {
    let binary_name = <A::Identity as AppIdentity>::BINARY_NAME;
    iterm2::install_panic_restore();
    let (mut terminal, profile_switch) = match setup_terminal::<A::Probe>(iterm2_profile) {
        Ok(started) => started,
        Err(error) => {
            eprintln!("{binary_name}: terminal setup: {error}");
            return ExitCode::FAILURE;
        },
    };
    let mut work = start(app);
    let loop_result = event_loop::event_loop(&mut terminal, app, &mut work);
    app.before_exit();
    let restart_requested = app.framework().restart_requested();
    let restore_result = restore_terminal(&mut terminal, profile_switch.as_ref());

    if restart_requested && loop_result.is_ok() && restore_result.is_ok() {
        restart_self::<A::Identity>();
    }

    match loop_result.and(restore_result) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{binary_name}: {error}");
            ExitCode::FAILURE
        },
    }
}

/// Enter raw mode and the alternate screen so the app owns the whole
/// terminal, and turn mouse reporting on so a click can pick a cell.
///
/// The iTerm2 profile is taken last, once the terminal is otherwise
/// ready: a failure before that point returns without having changed
/// anything the caller would then have to put back. Everything here is
/// written to standard output before the probe wraps it, so none of it
/// is counted.
fn setup_terminal<P: FrameProbe>(
    iterm2_profile: &str,
) -> io::Result<(Terminal<ProbeBackend<P>>, Option<ProfileSwitch>)> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let profile_switch = ProfileSwitch::enter(iterm2_profile, &mut stdout)?;
    Ok((
        Terminal::new(CrosstermBackend::new(P::output(stdout)))?,
        profile_switch,
    ))
}

/// Undo [`setup_terminal`], leaving the shell as it was found.
///
/// The profile goes back after the alternate screen does, so the shell
/// coming back into view is already wearing it.
fn restore_terminal<W: io::Write>(
    terminal: &mut Terminal<CrosstermBackend<W>>,
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

/// Relaunch this binary with the arguments it was started with.
///
/// On unix this replaces the process, so the shell that started
/// `cargo run -p <binary>` keeps waiting on the same job.
fn restart_self<I: AppIdentity>() {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(I::BINARY_NAME));
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();

    #[cfg(unix)]
    {
        // `exec` only returns on failure.
        let error = Command::new(&exe).args(&args).exec();
        eprintln!("{}: restart: {error}", I::BINARY_NAME);
    }

    #[cfg(windows)]
    match Command::new(&exe).args(&args).spawn() {
        Ok(_) => std::process::exit(0),
        Err(error) => eprintln!("{}: restart: {error}", I::BINARY_NAME),
    }
}
