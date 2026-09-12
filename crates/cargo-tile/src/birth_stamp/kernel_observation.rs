//! Acquire pid-bound kernel observations and compare registration identity evidence.

use std::io;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::OnceLock;

use super::BirthStamp;
#[cfg(target_os = "linux")]
use super::linux;
#[cfg(target_os = "macos")]
use super::macos;
use crate::constants::BIRTH_BOOT_EMPTY;
#[cfg(target_os = "linux")]
use crate::constants::BIRTH_BOOT_ID_PATH;
#[cfg(target_os = "macos")]
use crate::constants::BIRTH_MACOS_BOOT_NAME;
use crate::progress::PathFailure;

/// An identity comparison input; only `KernelObservation` proves its kernel source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Observation {
    /// The process still exists and supplied a complete birth stamp.
    Present(BirthStamp),
    /// The kernel reports that this pid no longer exists.
    Ended,
    /// Access was denied or the response was incomplete.
    Unknown,
}

/// Kernel evidence bound to the pid whose state was read, independently of snapshots.
/// Private fields prevent parsed birth stamps from becoming kernel observations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct KernelObservation {
    /// A matching stamp cannot verify a different pid from this observation's source.
    pid:         u32,
    /// Only the platform reader supplies this evidence in production.
    observation: Observation,
}

impl KernelObservation {
    /// Evidence about another pid authorizes neither admission nor removal.
    pub(crate) fn compare(&self, pid: u32, identity: &IdentityEvidence) -> Verification {
        if self.pid != pid {
            return Verification::Unknown;
        }
        identity.compare(&self.observation)
    }

    /// Sweep race tests control each observation without exposing production injection.
    #[cfg(test)]
    pub(crate) const fn for_test(pid: u32, observation: Observation) -> Self {
        Self { pid, observation }
    }
}

/// Whether a record supplied enough identity information for a comparison.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum IdentityEvidence {
    /// Both fields use the publisher's normalized representation.
    Available(BirthStamp),
    /// Empty or old-format identity fields authorize neither rows nor cleanup.
    Unavailable,
}

impl IdentityEvidence {
    /// Even a missing live pid cannot authorize deletion of an unverified record.
    fn compare(&self, observation: &Observation) -> Verification {
        match self {
            Self::Available(stamp) => stamp.compare(observation),
            Self::Unavailable => Verification::Unknown,
        }
    }
}

/// Only a completed identity comparison may authorize an active registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Verification {
    /// The registered boot and birth match the process currently holding the pid.
    Confirmed,
    /// The writer ended, even if another process now holds its pid.
    Ended,
    /// Uncertainty authorizes neither an active row nor an artifact deletion.
    Unknown,
}

/// Read kernel state afresh; process-table snapshot absence is never enough.
pub(crate) fn observe(pid: u32) -> KernelObservation {
    KernelObservation {
        pid,
        observation: observe_process(pid),
    }
}

/// Preserve a failed boot read for the session, including an empty kernel response.
pub(super) fn cached_boot(
    cache: &OnceLock<io::Result<String>>,
    read: impl FnOnce() -> io::Result<String>,
) -> &io::Result<String> {
    cache.get_or_init(|| {
        let boot = read()?;
        if boot.trim().is_empty() {
            Err(io::Error::new(ErrorKind::InvalidData, BIRTH_BOOT_EMPTY))
        } else {
            Ok(boot)
        }
    })
}

/// Settings names the cached failure separately from a process unreadable this scan.
pub(crate) fn boot_verification() -> Result<(), PathFailure> {
    #[cfg(target_os = "linux")]
    {
        boot_read_result(linux::boot(), Path::new(BIRTH_BOOT_ID_PATH))
    }
    #[cfg(target_os = "macos")]
    {
        boot_read_result(macos::boot(), Path::new(BIRTH_MACOS_BOOT_NAME))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Ok(())
    }
}

/// Retain the kernel interface and original I/O cause without retrying the read.
fn boot_read_result(boot: &io::Result<String>, path: &Path) -> Result<(), PathFailure> {
    boot.as_ref().map(|_| ()).map_err(|error| PathFailure {
        path:    path.to_owned(),
        failure: io::Error::new(error.kind(), error.to_string()).into(),
    })
}

