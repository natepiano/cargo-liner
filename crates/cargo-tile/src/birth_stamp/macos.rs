//! Read Darwin birth times directly; scanning never starts a subprocess.
//!
//! Apple's `sys/sysctl.h` puts `extern_proc` first in `kinfo_proc`, and
//! `sys/proc.h` puts its `p_starttime` timeval at offset zero. libc exposes
//! `timeval` but not Darwin's `kinfo_proc`, so read the bounded opaque reply
//! and decode only that prefix using libc's native field sizes and offsets.
//! Sources: <https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/sysctl.h>
//! and <https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc.h>.

use std::io;
use std::mem::offset_of;
use std::mem::size_of;
use std::ptr;
use std::sync::OnceLock;
use std::time::Duration;

use super::BirthStamp;
use super::Observation;
use crate::constants::BIRTH_MACOS_BOOT_PREFIX;
use crate::constants::BIRTH_MACOS_BOOT_SEPARATOR;
use crate::constants::BIRTH_MACOS_BOOT_SUFFIX;
use crate::constants::BIRTH_MICROSECONDS_PER_SECOND;
use crate::constants::BIRTH_SYSCTL_INCOMPLETE;
use crate::constants::BIRTH_SYSCTL_MAX_BYTES;

/// Boot identity cannot change while this scanner runs; a denied read stays unknown.
static BOOT: OnceLock<io::Result<String>> = OnceLock::new();

/// The verifier and its session diagnostic share exactly one cached sysctl result.
pub(super) fn boot() -> &'static io::Result<String> { super::cached_boot(&BOOT, read_boot) }

/// Observe the requested process each time, including immediately before cleanup.
pub(super) fn observe(pid: u32) -> Observation {
    let Ok(boot) = boot() else {
        return Observation::Unknown;
    };
    let Ok(pid) = libc::c_int::try_from(pid) else {
        return Observation::Unknown;
    };
    let mut name = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
    let mut bytes = [0; BIRTH_SYSCTL_MAX_BYTES];
    match read_sysctl(&mut name, &mut bytes) {
        Ok(0) => Observation::Ended,
        Ok(length) => decode_timeval(&bytes[..length]).map_or(Observation::Unknown, |birth| {
            Observation::Present(BirthStamp::macos(boot.clone(), birth))
        }),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Observation::Ended,
        Err(_) => Observation::Unknown,
    }
}

/// Process rows retain fractional birth precision even though registrations cannot.
pub(super) fn lifetime(pid: u32) -> super::LifetimeEvidence {
    let Ok(boot) = boot() else {
        return super::LifetimeEvidence::Unavailable;
    };
    let Ok(pid) = libc::c_int::try_from(pid) else {
        return super::LifetimeEvidence::Unavailable;
    };
    let mut name = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
    let mut bytes = [0; BIRTH_SYSCTL_MAX_BYTES];
    read_sysctl(&mut name, &mut bytes)
        .and_then(|length| decode_timeval(&bytes[..length]))
        .map_or(super::LifetimeEvidence::Unavailable, |birth| {
            super::ProcessLifetime::macos(boot.clone(), birth)
        })
}

/// Match the numeric prefix of `sysctl -n kern.boottime`, without its local date.
fn read_boot() -> io::Result<String> {
    let mut name = [libc::CTL_KERN, libc::KERN_BOOTTIME];
    let mut bytes = [0; size_of::<libc::timeval>()];
    let length = read_sysctl(&mut name, &mut bytes)?;
    if length != bytes.len() {
        return Err(incomplete());
    }
    let boot = decode_timeval(&bytes)?;
    let seconds = boot.as_secs();
    let microseconds = boot.subsec_micros();
    Ok(format!(
        "{BIRTH_MACOS_BOOT_PREFIX}{seconds}{BIRTH_MACOS_BOOT_SEPARATOR}{microseconds}{BIRTH_MACOS_BOOT_SUFFIX}"
    ))
}

/// Read-only sysctl has no safe wrapper in the crate's dependency tree.
#[allow(
    unsafe_code,
    reason = "Darwin process births require read-only sysctl FFI, which rustix does not expose"
)]
fn read_sysctl(name: &mut [libc::c_int], bytes: &mut [u8]) -> io::Result<usize> {
    let count = libc::c_uint::try_from(name.len()).map_err(|_| incomplete())?;
    let mut length = bytes.len();
    // SAFETY: name and bytes are exclusively borrowed, initialized buffers valid
    // for their supplied lengths. length is a live size_t. Null newp and zero
    // newlen request no write to kernel state. sysctl retains none of the pointers;
    // its result and returned length are checked before any bytes are interpreted.
    let result = unsafe {
        libc::sysctl(
            name.as_mut_ptr(),
            count,
            bytes.as_mut_ptr().cast(),
            ptr::from_mut(&mut length),
            ptr::null_mut(),
            0,
        )
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
    if microseconds >= BIRTH_MICROSECONDS_PER_SECOND {
        return Err(incomplete());
    }
    Ok(Duration::from_secs(seconds) + Duration::from_micros(u64::from(microseconds)))
}

/// A partial or invalid reply preserves the record instead of claiming it ended.
fn incomplete() -> io::Error { io::Error::new(io::ErrorKind::InvalidData, BIRTH_SYSCTL_INCOMPLETE) }

#[cfg(test)]
mod tests {
    use super::observe;
    use crate::birth_stamp::Observation;

    #[test]
    fn current_process_has_a_repeatable_kernel_stamp() {
        let first = observe(std::process::id());
        assert!(matches!(first, Observation::Present(_)));
        assert_eq!(first, observe(std::process::id()));
    }
}
