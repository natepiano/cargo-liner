//! Ownership of a test command and the descendants its pausing shims start.

use std::io;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::Child;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

/// A test command whose entire process group is terminated when its owner unwinds.
///
/// Keep this guard before the temporary directories used by a pausing shim are removed.
/// Descendants must stay in the inherited process group.
pub struct TestProcessGroup {
    group:   u32,
    process: Child,
}

impl TestProcessGroup {
    /// Start a command in its own group, preserving its configured streams.
    ///
    /// # Errors
    /// Returns the operating system error if the command cannot start.
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        let child = command.process_group(0).spawn()?;
        Ok(Self {
            group:   child.id(),
            process: child,
        })
    }

    /// Access the owned child to supply a hook payload or inspect readiness.
    pub const fn child_mut(&mut self) -> &mut Child { &mut self.process }

    /// Collect normal output after the test has continued its shim.
    ///
    /// # Errors
    /// Returns an I/O error if waiting for the child or reading either pipe fails.
    pub fn wait_with_output(mut self) -> io::Result<Output> {
        drop(self.process.stdin.take());
        let stdout = self.process.stdout.take();
        let stderr = self.process.stderr.take();
        std::thread::scope(|scope| {
            let stdout = scope.spawn(move || read_pipe(stdout));
            let stderr = scope.spawn(move || read_pipe(stderr));
            let status = self.process.wait();
            // End inherited pipe writers before joining readers, also on a wait error.
            self.signal_group();
            Ok(Output {
                status: status?,
                stdout: stdout
                    .join()
                    .map_err(|_| io::Error::other("stdout reader panicked"))??,
                stderr: stderr
                    .join()
                    .map_err(|_| io::Error::other("stderr reader panicked"))??,
            })
        })
    }

    fn signal_group(&mut self) {
        let signalled = Command::new("/bin/sh")
            .args([
                "-c",
                "kill -KILL \"$1\"",
                "test-process-group",
                &format!("-{}", self.group),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        // An already exited group is normal after collecting its complete output.
        if !matches!(signalled, Ok(status) if status.success())
            && !matches!(self.process.try_wait(), Ok(Some(_)))
        {
            eprintln!(
                "could not signal test process group {}: {signalled:?}",
                self.group
            );
            let _ = self.process.kill();
        }
    }
}

impl Drop for TestProcessGroup {
    fn drop(&mut self) {
        // A Child's destructor does not terminate it. Signal the group, including a shell
        // blocked opening a FIFO and any Git or sleep children, before reaping our child.
        // The operating system adopts and reaps orphaned descendants.
        self.signal_group();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        loop {
            match self.process.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                },
                result => {
                    eprintln!("test process group {} did not reap: {result:?}", self.group);
                    break;
                },
            }
        }
    }
}

// Child's optional output pipes are converted to bytes at this standard-library boundary.
fn read_pipe(pipe: Option<impl Read>) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(&mut bytes)?;
    }
    Ok(bytes)
}