/// Keep platform comparison inputs behind the pid-binding observation constructor.
fn observe_process(pid: u32) -> Observation {
    #[cfg(target_os = "linux")]
    {
        linux::observe(pid)
    }
    #[cfg(target_os = "macos")]
    {
        macos::observe(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        Observation::Unknown
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::io::ErrorKind;
    use std::time::Duration;

    use super::BirthStamp;
    use super::IdentityEvidence;
    use super::Observation;
    use super::Verification;
    use super::observe;
    use crate::constants::BIRTH_BOOT_EMPTY;
    use crate::constants::BIRTH_MICROSECONDS_PER_SECOND;
    use crate::registration::Registration;
    use crate::registration::RegistrationVerification;

    #[test]
    fn failed_boot_read_retains_named_error_and_is_never_retried() {
        let cache = std::sync::OnceLock::new();
        let reads = std::cell::Cell::new(0);
        for _ in 0..2 {
            let boot = super::cached_boot(&cache, || {
                reads.set(reads.get() + 1);
                Err(std::io::Error::from(ErrorKind::PermissionDenied))
            });
            let diagnostic = super::boot_read_result(boot, std::path::Path::new("kernel/boot"))
                .expect_err("cached boot failure");
            assert_eq!(diagnostic.path, std::path::Path::new("kernel/boot"));
            assert_eq!(
                diagnostic.failure.kind,
                std::io::ErrorKind::PermissionDenied
            );
        }
        assert_eq!(reads.get(), 1);
        let still_failed = super::cached_boot(&cache, || Ok("recovered".to_owned()));
        assert!(still_failed.is_err());
    }

    #[test]
    fn empty_boot_response_is_a_cached_failure_too() {
        let cache = std::sync::OnceLock::new();
        assert!(super::cached_boot(&cache, || Ok("  \n".to_owned())).is_err());
        let error = super::cached_boot(&cache, || Ok("recovered".to_owned()))
            .as_ref()
            .expect_err("empty first boot remains invalid");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), BIRTH_BOOT_EMPTY);
    }

    #[test]
    fn kernel_observation_verifies_only_the_pid_it_read() {
        let pid = std::process::id();
        let observation = observe(pid);
        let stamp = match &observation.observation {
            Observation::Present(stamp) => Ok(stamp),
            Observation::Ended | Observation::Unknown => {
                Err("current process birth is unavailable")
            },
        }
        .expect("kernel must report this test process birth");
        let bytes = [
            "cargo-tile-v2",
            "generation",
            &stamp.boot,
            &stamp.birth.to_string(),
            "exact.log",
            "/work",
            "/home/writer",
            "0",
            "",
        ]
        .join("\0");
        let record = match Registration::parse(bytes.as_bytes()).expect("valid live record") {
            Registration::Versioned(record) => Ok(record),
            Registration::Legacy(_) => Err("expected versioned record"),
        }
        .expect("fixture supplies a versioned record");
        assert!(matches!(
            record.verify_observation(pid, &observation),
            RegistrationVerification::Confirmed(verified) if verified.pid() == pid && verified.record() == &record
        ));
        assert_eq!(
            record.verify_observation(pid.wrapping_add(1), &observation),
            RegistrationVerification::Unknown
        );
    }

    #[test]
    fn mismatched_live_birth_is_ended() {
        let record = BirthStamp::from_fields("boot", "101");
        let live = Observation::Present(BirthStamp::linux("boot".to_owned(), 102));
        assert_eq!(record.compare(&live), Verification::Ended);
    }

    #[test]
    fn boot_change_ends_an_otherwise_identical_birth() {
        let record = BirthStamp::from_fields("previous", "101");
        let live = Observation::Present(BirthStamp::linux("current".to_owned(), 101));
        assert_eq!(record.compare(&live), Verification::Ended);
    }

    #[test]
    fn missing_or_nonnumeric_identity_remains_unknown_even_when_pid_ended() {
        for (boot, birth) in [
            ("", "101"),
            ("boot", ""),
            ("boot", "+101"),
            ("boot", "101 "),
            ("boot", "Wed Sep 9 20:00:00 2026"),
            ("boot", "18446744073709551616"),
        ] {
            let record = BirthStamp::from_fields(boot, birth);
            assert_eq!(record, IdentityEvidence::Unavailable);
            assert_eq!(record.compare(&Observation::Ended), Verification::Unknown);
        }
    }

    #[test]
    fn denied_observation_neither_confirms_nor_ends_a_registration() {
        assert_eq!(
            BirthStamp::from_fields("boot", "101").compare(&Observation::Unknown),
            Verification::Unknown
        );
    }

    #[test]
    fn macos_discards_fractional_seconds_but_linux_keeps_individual_ticks() {
        let record = BirthStamp::from_fields("boot", "101");
        let birth = Duration::from_secs(101)
            + Duration::from_micros(u64::from(BIRTH_MICROSECONDS_PER_SECOND - 1));
        assert_eq!(
            record.compare(&Observation::Present(BirthStamp::macos(
                "boot".to_owned(),
                birth
            ))),
            Verification::Confirmed
        );
        assert_eq!(
            record.compare(&Observation::Present(BirthStamp::linux(
                "boot".to_owned(),
                102
            ))),
            Verification::Ended
        );
    }

    #[test]
    fn legacy_macos_clock_changes_cannot_end_a_live_registration() {
        let boot_session = "01234567-89AB-4CDE-8F01-23456789ABCD";
        let live = match BirthStamp::from_fields("{ sec = 100, usec = 24 }", "101") {
            IdentityEvidence::Available(live) => Ok(live),
            IdentityEvidence::Unavailable => Err("valid legacy Darwin stamp"),
        }
        .expect("complete fixture identity");
        let old = BirthStamp::from_fields("{ sec = 100, usec = 23 }", "101");
        assert_eq!(
            old.compare(&Observation::Present(live)),
            Verification::Unknown
        );
        assert_eq!(old.compare(&Observation::Ended), Verification::Ended);
        let current = Observation::Present(BirthStamp::macos(
            boot_session.to_owned(),
            Duration::from_secs(101),
        ));
        assert_eq!(old.compare(&current), Verification::Unknown);
        assert_eq!(
            BirthStamp::from_fields(boot_session, "101").compare(&current),
            Verification::Confirmed
        );
        assert_eq!(
            BirthStamp::from_fields(boot_session, "100").compare(&current),
            Verification::Ended
        );
    }
}
