use std::io;
use std::process::Command;
use std::process::Output;

use crate::process_observation::DiskReadBytes;

/// `run` captures `Command` output and `DiskReadBytes` before reaping removes the child's counters.
pub(super) fn run(command: &mut Command) -> io::Result<(Output, DiskReadBytes)> {
    #[cfg(target_os = "linux")]
    if crate::process_observation::DiskReadMeasurement::enabled() {
        return linux::capture(command);
    }
    command
        .output()
        .map(|output| (output, DiskReadBytes::Unavailable))
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io;
    use std::io::ErrorKind;
    use std::io::Read;
    use std::mem::MaybeUninit;
    use std::os::fd::AsRawFd;
    use std::os::unix::process::ExitStatusExt;
    use std::process::Child;
    use std::process::ChildStderr;
    use std::process::ChildStdout;
    use std::process::Command;
    use std::process::ExitStatus;
    use std::process::Output;
    use std::process::Stdio;

    use crate::process_observation::DiskReadBytes;

    /// `GitChildOwner` stops and reaps the child if output capture fails.
    enum GitChildState {
        Running,
        Reaped,
    }

    struct GitChildOwner {
        child: Child,
        state: GitChildState,
    }

    impl Drop for GitChildOwner {
        fn drop(&mut self) {
            if matches!(self.state, GitChildState::Running) {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    pub(super) fn capture(command: &mut Command) -> io::Result<(Output, DiskReadBytes)> {
        let mut owner = GitChildOwner {
            child: command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?,
            state: GitChildState::Running,
        };
        let stdout = owner
            .child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("Git stdout pipe is missing"))?;
        let stderr = owner
            .child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("Git stderr pipe is missing"))?;
        let (stdout, stderr) = read_pipes(stdout, stderr)?;
        let (status, read_bytes) = wait_with_read_bytes(&mut owner)?;
        Ok((
            Output {
                status,
                stdout,
                stderr,
            },
            read_bytes,
        ))
    }

    #[expect(
        unsafe_code,
        reason = "wait4 FFI returns read counters for this owned Git child"
    )]
    fn wait_with_read_bytes(owner: &mut GitChildOwner) -> io::Result<(ExitStatus, DiskReadBytes)> {
        let pid = libc::pid_t::try_from(owner.child.id())
            .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
        let mut raw_status = 0;
        let mut usage = MaybeUninit::<libc::rusage>::uninit();
        loop {
            // SAFETY: Both output pointers are writable. The owner retains the exact child PID.
            let result = unsafe { libc::wait4(pid, &raw mut raw_status, 0, usage.as_mut_ptr()) };
            if result >= 0 {
                owner.state = GitChildState::Reaped;
                break;
            }
            let error = io::Error::last_os_error();
            if error.kind() != ErrorKind::Interrupted {
                return Err(error);
            }
        }
        // SAFETY: A successful wait4 call initializes the rusage output.
        let usage = unsafe { usage.assume_init() };
        let blocks = u64::try_from(usage.ru_inblock)
            .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
        Ok((
            ExitStatus::from_raw(raw_status),
            DiskReadBytes::Measured(blocks.saturating_mul(512)),
        ))
    }

    #[expect(
        unsafe_code,
        reason = "poll observes only the two owned Git output pipes"
    )]
    fn read_pipes(
        mut stdout: ChildStdout,
        mut stderr: ChildStderr,
    ) -> io::Result<(Vec<u8>, Vec<u8>)> {
        let mut descriptors = [
            libc::pollfd {
                fd:      stdout.as_raw_fd(),
                events:  libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd:      stderr.as_raw_fd(),
                events:  libc::POLLIN,
                revents: 0,
            },
        ];
        let mut output = Vec::new();
        let mut errors = Vec::new();
        while descriptors.iter().any(|descriptor| descriptor.fd >= 0) {
            // SAFETY: The array contains two valid descriptors for owned, open pipes.
            let result = unsafe { libc::poll(descriptors.as_mut_ptr(), 2, -1) };
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            read_ready_pipe(&mut stdout, &mut descriptors[0], &mut output)?;
            read_ready_pipe(&mut stderr, &mut descriptors[1], &mut errors)?;
        }
        Ok((output, errors))
    }

    fn read_ready_pipe(
        pipe: &mut impl Read,
        descriptor: &mut libc::pollfd,
        bytes: &mut Vec<u8>,
    ) -> io::Result<()> {
        if descriptor.fd < 0 || descriptor.revents == 0 {
            return Ok(());
        }
        let mut buffer = [0; 8192];
        match pipe.read(&mut buffer) {
            Ok(0) => descriptor.fd = -1,
            Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == ErrorKind::Interrupted => {},
            Err(error) => return Err(error),
        }
        Ok(())
    }
}
