//! Read Darwin birth times directly; scanning never starts a subprocess.
//!
//! Apple's `sys/sysctl.h` puts `extern_proc` first in `kinfo_proc`, and
//! `sys/proc.h` puts its `p_starttime` timeval at offset zero. libc exposes
//! `timeval` but not Darwin's `kinfo_proc`, so read the bounded opaque reply
//! and decode only that prefix using libc's native field sizes and offsets.
//! Sources: <https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/sysctl.h>
//! and <https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc.h>.

use std::ffi::CStr;
use std::ffi::CString;
use std::io;
use std::io::Error;
use std::io::ErrorKind;
use std::mem::offset_of;
use std::mem::size_of;
use std::ptr;
use std::sync::OnceLock;
use std::time::Duration;

use rustix::process::Pid;
use rustix::process::test_kill_process;
use uuid::Uuid;

use super::BirthStamp;
use super::LifetimeEvidence;
use super::Observation;
use crate::constants::BIRTH_MACOS_BOOT_NAME;
use crate::constants::BIRTH_MICROSECONDS_PER_SECOND;
use crate::constants::BIRTH_SYSCTL_INCOMPLETE;
use crate::constants::BIRTH_SYSCTL_MAX_BYTES;

/// Boot identity cannot change while this scanner runs; a denied read stays unknown.
static BOOT: OnceLock<io::Result<String>> = OnceLock::new();

/// The verifier and its session diagnostic share exactly one cached sysctl result.
pub(super) fn boot() -> &'static io::Result<String> { super::cached_boot(&BOOT, read_boot) }

