//! Taking the terminal, handing it back, and relaunching the binary.

use std::backtrace::Backtrace;
use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::panic;
use std::panic::Location;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitCode;
use std::sync::Once;

use chrono::SecondsFormat;
use chrono::Utc;
use crossterm::cursor::Show;
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
use super::constants::PANIC_LOCATION_UNKNOWN;
use super::constants::PANIC_PAYLOAD_NOT_TEXT;
use super::event_loop;
use super::event_loop::ProbeBackend;
use super::iterm2;
use super::iterm2::ProfileSwitch;
use crate::AppIdentity;

/// Run `app` in the alternate screen until it quits or restarts.
///
/// Restores the terminal and iTerm2 profile on a panic and appends the
/// panic to [`TerminalApp::panic_log`], takes the terminal (raw
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
    let panic_log = app.panic_log();
    let (mut terminal, profile_switch) =
        match setup_terminal::<A::Probe>(iterm2_profile, binary_name, panic_log) {
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
    binary_name: &'static str,
    panic_log: Option<PathBuf>,
) -> io::Result<(Terminal<ProbeBackend<P>>, Option<ProfileSwitch>)> {
    install_panic_restore(binary_name, panic_log);
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
    write_terminal_restore(terminal.backend_mut())?;
    if let Some(switch) = profile_switch {
        switch.leave(terminal.backend_mut())?;
    }
    Ok(())
}

/// Write the controls that return the screen and mouse to the shell.
fn write_terminal_restore(out: &mut impl io::Write) -> io::Result<()> {
    execute!(out, DisableMouseCapture, LeaveAlternateScreen, Show)
}

/// Restore terminal modes before the profile and the existing panic
/// reporter, then append the panic to `panic_log` and name the file on
/// standard error.
///
/// The path is printed last, below the message the existing reporter
/// prints, and only when the append succeeded: a hook that cannot write
/// the file says nothing about it rather than naming a file that does not
/// hold this panic.
fn install_panic_restore(binary_name: &'static str, panic_log: Option<PathBuf>) {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        iterm2::install_panic_restore();
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = write_terminal_restore(&mut io::stdout());
            previous(info);
            let Some(path) = &panic_log else {
                return;
            };
            let message = info.payload_as_str().unwrap_or(PANIC_PAYLOAD_NOT_TEXT);
            let backtrace = Backtrace::force_capture();
            if append_panic_report(path, message, info.location(), &backtrace).is_ok() {
                let _ = writeln!(
                    io::stderr(),
                    "{binary_name}: panic recorded in {}",
                    path.display()
                );
            }
        }));
    });
}

/// Append one panic to `path`: the time in UTC, where it was raised, its
/// message and `backtrace`. The file's directory is created when missing.
///
/// Every failure is returned rather than raised, since this runs inside
/// the panic hook, where a second panic aborts the process.
fn append_panic_report(
    path: &Path,
    message: &str,
    location: Option<&Location<'_>>,
    backtrace: &impl std::fmt::Display,
) -> io::Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let time = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let location = location.map_or_else(|| PANIC_LOCATION_UNKNOWN.to_string(), ToString::to_string);
    writeln!(
        file,
        "{time} panicked at {location}:\n{message}\nbacktrace:\n{backtrace}"
    )
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
        // `Command::exec` only returns on failure.
        let error = Command::new(&exe).args(&args).exec();
        eprintln!("{}: restart: {error}", I::BINARY_NAME);
    }

    #[cfg(windows)]
    match Command::new(&exe).args(&args).spawn() {
        Ok(_) => std::process::exit(0),
        Err(error) => eprintln!("{}: restart: {error}", I::BINARY_NAME),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_report_holds_its_message_location_and_backtrace() -> io::Result<()> {
        const FRAME: &str = "0: tui_pane::runner::lifecycle::install_panic_report";
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("missing").join("panic.log");
        let location = Location::caller();
        append_panic_report(&path, "the grid lost a tile", Some(location), &FRAME)?;
        let report = fs::read_to_string(&path)?;
        assert!(report.contains("the grid lost a tile"), "{report}");
        assert!(
            report.contains(&format!("panicked at {location}:")),
            "{report}"
        );
        let backtrace = report
            .split_once("\nbacktrace:\n")
            .map(|(_, backtrace)| backtrace.trim());
        assert!(
            backtrace.is_some_and(|frames| !frames.is_empty()),
            "{report}"
        );
        assert!(report.contains(FRAME));
        Ok(())
    }

    #[test]
    fn terminal_restore_disables_mouse_leaves_the_screen_and_shows_the_cursor() -> io::Result<()> {
        let mut written = Vec::new();
        write_terminal_restore(&mut written)?;
        let expected =
            b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1049l\x1b[?25h";
        assert_eq!(written, expected);
        Ok(())
    }
}
