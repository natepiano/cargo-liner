//! `cargo-berth` invocation that keeps the test process's Claude Code session out.

use std::ffi::OsStr;

use crate::FixtureCommand as Command;
use crate::constants::CLAUDE_CODE_SESSION_ENVIRONMENT;
use crate::git_driver;

/// Start the `cargo-berth` under test without this process's Claude Code session.
///
/// Its git children inherit disabled auto maintenance, so they leave no background process in a
/// test repository.
///
/// `executable` is the test crate's own `env!("CARGO_BIN_EXE_cargo-berth")`, which
/// only that crate can expand.
#[must_use]
pub fn berth_command(executable: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(executable);
    command.env_remove(CLAUDE_CODE_SESSION_ENVIRONMENT);
    git_driver::disable_git_auto_maintenance(&mut command);
    command
}

/// Keep a Git fault or concurrency fixture on the subprocess read route.
/// The benign monitor override makes command configuration explicit without changing status.
pub fn use_git_read_subprocess(command: &mut Command) -> &mut Command {
    command
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.fsmonitor")
        .env("GIT_CONFIG_VALUE_0", "false")
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn fixture_commands_cannot_spawn_during_another_commands_pipe_setup()
    -> Result<(), Box<dyn Error>> {
        let (attempted, attempting) = mpsc::channel();
        let (finished, completed) = mpsc::channel();
        let worker = crate::with_spawn_lock(|| {
            let worker = thread::spawn(move || {
                attempted
                    .send(())
                    .expect("spawn attempt should be observed");
                let output = super::berth_command("/bin/sh")
                    .args(["-c", "printf completed"])
                    .output();
                finished
                    .send(output)
                    .expect("command result should be observed");
            });
            attempting.recv_timeout(Duration::from_millis(250))?;
            // Always release the lock before joining, including on the unfixed path.
            let premature = completed.recv_timeout(Duration::from_millis(100));
            Ok::<_, Box<dyn Error>>((worker, premature))
        })??;
        worker.0.join().map_err(|_| "command worker panicked")?;
        assert!(
            worker.1.is_err(),
            "a fixture child executed while another command's pipe setup held the spawn lock"
        );
        let output = completed.recv_timeout(Duration::from_millis(250))??;
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"completed");
        Ok(())
    }
}
