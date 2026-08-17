//! Free functions shared by unrelated application modules.

#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::io::ErrorKind;

#[cfg(unix)]
use rustix::process::Pid;
#[cfg(unix)]
use rustix::process::Signal;
#[cfg(unix)]
use rustix::process::getpgrp;
#[cfg(unix)]
use rustix::process::kill_process_group as rustix_kill_process_group;
#[cfg(all(unix, test))]
use rustix::process::test_kill_process;
#[cfg(unix)]
use rustix::process::test_kill_process_group;

#[cfg(unix)]
pub(crate) fn hard_kill_process_group(process_group_id: u32) -> io::Result<()> {
    signal_process_group(process_group_id, Signal::KILL)
}

/// Whether `kill(pid, 0)` still reaches this process. A process that has exited
/// but has not been reaped by its parent still answers, so this reports
/// membership in the process table, not that the process is running.
#[cfg(all(unix, test))]
pub(crate) fn process_exists(process_id: u32) -> bool {
    signal_target_pid(process_id).is_ok_and(|target_pid| test_kill_process(target_pid).is_ok())
}

#[cfg(unix)]
pub(crate) fn process_group_exists(process_group_id: u32) -> bool {
    signal_target_pid(process_group_id)
        .is_ok_and(|process_group_pid| test_kill_process_group(process_group_pid).is_ok())
}

#[cfg(unix)]
pub(crate) fn terminate_process_group(process_group_id: u32) -> io::Result<()> {
    signal_process_group(process_group_id, Signal::TERM)
}

/// Convert a process or process group id into the [`Pid`] the `kill` wrappers
/// above take.
#[cfg(unix)]
fn signal_target_pid(process_id: u32) -> io::Result<Pid> {
    let raw_pid = i32::try_from(process_id)
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "signal target exceeds pid_t"))?;
    Pid::from_raw(raw_pid)
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "signal target must be nonzero"))
}

#[cfg(unix)]
fn signal_process_group(process_group_id: u32, signal: Signal) -> io::Result<()> {
    let target_pid = signal_target_pid(process_group_id)?;
    if target_pid == getpgrp() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "refusing to signal Cargo Port's process group",
        ));
    }
    rustix_kill_process_group(target_pid, signal).map_err(io::Error::from)
}
