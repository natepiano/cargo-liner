//! Bounded request/reply exchanges for persistent Git children.

use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Write;
use std::process::ChildStdin;
use std::process::ChildStdout;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use super::child;
use super::child::ChildDeadline;
use super::child::ChildProcess;

/// A local protocol acknowledgement or installation probe performs no history walk.
pub(super) const CONTROL_REPLY_DEADLINE: Duration = Duration::from_millis(750);

impl ChildProcess {
    /// Read a single protocol reply and restore stdout for subsequent capture.
    ///
    /// # Errors
    /// Returns a named timeout or a pipe error, stopping the child on failure.
    pub(super) fn read_line(&mut self, duration: Duration) -> io::Result<String> {
        let mut output = self
            .child_mut()
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("child stdout was not piped"))?;
        let response = child::worker(move || {
            let mut bytes = Vec::new();
            let mut byte = [0];
            while output.read(&mut byte)? != 0 {
                bytes.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            let line = String::from_utf8(bytes).map_err(io::Error::other)?;
            Ok((output, line))
        })?;
        let deadline = ChildDeadline::new(self.command(), "protocol reply", duration);
        match deadline.receive(&response) {
            Ok((output, line)) => {
                self.child_mut().stdout = Some(output);
                Ok(line)
            },
            Err(error) => {
                self.stop();
                Err(error)
            },
        }
    }

    /// Exchange one line with a persistent child's dedicated protocol worker.
    ///
    /// # Errors
    /// Returns a named timeout or protocol I/O error and stops the child on failure.
    pub(super) fn exchange(
        &mut self,
        pipes: &LineExchange,
        line: &str,
        duration: Duration,
    ) -> io::Result<String> {
        let deadline = ChildDeadline::new(self.command(), "protocol reply", duration);
        let result = pipes
            .requests
            .send(line.to_owned())
            .map_err(io::Error::other)
            .and_then(|()| deadline.receive(&pipes.responses));
        if result.is_err() {
            self.stop();
        }
        result
    }
}

/// A persistent child's serialized request/response pipes, owned by one worker.
pub(super) struct LineExchange {
    requests:  Sender<String>,
    responses: Receiver<io::Result<String>>,
}

impl LineExchange {
    /// Start the line protocol worker; callers receive every reply with a deadline.
    ///
    /// # Errors
    /// Returns an error if the protocol worker cannot start.
    pub(super) fn new(mut input: ChildStdin, output: ChildStdout) -> io::Result<Self> {
        let (requests, pending) = mpsc::channel::<String>();
        let (completed, responses) = mpsc::channel();
        thread::Builder::new()
            .name("child-line-protocol".to_owned())
            .spawn(move || {
                let mut output = BufReader::new(output);
                for request in pending {
                    let result = (|| {
                        writeln!(input, "{request}")?;
                        input.flush()?;
                        let mut response = String::new();
                        output.read_line(&mut response)?;
                        Ok(response)
                    })();
                    if completed.send(result).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests,
            responses,
        })
    }
}
