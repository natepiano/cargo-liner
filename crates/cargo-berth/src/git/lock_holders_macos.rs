//! Inspect open vnode descriptors through the macOS libproc API.

use std::io;
use std::mem::MaybeUninit;
use std::mem::size_of;
use std::mem::size_of_val;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;

use super::GitLockHolders;

/// `PROC_PIDFDVNODEPATHINFO` from the macOS SDK's `sys/proc_info.h`.
const PROC_PIDFDVNODEPATHINFO: i32 = 2;

/// `proc_fileinfo` from the macOS SDK's `sys/proc_info.h`.
#[repr(C)]
struct ProcFileInfo {
    open_flags:  u32,
    status:      u32,
    offset:      i64,
    kind:        i32,
    guard_flags: u32,
}

/// `vnode_fdinfowithpath` from the macOS SDK's `sys/proc_info.h`.
#[repr(C)]
struct VnodeFdInfoWithPath {
    file:  ProcFileInfo,
    vnode: libc::vnode_info_path,
}

/// Report absence only when every relevant process was inspected.
pub(super) fn open_file_holders(path: &Path) -> GitLockHolders {
    let Ok(path) = path.canonicalize() else {
        return GitLockHolders::Unavailable;
    };
    let Ok(metadata) = path.metadata() else {
        return GitLockHolders::Unavailable;
    };
    let Ok(processes) = process_ids() else {
        return GitLockHolders::Unavailable;
    };
    let mut holders = Vec::new();
    let mut incomplete = false;
    for pid in processes.into_iter().filter(|pid| *pid > 0) {
        let owner = match process_owner(pid) {
            Ok(owner) => owner,
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => continue,
            Err(_) => {
                incomplete = true;
                continue;
            },
        };
        match process_holds_file(pid, &path) {
            Ok(true) => {
                if let Ok(pid) = u32::try_from(pid) {
                    holders.push(pid);
                }
            },
            Ok(false) => {},
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {},
            Err(_) => incomplete |= owner == metadata.uid(),
        }
    }
    if !holders.is_empty() {
        GitLockHolders::OpenByProcesses(holders)
    } else if incomplete {
        GitLockHolders::Unavailable
    } else {
        GitLockHolders::NoProcessHoldsIt
    }
}

#[allow(unsafe_code, reason = "libproc lists process IDs without a subprocess")]
fn process_ids() -> io::Result<Vec<libc::pid_t>> {
    // SAFETY: A null buffer and zero length request only the process count.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Err(io::Error::last_os_error());
    }
    let capacity = usize::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(2))
        .ok_or_else(|| io::Error::other("process list size is unavailable"))?;
    let mut processes = vec![0; capacity];
    let bytes = buffer_size(processes.as_slice())?;
    // SAFETY: The initialized PID array is writable for exactly `bytes` bytes.
    let count = unsafe { libc::proc_listallpids(processes.as_mut_ptr().cast(), bytes) };
    let count = usize::try_from(count)
        .ok()
        .filter(|count| *count > 0 && *count < processes.len())
        .ok_or_else(|| io::Error::other("process list is incomplete"))?;
    processes.truncate(count);
    Ok(processes)
}

#[allow(
    unsafe_code,
    reason = "libproc reads the short owner record across user IDs"
)]
fn process_owner(pid: libc::pid_t) -> io::Result<libc::uid_t> {
    let mut info = MaybeUninit::<libc::proc_bsdshortinfo>::zeroed();
    let bytes = buffer_size(std::slice::from_ref(&info))?;
    // SAFETY: The aligned buffer holds `bytes` initialized bytes for the short BSD record.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDT_SHORTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            bytes,
        )
    };
    if written <= 0 {
        return Err(io::Error::last_os_error());
    }
    if written != bytes {
        return Err(io::Error::other(
            "libproc returned an incomplete SDK record",
        ));
    }
    // SAFETY: The call returned the complete record; its fields contain only integer values.
    Ok(unsafe { info.assume_init() }.pbsi_uid)
}

