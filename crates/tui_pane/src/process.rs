//! Reading a process's parent where the usual scan cannot.
//!
//! [`kernel_parent`] exists because sysinfo leaves a process's parent
//! unset when it cannot read that process's BSD info, and on macOS
//! that is every process another user owns. One of those stands in the
//! middle of every terminal's chain: `/usr/bin/login` is root's, and it
//! sits between a terminal emulator and the shell running in it. A walk
//! that stopped there never reached the emulator above it.

use sysinfo::Pid;

/// The pid standing above `pid`, asked of the kernel rather than of a
/// process scan.
///
/// `PROC_PIDT_SHORTBSDINFO` is the read macOS allows against a process
/// the caller does not own, which is how `ps` answers this without
/// privileges. A parent of zero is no parent: the kernel reports it for
/// the init process, where a walk stops in any case.
///
/// Every other platform reads a parent out of `/proc` for any process
/// at all, so sysinfo has already answered there and this always hands
/// back [`None`].
#[cfg(target_os = "macos")]
#[allow(
    unsafe_code,
    reason = "the kernel's own parent read has no safe binding"
)]
#[must_use]
pub fn kernel_parent(pid: Pid) -> Option<Pid> {
    let size = i32::try_from(std::mem::size_of::<libc::proc_bsdshortinfo>()).ok()?;
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdshortinfo>::zeroed();
    // SAFETY: `proc_pidinfo` writes at most the byte count it is handed,
    // and that count is the size of the very struct the pointer is to.
    // It answers how many bytes it wrote, and the read below is claimed
    // only on the whole struct having been written.
    let written = unsafe {
        libc::proc_pidinfo(
            i32::try_from(pid.as_u32()).ok()?,
            libc::PROC_PIDT_SHORTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    // SAFETY: the call above reported writing the whole struct.
    let parent = unsafe { info.assume_init() }.pbsi_ppid;
    (parent > 0).then(|| Pid::from_u32(parent))
}

/// The parent sysinfo has already read everywhere but macOS.
#[cfg(not(target_os = "macos"))]
#[must_use]
pub const fn kernel_parent(_pid: Pid) -> Option<Pid> { None }

#[cfg(test)]
mod tests {
    use super::*;

    /// The init process's pid, whose own parent the kernel reports as
    /// nought.
    const ROOT_PROCESS_PID: u32 = 1;

    /// The kernel answers a parent for a process sysinfo could not read
    /// the BSD info of, which on macOS is every process another user
    /// owns -- `/usr/bin/login` among them, standing between a terminal
    /// and the shell in it.
    ///
    /// A parent of nought is the init process's, and reads as no parent
    /// at all: [`ROOT_PROCESS_PID`] is where a walk stops in any case.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_kernel_answers_for_a_process_sysinfo_could_not_read() {
        assert!(kernel_parent(Pid::from_u32(std::process::id())).is_some());
        assert_eq!(kernel_parent(Pid::from_u32(ROOT_PROCESS_PID)), None);
    }
}
