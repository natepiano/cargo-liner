//! Read Linux process births without losing the kernel's tick resolution.

use std::fs;
use std::io;
use std::io::Error;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::OnceLock;

use super::BirthStamp;
use super::Observation;
use crate::constants::BIRTH_BOOT_ID_PATH;
use crate::constants::BIRTH_PROC_DIRECTORY;
use crate::constants::BIRTH_STAT_COMM_END;
use crate::constants::BIRTH_STAT_FILENAME;
use crate::constants::BIRTH_STAT_START_INDEX;

/// Read the boot once, retaining a denied read as uncertainty for this process.
static BOOT: OnceLock<Result<String, Error>> = OnceLock::new();

/// The verifier and its session diagnostic share exactly one cached boot result.
pub(super) fn boot() -> &'static io::Result<String> {
    super::cached_boot(&BOOT, || fs::read_to_string(BIRTH_BOOT_ID_PATH))
}

/// Read the registered shim pid, not this scanner or an intermediate child.
pub(super) fn observe(pid: u32) -> Observation {
    let Ok(boot) = boot() else {
        return Observation::Unknown;
    };
    let boot = boot.trim();
    let path = Path::new(BIRTH_PROC_DIRECTORY)
        .join(pid.to_string())
        .join(BIRTH_STAT_FILENAME);
    match fs::read(path) {
        Ok(bytes) => start_ticks(&bytes).map_or(Observation::Unknown, |ticks| {
            Observation::Present(BirthStamp::linux(boot.to_owned(), ticks))
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => Observation::Ended,
        Err(_) => Observation::Unknown,
    }
}

/// `comm` may contain parentheses, whitespace, newlines, and non-UTF-8 bytes.
/// Only fields after its final closing parenthesis have whitespace boundaries.
fn start_ticks(stat: &[u8]) -> Result<u64, ()> {
    let end = stat
        .iter()
        .rposition(|byte| *byte == BIRTH_STAT_COMM_END)
        .ok_or(())?;
    let field = stat[end + 1..]
        .split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty())
        .nth(BIRTH_STAT_START_INDEX)
        .ok_or(())?;
    let field = std::str::from_utf8(field).map_err(|_| ())?;
    super::decimal(field)
}

#[cfg(test)]
mod tests {
    use super::observe;
    use super::start_ticks;
    use crate::birth_stamp::Observation;
    use crate::constants::BIRTH_STAT_START_INDEX;

    #[test]
    fn stat_comm_cannot_move_the_starttime_field() {
        let mut stat = b"123 (spaces ) and ( parentheses\n\xff) S".to_vec();
        for _ in 1..BIRTH_STAT_START_INDEX {
            stat.extend_from_slice(b" 0");
        }
        stat.extend_from_slice(b" 1234567890123 0");
        assert_eq!(start_ticks(&stat), Ok(1_234_567_890_123));
    }

    #[test]
    fn short_or_invalid_stat_is_incomplete() {
        for stat in [b"123 no comm".as_slice(), b"123 (comm) S", b""] {
            assert!(start_ticks(stat).is_err());
        }
    }

    #[test]
    fn current_process_has_a_repeatable_kernel_stamp() {
        let first = observe(std::process::id());
        assert!(matches!(first, Observation::Present(_)));
        assert_eq!(first, observe(std::process::id()));
    }

    #[test]
    fn unallocated_pid_is_ended() {
        assert_eq!(observe(u32::MAX), Observation::Ended);
    }
}