fn process_holds_file(pid: libc::pid_t, path: &Path) -> io::Result<bool> {
    let mut incomplete = false;
    for descriptor in process_descriptors(pid)? {
        if descriptor.proc_fdtype
            != u32::try_from(libc::PROX_FDTYPE_VNODE).map_err(io::Error::other)?
        {
            continue;
        }
        match descriptor_path(pid, descriptor.proc_fd) {
            Ok(target) if target == path.as_os_str().as_bytes() => return Ok(true),
            Ok(_) => {},
            // A descriptor can close between the list and the path query.
            Err(error) if error.raw_os_error() == Some(libc::EBADF) => {},
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => return Err(error),
            Err(_) => incomplete = true,
        }
    }
    if incomplete {
        Err(io::Error::other("process descriptors are incomplete"))
    } else {
        Ok(false)
    }
}

#[allow(unsafe_code, reason = "libproc lists descriptors without a subprocess")]
fn process_descriptors(pid: libc::pid_t) -> io::Result<Vec<libc::proc_fdinfo>> {
    // SAFETY: A null buffer and zero length request only the descriptor buffer size.
    let bytes =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if bytes <= 0 {
        return Err(io::Error::last_os_error());
    }
    let capacity = usize::try_from(bytes)
        .map_err(io::Error::other)?
        .checked_div(size_of::<libc::proc_fdinfo>())
        .and_then(|count| count.checked_mul(2))
        .ok_or_else(|| io::Error::other("descriptor list size is unavailable"))?;
    let mut descriptors = vec![
        libc::proc_fdinfo {
            proc_fd:     0,
            proc_fdtype: 0,
        };
        capacity
    ];
    let bytes = buffer_size(descriptors.as_slice())?;
    // SAFETY: The initialized descriptor array is writable for exactly `bytes` bytes.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDLISTFDS,
            0,
            descriptors.as_mut_ptr().cast(),
            bytes,
        )
    };
    if written <= 0 {
        return Err(io::Error::last_os_error());
    }
    let written = usize::try_from(written).map_err(io::Error::other)?;
    if written >= size_of_val(descriptors.as_slice())
        || !written.is_multiple_of(size_of::<libc::proc_fdinfo>())
    {
        return Err(io::Error::other("descriptor list is incomplete"));
    }
    descriptors.truncate(written / size_of::<libc::proc_fdinfo>());
    Ok(descriptors)
}

#[allow(
    unsafe_code,
    reason = "libproc reads vnode paths into an SDK-compatible record"
)]
fn descriptor_path(pid: libc::pid_t, descriptor: i32) -> io::Result<Vec<u8>> {
    let mut info = MaybeUninit::<VnodeFdInfoWithPath>::zeroed();
    let bytes = buffer_size(std::slice::from_ref(&info))?;
    // SAFETY: The aligned, initialized buffer has the SDK layout and exactly `bytes` writable
    // bytes.
    let written = unsafe {
        libc::proc_pidfdinfo(
            pid,
            descriptor,
            PROC_PIDFDVNODEPATHINFO,
            info.as_mut_ptr().cast(),
            bytes,
        )
    };
    if written <= 0 {
        return Err(io::Error::last_os_error());
    }
    if written != bytes {
        return Err(io::Error::other(
            "libproc returned an incomplete SDK record",
        ));
    }
    // SAFETY: The call returned the complete SDK record, whose fields contain only integer values.
    let info = unsafe { info.assume_init() };
    let path = info.vnode.vip_path.as_flattened();
    let end = path
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| io::Error::other("descriptor path is incomplete"))?;
    Ok(path[..end]
        .iter()
        .map(|byte| byte.to_ne_bytes()[0])
        .collect())
}

fn buffer_size<T>(buffer: &[T]) -> io::Result<i32> {
    i32::try_from(size_of_val(buffer)).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::process_owner;

    #[test]
    fn process_owner_reads_launchds_root_uid() -> std::io::Result<()> {
        assert_eq!(process_owner(1)?, 0);
        Ok(())
    }
}