/// Darwin exposes process records by numeric MIB and the boot UUID only by name.
enum KernelQuery<'query> {
    /// Numeric selectors include the particular process whose birth is requested.
    Process(&'query mut [libc::c_int]),
    /// The boot-session UUID has no public numeric selector.
    BootSession(&'query CStr),
}

/// Observe the requested process each time, including immediately before cleanup.
pub(super) fn observe(pid: u32) -> Observation { observe_with_boot(pid, boot()) }

/// A denied or incomplete boot response authorizes neither admission nor deletion.
fn observe_with_boot(pid: u32, boot: &io::Result<String>) -> Observation {
    let Ok(boot) = boot else {
        return Observation::Unknown;
    };
    let Ok(pid) = libc::c_int::try_from(pid) else {
        return Observation::Unknown;
    };
    let mut name = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
    let mut bytes = [0; BIRTH_SYSCTL_MAX_BYTES];
    match read_sysctl(KernelQuery::Process(&mut name), &mut bytes) {
        Ok(0) => process_absence(pid),
        Ok(length) => decode_timeval(&bytes[..length]).map_or(Observation::Unknown, |birth| {
            Observation::Present(BirthStamp::macos(boot.clone(), birth))
        }),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => process_absence(pid),
        Err(_) => Observation::Unknown,
    }
}

/// A hidden process record is not proof of exit; only ESRCH from the pid check is.
fn process_absence(pid: libc::c_int) -> Observation {
    let Some(pid) = Pid::from_raw(pid) else {
        return Observation::Unknown;
    };
    match test_kill_process(pid) {
        Err(rustix::io::Errno::SRCH) => Observation::Ended,
        Ok(()) | Err(_) => Observation::Unknown,
    }
}

/// Process rows retain fractional birth precision even though registrations cannot.
pub(super) fn lifetime(pid: u32) -> super::LifetimeEvidence {
    let Ok(boot) = boot() else {
        return LifetimeEvidence::Unavailable;
    };
    let Ok(pid) = libc::c_int::try_from(pid) else {
        return LifetimeEvidence::Unavailable;
    };
    let mut name = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
    let mut bytes = [0; BIRTH_SYSCTL_MAX_BYTES];
    read_sysctl(KernelQuery::Process(&mut name), &mut bytes)
        .and_then(|length| decode_timeval(&bytes[..length]))
        .map_or(LifetimeEvidence::Unavailable, |birth| {
            super::ProcessLifetime::macos(boot.clone(), birth)
        })
}

/// The session UUID remains stable when the calendar clock changes during a run.
fn read_boot() -> io::Result<String> {
    let name = CString::new(BIRTH_MACOS_BOOT_NAME).map_err(|_| incomplete())?;
    let mut bytes = [0; BIRTH_SYSCTL_MAX_BYTES];
    let length = read_sysctl(KernelQuery::BootSession(&name), &mut bytes)?;
    decode_boot_session(&bytes[..length])
}

/// Only a complete UUID response can qualify a process birth with its boot session.
fn decode_boot_session(bytes: &[u8]) -> io::Result<String> {
    let boot = CStr::from_bytes_with_nul(bytes)
        .map_err(|_| incomplete())?
        .to_str()
        .map_err(|_| incomplete())?;
    Uuid::parse_str(boot).map_err(|_| incomplete())?;
    Ok(boot.to_owned())
}

/// Read-only sysctl has no safe wrapper in the crate's dependency tree.
#[allow(
    unsafe_code,
    reason = "Darwin process births require read-only sysctl FFI, which rustix does not expose"
)]
fn read_sysctl(query: KernelQuery<'_>, bytes: &mut [u8]) -> io::Result<usize> {
    let mut length = bytes.len();
    // SAFETY: query contains either a borrowed numeric selector or a terminated
    // C string; bytes is exclusively borrowed for its supplied length. length is
    // a live size_t. Null newp and zero newlen request no kernel writes. Neither
    // call retains pointers, and the returned length is checked before decoding.
    let result = unsafe {
        match query {
            KernelQuery::Process(name) => libc::sysctl(
                name.as_mut_ptr(),
                libc::c_uint::try_from(name.len()).map_err(|_| incomplete())?,
                bytes.as_mut_ptr().cast(),
                ptr::from_mut(&mut length),
                ptr::null_mut(),
                0,
            ),
            KernelQuery::BootSession(name) => libc::sysctlbyname(
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                ptr::from_mut(&mut length),
                ptr::null_mut(),
                0,
            ),
        }
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    if length > bytes.len() {
        return Err(incomplete());
    }
    Ok(length)
}

/// Decode integer bytes without assuming the output buffer has timeval alignment.
/// Native libc offsets also account for timeval's trailing ABI padding.
fn decode_timeval(bytes: &[u8]) -> io::Result<Duration> {
    if bytes.len() < size_of::<libc::timeval>() {
        return Err(incomplete());
    }
    let seconds_offset = offset_of!(libc::timeval, tv_sec);
    let microseconds_offset = offset_of!(libc::timeval, tv_usec);
    let seconds = libc::time_t::from_ne_bytes(
        bytes[seconds_offset..seconds_offset + size_of::<libc::time_t>()]
            .try_into()
            .map_err(|_| incomplete())?,
    );
    let microseconds = libc::suseconds_t::from_ne_bytes(
        bytes[microseconds_offset..microseconds_offset + size_of::<libc::suseconds_t>()]
            .try_into()
            .map_err(|_| incomplete())?,
    );
    let seconds = u64::try_from(seconds).map_err(|_| incomplete())?;
    let microseconds = u32::try_from(microseconds).map_err(|_| incomplete())?;
    if seconds == 0 || microseconds >= BIRTH_MICROSECONDS_PER_SECOND {
        return Err(incomplete());
    }
    Ok(Duration::from_secs(seconds) + Duration::from_micros(u64::from(microseconds)))
}

/// A partial or invalid reply preserves the record instead of claiming it ended.
fn incomplete() -> Error { io::Error::new(ErrorKind::InvalidData, BIRTH_SYSCTL_INCOMPLETE) }

#[cfg(test)]
mod tests {
    use std::io;
    use std::io::ErrorKind;
    use std::mem::size_of;

    use super::decode_boot_session;
    use super::decode_timeval;
    use super::observe;
    use super::observe_with_boot;
    use super::process_absence;
    use crate::birth_stamp::Observation;

    #[test]
    fn current_process_has_a_repeatable_kernel_stamp() {
        let first = observe(std::process::id());
        assert!(matches!(first, Observation::Present(_)));
        assert_eq!(first, observe(std::process::id()));
    }

    #[test]
    fn missing_birth_data_cannot_prove_a_live_process_ended() {
        assert!(decode_timeval(&[0; size_of::<libc::timeval>()]).is_err());
        assert_eq!(
            process_absence(rustix::process::getpid().as_raw_nonzero().get()),
            Observation::Unknown
        );
    }

    #[test]
    fn unallocated_pid_is_ended() {
        // Darwin's pid allocation range is smaller than the signed syscall limit.
        assert_eq!(observe(i32::MAX.unsigned_abs()), Observation::Ended);
    }

    #[test]
    fn denied_or_incomplete_boot_session_cannot_authorize_cleanup() {
        let pid = std::process::id();
        let denied = Err(io::Error::from(ErrorKind::PermissionDenied));
        assert_eq!(observe_with_boot(pid, &denied), Observation::Unknown);
        for bytes in [
            b"".as_slice(),
            b"\0",
            b"not-a-uuid\0",
            b"01234567-89AB-4CDE-8F01-23456789ABCD",
            b"01234567-89AB-4CDE-8F01-23456789ABCD\0trailing",
        ] {
            let incomplete = decode_boot_session(bytes);
            assert!(incomplete.is_err());
            assert_eq!(observe_with_boot(pid, &incomplete), Observation::Unknown);
        }
        assert!(
            decode_boot_session(b"01234567-89AB-4CDE-8F01-23456789ABCD\0")
                .is_ok_and(|boot| boot == "01234567-89AB-4CDE-8F01-23456789ABCD")
        );
    }
}
